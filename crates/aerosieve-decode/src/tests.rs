use std::io::Cursor;

use aerosieve_acoustic::rms_dbfs;
use aerosieve_fixtures::{rms, tone};
use tempfile::TempDir;

use super::*;

const TARGET_RATE: u32 = 16_000;

fn without_normalization() -> DecodeConfig {
    DecodeConfig {
        normalize: false,
        ..DecodeConfig::default()
    }
}

/// Encode `samples` as a 32-bit float WAV, interleaved across `channels`.
fn wav_bytes(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
    for &sample in samples {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    bytes.into_inner()
}

fn decode_wav(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    config: &DecodeConfig,
) -> DecodedAudio {
    let bytes = wav_bytes(samples, sample_rate, channels);
    decode_reader(Cursor::new(bytes), Some("wav"), config).unwrap()
}

fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).flat_map(|(&l, &r)| [l, r]).collect()
}

#[test]
fn audio_already_at_the_target_rate_passes_through_unresampled() {
    let source = tone(TARGET_RATE, 440.0, 0.1, 32_000);
    let audio = decode_wav(&source, TARGET_RATE, 1, &without_normalization());

    assert_eq!(audio.sample_rate, TARGET_RATE);
    assert_eq!(audio.source_sample_rate, TARGET_RATE);
    assert_eq!(audio.source_channels, 1);
    assert_eq!(audio.samples, source);
    assert!((audio.duration_s() - 2.0).abs() < 1e-4);
}

#[test]
fn resampling_is_sample_exact_and_keeps_both_ends_of_the_signal() {
    for source_rate in [8_000u32, 22_050, 44_100, 48_000] {
        let source_len = source_rate as usize * 3 + 137;
        let source = tone(source_rate, 440.0, 0.1, source_len);
        let audio = decode_wav(&source, source_rate, 1, &DecodeConfig::default());

        let expected_len =
            (source_len as f64 * f64::from(TARGET_RATE) / f64::from(source_rate)).round() as usize;
        assert_eq!(audio.samples.len(), expected_len, "{source_rate} Hz");
        assert_eq!(audio.source_sample_rate, source_rate);

        let (head, tail) = (&audio.samples[..64], &audio.samples[expected_len - 800..]);
        assert!(
            rms(head) > 0.01,
            "{source_rate} Hz starts with filter ramp-up"
        );
        assert!(rms(tail) > 0.01, "{source_rate} Hz lost its tail");
    }
}

#[test]
fn stereo_is_downmixed_by_averaging() {
    let left = tone(TARGET_RATE, 440.0, 0.2, 16_000);
    let inverted: Vec<f32> = left.iter().map(|s| -s).collect();

    let cancelled = decode_wav(
        &interleave(&left, &inverted),
        TARGET_RATE,
        2,
        &without_normalization(),
    );
    assert_eq!(cancelled.source_channels, 2);
    assert_eq!(cancelled.samples.len(), left.len());
    assert!(rms(&cancelled.samples) < 1e-5);

    let doubled = decode_wav(
        &interleave(&left, &left),
        TARGET_RATE,
        2,
        &without_normalization(),
    );
    assert!((rms(&doubled.samples) - rms(&left)).abs() < 1e-4);
}

#[test]
fn gain_is_clamped_in_both_directions() {
    let limit = DecodeConfig::default().max_gain_db;

    let quiet = tone(TARGET_RATE, 440.0, 0.007, 16_000);
    let boosted = decode_wav(&quiet, TARGET_RATE, 1, &DecodeConfig::default());
    assert!((boosted.applied_gain_db - limit).abs() < 1e-3);

    let loud = tone(TARGET_RATE, 440.0, 0.99, 16_000);
    let attenuated = decode_wav(&loud, TARGET_RATE, 1, &DecodeConfig::default());
    assert!((attenuated.applied_gain_db + limit).abs() < 1e-3);
}

