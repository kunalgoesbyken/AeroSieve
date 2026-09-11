//! Tests for the standardization stage.
//!
//! Fixtures are written at runtime with `hound` into a temp directory and
//! removed by a `Drop` guard, so no binary audio is checked into the repo.

use std::f32::consts::TAU;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp directory that deletes itself when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aerosieve-decode-{tag}-{nanos:x}-{n}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write a 32-bit float WAV. Samples are interleaved across `channels`.
fn write_wav(path: &Path, samples: &[f32], rate: u32, channels: u16) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in samples {
        w.write_sample(s).expect("write sample");
    }
    w.finalize().expect("finalize wav");
}

/// A mono sine of `freq` Hz at `amp` amplitude.
fn sine(freq: f32, amp: f32, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (TAU * freq * i as f32 / rate as f32).sin())
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

// --- Rate and channel handling ---------------------------------------------

#[test]
fn passthrough_16k_mono_preserves_sample_count_and_rate() {
    let dir = TempDir::new("pass16k");
    let path = dir.join("mono16k.wav");
    let frames = 16_000 * 2; // 2 s
    write_wav(&path, &sine(440.0, 0.1, 16_000, frames), 16_000, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode");

    assert_eq!(audio.sample_rate, 16_000);
    assert_eq!(audio.source_sample_rate, 16_000);
    assert_eq!(audio.source_channels, 1);
    // No resampling: sample count is bit-exactly preserved.
    assert_eq!(audio.samples.len(), frames);
    assert!((audio.duration_s - 2.0).abs() < 1e-4, "{}", audio.duration_s);
}

#[test]
fn resample_44100_to_16000_preserves_duration_within_one_percent() {
    let dir = TempDir::new("resample44k");
    let path = dir.join("mono44k.wav");
    let src_rate = 44_100;
    let secs = 3.0f32;
    let frames = (src_rate as f32 * secs) as usize;
    write_wav(&path, &sine(440.0, 0.1, src_rate, frames), src_rate, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode");

    assert_eq!(audio.sample_rate, 16_000);
    assert_eq!(audio.source_sample_rate, src_rate);

    // Duration within 1% is the stated requirement, but the implementation is
    // sample-exact, so assert that: a resampler that drops the trailing
    // partial chunk lands short, and one that forgets to compensate the sinc
    // filter delay lands long (measured ~0.52% long at this rate). Both are
    // caught here.
    let err = (audio.duration_s - secs).abs() / secs;
    assert!(
        err < 0.01,
        "duration drifted {:.4}% (got {:.6}s, want {:.6}s, {} samples)",
        err * 100.0,
        audio.duration_s,
        secs,
        audio.samples.len()
    );
    let ideal = (frames as f64 * 16_000.0 / src_rate as f64).round() as usize;
    assert_eq!(
        audio.samples.len(),
        ideal,
        "expected sample-exact resampling"
    );

    // The tail must contain real signal, not the silence a truncating
    // implementation would leave behind.
    let tail = &audio.samples[audio.samples.len() - 1_600..];
    assert!(rms(tail) > 0.01, "tail is silent: rms {}", rms(tail));

    // Phase alignment: the output must start where the input started. An
    // uncompensated filter delay prepends ~SINC_LEN/2 frames of ramp-up, which
    // shows up as a non-trivial head-of-signal offset.
    let head = &audio.samples[..64];
    assert!(
        rms(head) > 0.05,
        "output starts with filter ramp-up instead of signal: rms {}",
        rms(head)
    );
}

#[test]
fn resample_8000_to_16000_upsamples_and_preserves_duration() {
    let dir = TempDir::new("resample8k");
    let path = dir.join("mono8k.wav");
    let frames = 8_000; // 1 s
    write_wav(&path, &sine(300.0, 0.1, 8_000, frames), 8_000, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode");

    assert_eq!(audio.sample_rate, 16_000);
    // Sample-exact 2x upsample. A naive flush overshoots here by ~5% and
    // leaves a fully silent tail.
    assert_eq!(audio.samples.len(), 16_000);
    let tail = &audio.samples[audio.samples.len() - 800..];
    assert!(rms(tail) > 0.01, "tail is silent: rms {}", rms(tail));
    assert!(rms(&audio.samples[..64]) > 0.01, "head is silent");
}

#[test]
fn stereo_is_downmixed_to_mono_by_averaging() {
    let dir = TempDir::new("stereo");
    let path = dir.join("stereo16k.wav");
    let frames = 16_000;

    // Left = +a, right = -a. Averaging must cancel to (near) silence, which
    // proves we average rather than take one channel or concatenate.
    let mut interleaved = Vec::with_capacity(frames * 2);
    let left = sine(440.0, 0.2, 16_000, frames);
    for &s in &left {
        interleaved.push(s);
        interleaved.push(-s);
    }
    write_wav(&path, &interleaved, 16_000, 2);

    let cfg = DecodeConfig {
        normalize: false,
        ..Default::default()
    };
    let audio = decode_file(&path, &cfg).expect("decode");

    assert_eq!(audio.source_channels, 2);
    assert_eq!(audio.sample_rate, 16_000);
    // One mono frame per interleaved stereo frame.
    assert_eq!(audio.samples.len(), frames);
    assert!(
        rms(&audio.samples) < 1e-5,
        "anti-phase stereo did not cancel: rms {}",
        rms(&audio.samples)
    );
}

#[test]
fn stereo_downmix_of_identical_channels_preserves_level() {
    let dir = TempDir::new("stereo_same");
    let path = dir.join("stereo_same.wav");
    let frames = 8_000;
    let mono = sine(440.0, 0.25, 16_000, frames);
    let mut interleaved = Vec::with_capacity(frames * 2);
    for &s in &mono {
        interleaved.push(s);
        interleaved.push(s);
    }
    write_wav(&path, &interleaved, 16_000, 2);

    let cfg = DecodeConfig {
        normalize: false,
        ..Default::default()
    };
    let audio = decode_file(&path, &cfg).expect("decode");

    assert_eq!(audio.samples.len(), frames);
    assert!((rms(&audio.samples) - rms(&mono)).abs() < 1e-4);
}

// --- Normalization ----------------------------------------------------------

#[test]
fn quiet_signal_gets_positive_gain_clamped_to_max() {
    let dir = TempDir::new("quiet");
    let path = dir.join("quiet.wav");
    // ~-46 dBFS: far below the -20 dBFS target, so the clamp must bind.
    write_wav(&path, &sine(440.0, 0.007, 16_000, 16_000), 16_000, 1);

    let cfg = DecodeConfig::default();
    let audio = decode_file(&path, &cfg).expect("decode");

    assert!(
        audio.applied_gain_db > 0.0,
        "expected boost, got {}",
        audio.applied_gain_db
    );
    assert!(
        audio.applied_gain_db.abs() <= 3.0 + 1e-4,
        "gain {} exceeded +/-3 dB clamp",
        audio.applied_gain_db
    );
    assert!((audio.applied_gain_db - 3.0).abs() < 1e-3);
}

#[test]
fn loud_signal_gets_negative_gain_clamped_to_max() {
    let dir = TempDir::new("loud");
    let path = dir.join("loud.wav");
    // ~-3 dBFS RMS: far above the -20 dBFS target.
    write_wav(&path, &sine(440.0, 0.99, 16_000, 16_000), 16_000, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode");

    assert!(
        audio.applied_gain_db < 0.0,
        "expected attenuation, got {}",
        audio.applied_gain_db
    );
    assert!(
        audio.applied_gain_db.abs() <= 3.0 + 1e-4,
        "gain {} exceeded +/-3 dB clamp",
        audio.applied_gain_db
    );
    assert!((audio.applied_gain_db + 3.0).abs() < 1e-3);
}

#[test]
fn signal_within_clamp_range_reaches_target_exactly() {
    let dir = TempDir::new("near_target");
    let path = dir.join("near.wav");
    // Sine of amplitude a has RMS a/sqrt(2). Aim for about -21 dBFS, 1 dB
    // under target, so the needed gain sits inside the +/-3 dB clamp.
    let target_rms = 10f32.powf(-21.0 / 20.0);
    let amp = target_rms * std::f32::consts::SQRT_2;
    write_wav(&path, &sine(440.0, amp, 16_000, 16_000), 16_000, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode");

    assert!(audio.applied_gain_db.abs() < 3.0);
    let out_dbfs = rms_dbfs(&audio.samples).expect("non-silent");
    assert!(
        (out_dbfs - (-20.0)).abs() < 0.1,
        "normalized to {out_dbfs} dBFS, want -20"
    );
}

#[test]
fn normalize_false_leaves_samples_and_gain_untouched() {
    let dir = TempDir::new("nonorm");
    let path = dir.join("nonorm.wav");
    let src = sine(440.0, 0.01, 16_000, 16_000);
    write_wav(&path, &src, 16_000, 1);

    let cfg = DecodeConfig {
        normalize: false,
        ..Default::default()
    };
    let audio = decode_file(&path, &cfg).expect("decode");

    assert_eq!(audio.applied_gain_db, 0.0);
    assert_eq!(audio.samples.len(), src.len());
    for (got, want) in audio.samples.iter().zip(&src) {
        assert!((got - want).abs() < 1e-6, "sample changed: {got} vs {want}");
    }
}

#[test]
fn digital_silence_normalizes_to_zero_gain_without_dividing_by_zero() {
    let dir = TempDir::new("silence");
    let path = dir.join("silence.wav");
    write_wav(&path, &vec![0.0f32; 16_000], 16_000, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode");

    assert_eq!(audio.applied_gain_db, 0.0);
    assert_eq!(audio.samples.len(), 16_000);
    assert!(audio.samples.iter().all(|&s| s == 0.0));
    assert!(rms_dbfs(&audio.samples).is_none());
}

// --- Degenerate and failure cases ------------------------------------------

#[test]
fn zero_length_audio_is_handled_without_panic() {
    let dir = TempDir::new("empty");
    let path = dir.join("empty.wav");
    write_wav(&path, &[], 16_000, 1);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode empty wav");

    assert!(audio.samples.is_empty());
    assert_eq!(audio.sample_rate, 16_000);
    assert_eq!(audio.applied_gain_db, 0.0);
    assert_eq!(audio.duration_s, 0.0);
}

#[test]
fn zero_length_audio_at_odd_rate_is_handled_without_panic() {
    let dir = TempDir::new("empty44k");
    let path = dir.join("empty44k.wav");
    write_wav(&path, &[], 44_100, 2);

    let audio = decode_file(&path, &DecodeConfig::default()).expect("decode empty wav");

    assert!(audio.samples.is_empty());
    assert_eq!(audio.source_sample_rate, 44_100);
    assert_eq!(audio.duration_s, 0.0);
}

#[test]
fn nonexistent_file_returns_io_error_not_panic() {
    let dir = TempDir::new("missing");
    let path = dir.join("does-not-exist.wav");

    let err = decode_file(&path, &DecodeConfig::default()).expect_err("should fail");

    assert!(
        matches!(err, DecodeError::Io { .. }),
        "expected Io, got {err:?}"
    );
}

#[test]
fn garbage_file_returns_error_not_panic() {
    let dir = TempDir::new("garbage");
    let path = dir.join("garbage.wav");
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(b"this is definitely not audio, just ascii filler bytes 0123456789")
        .expect("write");
    f.sync_all().expect("sync");
    drop(f);

    let err = decode_file(&path, &DecodeConfig::default()).expect_err("should fail");

    assert!(
        matches!(
            err,
            DecodeError::UnsupportedFormat(_) | DecodeError::Decode(_) | DecodeError::Io { .. }
        ),
        "unexpected variant {err:?}"
    );
}

#[test]
fn empty_file_returns_error_not_panic() {
    let dir = TempDir::new("zerobytes");
    let path = dir.join("zero.wav");
    std::fs::write(&path, b"").expect("write");

    let err = decode_file(&path, &DecodeConfig::default()).expect_err("should fail");
    // Any typed error is acceptable; a panic is not.
    let _ = err.to_string();
}

#[test]
fn zero_target_sample_rate_is_rejected() {
    let dir = TempDir::new("badcfg");
    let path = dir.join("ok.wav");
    write_wav(&path, &sine(440.0, 0.1, 16_000, 1_600), 16_000, 1);

    let cfg = DecodeConfig {
        target_sample_rate: 0,
        ..Default::default()
    };
    let err = decode_file(&path, &cfg).expect_err("should fail");
    assert!(matches!(err, DecodeError::InvalidConfig(_)), "{err:?}");
}

// --- Reader API -------------------------------------------------------------

#[test]
fn decode_reader_matches_decode_file() {
    let dir = TempDir::new("reader");
    let path = dir.join("reader.wav");
    write_wav(&path, &sine(440.0, 0.1, 22_050, 22_050), 22_050, 1);

    let from_file = decode_file(&path, &DecodeConfig::default()).expect("decode file");

    let bytes = std::fs::read(&path).expect("read bytes");
    let from_reader = decode_reader(
        std::io::Cursor::new(bytes),
        Some("wav"),
        &DecodeConfig::default(),
    )
    .expect("decode reader");

    assert_eq!(from_reader.sample_rate, from_file.sample_rate);
    assert_eq!(from_reader.samples.len(), from_file.samples.len());
    assert_eq!(from_reader.source_sample_rate, 22_050);
}

#[test]
fn decode_reader_without_hint_still_probes() {
    let dir = TempDir::new("nohint");
    let path = dir.join("nohint.wav");
    write_wav(&path, &sine(440.0, 0.1, 16_000, 3_200), 16_000, 1);

    let bytes = std::fs::read(&path).expect("read bytes");
    let audio = decode_reader(std::io::Cursor::new(bytes), None, &DecodeConfig::default())
        .expect("decode without hint");

    assert_eq!(audio.samples.len(), 3_200);
}

// --- Integer PCM ------------------------------------------------------------

#[test]
fn decodes_16_bit_integer_pcm() {
    let dir = TempDir::new("pcm16");
    let path = dir.join("pcm16.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create");
    for s in sine(440.0, 0.5, 16_000, 16_000) {
        w.write_sample((s * i16::MAX as f32) as i16).expect("write");
    }
    w.finalize().expect("finalize");

    let cfg = DecodeConfig {
        normalize: false,
        ..Default::default()
    };
    let audio = decode_file(&path, &cfg).expect("decode");

    assert_eq!(audio.samples.len(), 16_000);
    // RMS of a 0.5-amplitude sine is 0.5/sqrt(2) ~ 0.354.
    assert!((rms(&audio.samples) - 0.3536).abs() < 0.01);
}
