//! Stage 1 of the AeroSieve pipeline: **standardization**.
//!
//! Takes any container symphonia can demux (wav, mp3, flac, ogg/vorbis,
//! mp4/aac, mkv, caf, alac, adpcm) and produces canonical
//! **16 kHz mono `f32`** samples, loudness-normalized to **-20 dBFS** with the
//! applied gain clamped to **+/-3 dB**. This mirrors stage 1 of Emilia-Pipe
//! (arXiv 2407.05361), the reference architecture for the v2 pipeline.
//!
//! ```no_run
//! use std::path::Path;
//! use aerosieve_decode::{decode_file, DecodeConfig};
//!
//! let audio = decode_file(Path::new("clip.mp3"), &DecodeConfig::default())?;
//! assert_eq!(audio.sample_rate, 16_000);
//! println!("{:.2}s, gain {:+.2} dB", audio.duration_s, audio.applied_gain_db);
//! # Ok::<(), aerosieve_decode::DecodeError>(())
//! ```

mod error;
mod normalize;
mod resample;

pub use error::DecodeError;
pub use normalize::rms_dbfs;

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Canonical 16 kHz mono audio plus provenance about the source it came from.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// Mono `f32` samples at [`Self::sample_rate`], loudness-normalized.
    pub samples: Vec<f32>,
    /// Output rate. Always equal to `DecodeConfig::target_sample_rate`.
    pub sample_rate: u32,
    /// Sample rate declared by the source track, before resampling.
    pub source_sample_rate: u32,
    /// Channel count declared by the source track, before downmixing.
    pub source_channels: u16,
    /// Gain actually applied by normalization, in dB. `0.0` when normalization
    /// is disabled or the signal is digital silence.
    pub applied_gain_db: f32,
    /// Duration of [`Self::samples`] in seconds.
    pub duration_s: f32,
}

/// Knobs for the standardization stage.
#[derive(Debug, Clone)]
pub struct DecodeConfig {
    /// Rate every output is converted to. Default `16_000`.
    pub target_sample_rate: u32,
    /// RMS loudness target in dBFS. Default `-20.0` (Emilia-Pipe stage 1).
    pub target_dbfs: f32,
    /// Maximum absolute gain normalization may apply, in dB. Default `3.0`.
    pub max_gain_db: f32,
    /// Whether to normalize at all. Default `true`.
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

/// Decode, downmix, resample, and normalize an audio file from disk.
///
/// The file extension is used as a format hint; symphonia still probes the
/// actual bytes, so a mislabeled extension does not cause a failure.
pub fn decode_file(path: &Path, cfg: &DecodeConfig) -> Result<DecodedAudio, DecodeError> {
    let file = File::open(path).map_err(|source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let ext = path.extension().and_then(|e| e.to_str());
    decode_source(Box::new(file), ext, cfg)
}

/// Decode from any seekable reader, e.g. an in-memory buffer.
///
/// `hint_ext` is an optional container extension (`"wav"`, `"mp3"`, ...) used
/// to speed up probing; it is only a hint.
pub fn decode_reader<R>(
    reader: R,
    hint_ext: Option<&str>,
    cfg: &DecodeConfig,
) -> Result<DecodedAudio, DecodeError>
where
    R: std::io::Read + std::io::Seek + Send + Sync + 'static,
{
    decode_source(Box::new(ReadSeekSource(reader)), hint_ext, cfg)
}

/// Adapter turning an arbitrary `Read + Seek` into a symphonia [`MediaSource`].
///
/// Length is reported as unknown, which symphonia handles by probing rather
/// than trusting a declared size.
struct ReadSeekSource<R>(R);

impl<R: std::io::Read> std::io::Read for ReadSeekSource<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<R: std::io::Seek> std::io::Seek for ReadSeekSource<R> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<R> MediaSource for ReadSeekSource<R>
where
    R: std::io::Read + std::io::Seek + Send + Sync,
{
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

fn decode_source(
    source: Box<dyn MediaSource>,
    hint_ext: Option<&str>,
    cfg: &DecodeConfig,
) -> Result<DecodedAudio, DecodeError> {
    if cfg.target_sample_rate == 0 {
        return Err(DecodeError::InvalidConfig(
            "target_sample_rate must be non-zero".into(),
        ));
    }

    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = hint_ext {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| match e {
            SymphoniaError::Unsupported(msg) => DecodeError::UnsupportedFormat(msg.to_string()),
            SymphoniaError::IoError(source) => DecodeError::Io {
                path: Default::default(),
                source,
            },
            other => DecodeError::UnsupportedFormat(other.to_string()),
        })?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;

    let source_sample_rate = track
        .codec_params
        .sample_rate
        .ok_or(DecodeError::MissingSampleRate)?;
    if source_sample_rate == 0 {
        return Err(DecodeError::MissingSampleRate);
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| DecodeError::UnsupportedFormat(e.to_string()))?;

    // Pre-size for the declared duration when the container knows it, so the
    // mono buffer usually grows exactly once.
    let mut mono: Vec<f32> = match track.codec_params.n_frames {
        Some(n) => Vec::with_capacity(n as usize),
        None => Vec::new(),
    };

    let mut source_channels: u16 = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(0);

    // Reused across packets; reallocated only when a packet needs more room
    // than the buffer we already hold.
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Clean end of stream: symphonia signals it as UnexpectedEof.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                // Track list changed mid-stream; the documented pattern is to
                // stop and re-probe. We only consume a single track, so stop.
                break;
            }
            Err(SymphoniaError::IoError(source)) => {
                return Err(DecodeError::Io {
                    path: Default::default(),
                    source,
                });
            }
            Err(e) => return Err(DecodeError::Decode(e.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                let frames = audio_buf.frames();
                if frames == 0 {
                    continue;
                }
                let channels = spec.channels.count().max(1);
                if source_channels == 0 {
                    source_channels = channels as u16;
                }

                let needed = frames * channels;
                let buf = match sample_buf {
                    Some(ref mut b) if b.capacity() >= needed => b,
                    _ => {
                        sample_buf =
                            Some(SampleBuffer::<f32>::new(audio_buf.capacity() as u64, spec));
                        sample_buf.as_mut().expect("just assigned")
                    }
                };
                buf.copy_interleaved_ref(audio_buf);

                // Downmix by averaging channels.
                let inv = 1.0 / channels as f32;
                mono.extend(
                    buf.samples()
                        .chunks_exact(channels)
                        .map(|frame| frame.iter().sum::<f32>() * inv),
                );
            }
            // Recoverable per symphonia's documented pattern: skip the packet.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(DecodeError::Decode(e.to_string())),
        }
    }

    if source_channels == 0 {
        source_channels = 1;
    }

    let mut samples = resample::resample_mono(mono, source_sample_rate, cfg.target_sample_rate)?;

    let applied_gain_db = if cfg.normalize {
        normalize::normalize_in_place(&mut samples, cfg.target_dbfs, cfg.max_gain_db)
    } else {
        0.0
    };

    let duration_s = samples.len() as f32 / cfg.target_sample_rate as f32;

    Ok(DecodedAudio {
        samples,
        sample_rate: cfg.target_sample_rate,
        source_sample_rate,
        source_channels,
        applied_gain_db,
        duration_s,
    })
}

#[cfg(test)]
mod tests;