#[test]
fn level_within_the_clamp_reaches_the_target() {
    let config = DecodeConfig::default();
    let one_db_under = 10f32.powf((config.target_dbfs - 1.0) / 20.0) * std::f32::consts::SQRT_2;
    let source = tone(TARGET_RATE, 440.0, one_db_under, 16_000);

    let audio = decode_wav(&source, TARGET_RATE, 1, &config);

    assert!((audio.applied_gain_db - 1.0).abs() < 0.05);
    assert!((rms_dbfs(&audio.samples) - config.target_dbfs).abs() < 0.1);
}

#[test]
fn normalization_can_be_disabled() {
    let source = tone(TARGET_RATE, 440.0, 0.01, 16_000);
    let audio = decode_wav(&source, TARGET_RATE, 1, &without_normalization());

    assert_eq!(audio.applied_gain_db, 0.0);
    assert_eq!(audio.samples, source);
}

#[test]
fn silence_is_not_amplified() {
    let audio = decode_wav(&[0.0; 16_000], TARGET_RATE, 1, &DecodeConfig::default());

    assert_eq!(audio.applied_gain_db, 0.0);
    assert!(audio.samples.iter().all(|&sample| sample == 0.0));
}

#[test]
fn zero_length_audio_decodes_to_nothing() {
    for (sample_rate, channels) in [(TARGET_RATE, 1), (44_100, 2)] {
        let audio = decode_wav(&[], sample_rate, channels, &DecodeConfig::default());

        assert!(audio.samples.is_empty());
        assert_eq!(audio.source_sample_rate, sample_rate);
        assert_eq!(audio.duration_s(), 0.0);
    }
}

#[test]
fn sixteen_bit_integer_pcm_is_scaled_to_unit_range() {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
    for sample in tone(TARGET_RATE, 440.0, 0.5, 16_000) {
        writer
            .write_sample((sample * f32::from(i16::MAX)) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();

    let audio = decode_reader(
        Cursor::new(bytes.into_inner()),
        None,
        &without_normalization(),
    )
    .unwrap();

    assert_eq!(audio.samples.len(), 16_000);
    assert!((rms(&audio.samples) - 0.5 / std::f32::consts::SQRT_2).abs() < 0.01);
}

#[test]
fn decode_file_matches_decode_reader() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("clip.wav");
    let bytes = wav_bytes(&tone(22_050, 440.0, 0.1, 22_050), 22_050, 1);
    std::fs::write(&path, &bytes).unwrap();

    let from_file = decode_file(&path, &DecodeConfig::default()).unwrap();
    let from_reader = decode_reader(Cursor::new(bytes), None, &DecodeConfig::default()).unwrap();

    assert_eq!(from_file.samples, from_reader.samples);
    assert_eq!(from_file.source_sample_rate, 22_050);
}

#[test]
fn missing_file_is_an_open_error_naming_the_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("absent.wav");

    let error = decode_file(&path, &DecodeConfig::default()).unwrap_err();

    assert!(matches!(&error, DecodeError::Open { path: reported, .. } if *reported == path));
}

#[test]
fn non_audio_bytes_are_an_error() {
    for bytes in [
        &b""[..],
        &b"definitely not audio, just ascii filler 0123456789"[..],
    ] {
        let result = decode_reader(
            Cursor::new(bytes.to_vec()),
            Some("wav"),
            &DecodeConfig::default(),
        );
        assert!(matches!(
            result,
            Err(DecodeError::UnsupportedFormat(_) | DecodeError::Read(_))
        ));
    }
}

#[test]
fn zero_target_sample_rate_is_rejected() {
    let config = DecodeConfig {
        target_sample_rate: 0,
        ..DecodeConfig::default()
    };
    let bytes = wav_bytes(&tone(TARGET_RATE, 440.0, 0.1, 1_600), TARGET_RATE, 1);

    let result = decode_reader(Cursor::new(bytes), None, &config);

    assert!(matches!(result, Err(DecodeError::InvalidConfig(_))));
}
