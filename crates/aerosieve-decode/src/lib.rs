//! Standardization: any container symphonia can demux, in; mono `f32` at one
//! sample rate and one loudness, out.
//!
//! Defaults follow stage 1 of Emilia-Pipe (arXiv 2407.05361): 16 kHz, RMS
//! normalized toward -20 dBFS with the applied gain clamped to +/-3 dB.
//!
//! ```no_run
//! use std::path::Path;
//! use aerosieve_decode::{decode_file, DecodeConfig};
//!
//! let audio = decode_file(Path::new("clip.mp3"), &DecodeConfig::default())?;
//! println!("{:.2}s, gain {:+.2} dB", audio.duration_s(), audio.applied_gain_db);
//! # Ok::<(), aerosieve_decode::DecodeError>(())
//! ```

mod error;
mod normalize;
mod resample;

#[cfg(test)]
mod tests;

pub use error::DecodeError;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, CodecParameters, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Mono audio at a known rate, with provenance about its source.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Sample rate of the source track, before resampling.
    pub source_sample_rate: u32,
    /// Channel count of the source track, before downmixing.
    pub source_channels: u16,
    /// Gain applied by loudness normalization, in dB.
    pub applied_gain_db: f32,
}

impl DecodedAudio {
    /// Wrap samples that are already mono at `sample_rate`.
    pub fn from_samples(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
            source_sample_rate: sample_rate,
            source_channels: 1,
            applied_gain_db: 0.0,
        }
    }

    pub fn duration_s(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

#[derive(Debug, Clone)]
pub struct DecodeConfig {
    pub target_sample_rate: u32,
    /// RMS loudness target, in dBFS.
    pub target_dbfs: f32,
    /// Largest gain normalization may apply in either direction, in dB.
    pub max_gain_db: f32,
    pub normalize: bool,
}

impl Default for DecodeConfig {
    fn default() -> Self {
        Self {
            target_sample_rate: 16_000,
            target_dbfs: -20.0,
            max_gain_db: 3.0,
            normalize: true,
        }
    }
}

/// Decode, downmix, resample, and normalize a file.
///
/// The extension is only a probing hint; a mislabeled file still decodes.
pub fn decode_file(path: &Path, config: &DecodeConfig) -> Result<DecodedAudio, DecodeError> {
    let file = File::open(path).map_err(|source| DecodeError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let extension = path.extension().and_then(|ext| ext.to_str());
    decode_source(Box::new(file), extension, config)
}

/// Decode from any seekable reader. `extension_hint` (`"wav"`, `"mp3"`, ...)
/// speeds up probing and may be omitted.
pub fn decode_reader<R>(
    reader: R,
    extension_hint: Option<&str>,
    config: &DecodeConfig,
) -> Result<DecodedAudio, DecodeError>
where
    R: Read + Seek + Send + Sync + 'static,
{
    decode_source(Box::new(ReadSeekSource(reader)), extension_hint, config)
}

struct ReadSeekSource<R>(R);

impl<R: Read> Read for ReadSeekSource<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<R: Seek> Seek for ReadSeekSource<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for ReadSeekSource<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

struct Track {
    id: u32,
    codec: CodecParameters,
    sample_rate: u32,
}

struct MonoTrack {
    samples: Vec<f32>,
    source_channels: u16,
}

fn decode_source(
    source: Box<dyn MediaSource>,
    extension_hint: Option<&str>,
    config: &DecodeConfig,
) -> Result<DecodedAudio, DecodeError> {
    if config.target_sample_rate == 0 {
        return Err(DecodeError::InvalidConfig(
            "target_sample_rate must be non-zero",
        ));
    }

    let mut format = probe(source, extension_hint)?;
    let track = select_track(format.as_ref())?;
    let mono = decode_to_mono(format.as_mut(), &track)?;

    let mut samples =
        resample::resample_mono(mono.samples, track.sample_rate, config.target_sample_rate)?;
    let applied_gain_db = if config.normalize {
        normalize::normalize_in_place(&mut samples, config.target_dbfs, config.max_gain_db)
    } else {
        0.0
    };

    Ok(DecodedAudio {
        samples,
        sample_rate: config.target_sample_rate,
        source_sample_rate: track.sample_rate,
        source_channels: mono.source_channels,
        applied_gain_db,
    })
}

fn probe(
    source: Box<dyn MediaSource>,
    extension_hint: Option<&str>,
) -> Result<Box<dyn FormatReader>, DecodeError> {
    let mut hint = Hint::new();
    if let Some(extension) = extension_hint {
        hint.with_extension(extension);
    }
    let format_options = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };

    symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(source, Default::default()),
            &format_options,
            &MetadataOptions::default(),
        )
        .map(|probed| probed.format)
        .map_err(|error| match error {
            SymphoniaError::IoError(source) => DecodeError::Read(source),
            other => DecodeError::UnsupportedFormat(other.to_string()),
        })
}

fn select_track(format: &dyn FormatReader) -> Result<Track, DecodeError> {
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoAudioTrack)?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .filter(|&rate| rate > 0)
        .ok_or(DecodeError::MissingSampleRate)?;

    Ok(Track {
        id: track.id,
        codec: track.codec_params.clone(),
        sample_rate,
    })
}

/// Decode every packet of `track`, averaging channels into one.
///
/// Malformed packets are skipped, which is symphonia's documented recovery
/// path; only unrecoverable errors abort the file.
fn decode_to_mono(format: &mut dyn FormatReader, track: &Track) -> Result<MonoTrack, DecodeError> {
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec, &DecoderOptions::default())
        .map_err(|error| DecodeError::UnsupportedFormat(error.to_string()))?;

    let mut samples = Vec::with_capacity(track.codec.n_frames.unwrap_or(0) as usize);
    let mut source_channels = track.codec.channels.map_or(0, |channels| channels.count());
    let mut interleaved: Option<SampleBuffer<f32>> = None;

    while let Some(packet) = next_packet(format, track.id)? {
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) if is_end_of_stream(&error) => break,
            Err(error) => return Err(DecodeError::Decode(error.to_string())),
        };
        if decoded.frames() == 0 {
            continue;
        }

        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        if source_channels == 0 {
            source_channels = channels;
        }

        let buffer = match &mut interleaved {
            Some(buffer) if buffer.capacity() >= decoded.frames() * channels => buffer,
            slot => slot.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        buffer.copy_interleaved_ref(decoded);
        samples.extend(
            buffer
                .samples()
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
    }

    Ok(MonoTrack {
        samples,
        source_channels: source_channels.max(1) as u16,
    })
}

/// The next packet belonging to `track_id`, or `None` at end of stream.
fn next_packet(
    format: &mut dyn FormatReader,
    track_id: u32,
) -> Result<Option<Packet>, DecodeError> {
    loop {
        match format.next_packet() {
            Ok(packet) if packet.track_id() == track_id => return Ok(Some(packet)),
            Ok(_) => continue,
            Err(SymphoniaError::ResetRequired) => return Ok(None),
            Err(error) if is_end_of_stream(&error) => return Ok(None),
            Err(SymphoniaError::IoError(source)) => return Err(DecodeError::Read(source)),
            Err(error) => return Err(DecodeError::Decode(error.to_string())),
        }
    }
}

/// symphonia reports a clean end of stream as an `UnexpectedEof` I/O error.
fn is_end_of_stream(error: &SymphoniaError) -> bool {
    matches!(error, SymphoniaError::IoError(io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
}
