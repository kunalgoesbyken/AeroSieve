use aerosieve_fixtures::{
    SAMPLE_RATE, at_dbfs, harmonic_speech, mix_at_snr, overdrive, samples, tone, white_noise,
};

use crate::spectrum::fft_in_place;
use crate::wada::snr_from_g;
use crate::wada_table::{G_VALS, SNR_MAX_DB, SNR_MIN_DB};
use crate::*;

const ONE_SECOND: usize = SAMPLE_RATE as usize;

fn sine(len: usize, freq_hz: f32, amplitude: f32) -> Vec<f32> {
    tone(SAMPLE_RATE, freq_hz, amplitude, len)
}

fn verdict(buffer: &[f32]) -> Option<RejectReason> {
    FilterPolicy::default().evaluate(&score(buffer, SAMPLE_RATE))
}

#[test]
fn default_policy_accepts_clean_speech_across_seeds_and_levels() {
    for seed in [1u64, 7, 99, 12345, 0xDEAD_BEEF] {
        for level in [-26.0f32, -23.0, -20.0, -16.0, -12.0] {
            let speech = at_dbfs(harmonic_speech(samples(2.0), seed), level);
            assert_eq!(verdict(&speech), None, "seed {seed} at {level} dBFS");
        }
    }
}

#[test]
fn silence_is_rejected_as_silence() {
    let metrics = score(&vec![0.0; ONE_SECOND], SAMPLE_RATE);
    assert_eq!(metrics.rms_dbfs, f32::NEG_INFINITY);
    assert_eq!(
        FilterPolicy::default().evaluate(&metrics),
        Some(RejectReason::Silence)
    );
}

#[test]
fn faint_speech_is_rejected_as_too_quiet() {
    let quiet = at_dbfs(harmonic_speech(ONE_SECOND, 5), -60.0);
    assert_eq!(verdict(&quiet), Some(RejectReason::TooQuiet));
}

#[test]
fn overdriven_speech_is_rejected() {
    let clipped = overdrive(&at_dbfs(harmonic_speech(ONE_SECOND, 11), -20.0), 25.0);
    assert!(clip_ratio(&clipped) > FilterPolicy::default().max_clip_ratio);
    assert!(matches!(
        verdict(&clipped),
        Some(RejectReason::Clipping | RejectReason::TooLoud)
    ));
}

#[test]
fn clip_ratio_counts_only_samples_at_full_scale() {
    let mut buffer = vec![0.5f32; 100];
    buffer[..10].fill(1.0);
    assert!((clip_ratio(&buffer) - 0.10).abs() < 1e-6);
    assert_eq!(clip_ratio(&[0.0; 10]), 0.0);
}

#[test]
fn pure_tone_is_rejected_as_tonal() {
    let tone = at_dbfs(sine(ONE_SECOND, 440.0, 0.5), -20.0);
    assert_eq!(verdict(&tone), Some(RejectReason::Tonal));
}

#[test]
fn flatness_floor_is_tunable() {
    let tone = at_dbfs(sine(ONE_SECOND, 440.0, 0.5), -20.0);
    let lenient = FilterPolicy {
        min_spectral_flatness: 0.0,
        ..FilterPolicy::default()
    };
    assert_ne!(
        lenient.evaluate(&score(&tone, SAMPLE_RATE)),
        Some(RejectReason::Tonal)
    );
}

#[test]
fn flatness_ignores_content_outside_the_speech_band() {
    let speech = harmonic_speech(ONE_SECOND, 3);
    let hiss_above_the_band = tone(SAMPLE_RATE, 6_000.0, 0.2, ONE_SECOND);
    let mixed: Vec<f32> = speech
        .iter()
        .zip(&hiss_above_the_band)
        .map(|(speech, hiss)| speech + hiss)
        .collect();

    let (alone, with_hiss) = (
        spectral_flatness(&speech, SAMPLE_RATE),
        spectral_flatness(&mixed, SAMPLE_RATE),
    );
    assert!((alone - with_hiss).abs() < 0.02, "{alone} vs {with_hiss}");
}

#[test]
fn flatness_is_zero_when_the_band_is_above_nyquist() {
    assert_eq!(
        spectral_flatness(&white_noise(ONE_SECOND, 0.1, 9), 500),
        0.0
    );
}

#[test]
fn flatness_orders_tone_below_speech_below_noise() {
    let tone = spectral_flatness(&sine(ONE_SECOND, 440.0, 0.5), SAMPLE_RATE);
    let speech = spectral_flatness(&harmonic_speech(ONE_SECOND, 3), SAMPLE_RATE);
    let noise = spectral_flatness(&white_noise(ONE_SECOND, 0.1, 9), SAMPLE_RATE);
    assert!(tone < speech && speech < noise, "{tone} {speech} {noise}");
    assert!(noise > 0.5, "{noise}");
}

#[test]
fn wada_snr_tracks_known_snr_within_3_db() {
    let clean = harmonic_speech(samples(4.0), 0x5EED);
    for target in [0.0f32, 10.0, 20.0, 30.0] {
        let estimate = wada_snr_db(&mix_at_snr(&clean, target, 0xBEEF + target as u64));
        assert!(
            (estimate - target).abs() <= 3.0,
            "true {target} dB estimated as {estimate:.2} dB"
        );
    }
}

#[test]
fn wada_snr_is_monotonic_in_true_snr() {
    let clean = harmonic_speech(samples(2.0), 0x1234);
    let estimates: Vec<f32> = [-5.0f32, 0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0]
        .iter()
        .map(|&target| wada_snr_db(&mix_at_snr(&clean, target, 77)))
        .collect();
    assert!(estimates.is_sorted_by(|a, b| a < b), "{estimates:?}");
}

#[test]
fn wada_snr_is_independent_of_level() {
    let reference = mix_at_snr(&harmonic_speech(ONE_SECOND, 2), 15.0, 3);
    let attenuated: Vec<f32> = reference.iter().map(|s| s * 0.01).collect();
    assert!((wada_snr_db(&reference) - wada_snr_db(&attenuated)).abs() < 0.5);
}

#[test]
fn wada_snr_is_clamped_to_the_table_range() {
    let buried = mix_at_snr(&harmonic_speech(ONE_SECOND, 8), -60.0, 4);
    assert!((SNR_MIN_DB..=SNR_MAX_DB).contains(&wada_snr_db(&buried)));
}

#[test]
fn snr_from_g_interpolates_and_clamps() {
    assert_eq!(snr_from_g(0.0), SNR_MIN_DB);
    assert_eq!(snr_from_g(10.0), SNR_MAX_DB);
    assert!((snr_from_g(G_VALS[40]) - 20.0).abs() < 0.01);
    let halfway = (G_VALS[40] + G_VALS[41]) / 2.0;
    assert!((snr_from_g(halfway) - 20.5).abs() < 0.01);
}

#[test]
fn empty_buffer_scores_as_silence() {
    let metrics = score(&[], SAMPLE_RATE);
    assert_eq!(metrics.rms_dbfs, f32::NEG_INFINITY);
    assert_eq!(metrics.wada_snr_db, f32::NEG_INFINITY);
    assert_eq!(metrics.clip_ratio, 0.0);
    assert_eq!(metrics.spectral_flatness, 0.0);
    assert_eq!(
        FilterPolicy::default().evaluate(&metrics),
        Some(RejectReason::Silence)
    );
}

#[test]
fn single_sample_is_never_accepted() {
    for value in [0.0f32, 0.5, 1.0, -1.0] {
        let metrics = score(&[value], SAMPLE_RATE);
        assert_eq!(metrics.wada_snr_db, f32::NEG_INFINITY);
        assert!(!FilterPolicy::default().accepts(&metrics));
    }
}

#[test]
fn dc_offset_reads_as_no_speech() {
    let metrics = score(&vec![0.25f32; ONE_SECOND], SAMPLE_RATE);
    assert_eq!(metrics.wada_snr_db, SNR_MIN_DB);
    assert!(!FilterPolicy::default().accepts(&metrics));
}

#[test]
fn non_finite_samples_are_never_accepted() {
    for poison in [f32::NAN, f32::INFINITY] {
        let mut buffer = at_dbfs(harmonic_speech(ONE_SECOND, 1), -20.0);
        buffer[100] = poison;
        assert!(!FilterPolicy::default().accepts(&score(&buffer, SAMPLE_RATE)));
    }
}

#[test]
fn buffers_around_every_length_threshold_do_not_panic() {
    for len in [0usize, 1, 2, 3, 255, 256, 257, 1023, 1024, 1025] {
        let metrics = score(&harmonic_speech(len, 42), SAMPLE_RATE);
        assert!(metrics.clip_ratio.is_finite());
        assert!(metrics.spectral_flatness.is_finite());
    }
}

#[test]
fn score_is_deterministic() {
    let buffer = harmonic_speech(ONE_SECOND, 555);
    assert_eq!(score(&buffer, SAMPLE_RATE), score(&buffer, SAMPLE_RATE));
}

#[test]
fn rms_dbfs_of_a_constant_signal_is_its_amplitude_in_db() {
    for amplitude in [1.0f32, 0.5, 0.1, 0.01] {
        let expected = 20.0 * amplitude.log10();
        assert!((rms_dbfs(&vec![amplitude; 1000]) - expected).abs() < 1e-4);
    }
}

#[test]
fn rms_dbfs_of_a_full_scale_sine_is_minus_3_db() {
    assert!((rms_dbfs(&sine(ONE_SECOND, 100.0, 1.0)) + 3.0103).abs() < 0.01);
}

#[test]
fn reject_reason_serializes_as_its_identifier() {
    for reason in [
        RejectReason::Silence,
        RejectReason::TooQuiet,
        RejectReason::TooLoud,
        RejectReason::Clipping,
        RejectReason::Tonal,
        RejectReason::LowSnr,
    ] {
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, format!("\"{reason}\""));
        assert_eq!(serde_json::from_str::<RejectReason>(&json).unwrap(), reason);
    }
}

#[test]
fn partial_policy_falls_back_to_defaults() {
    let policy: FilterPolicy = serde_json::from_str(r#"{"min_snr_db": 15.0}"#).unwrap();
    assert_eq!(
        policy,
        FilterPolicy {
            min_snr_db: 15.0,
            ..FilterPolicy::default()
        }
    );
}

#[test]
fn fft_puts_a_bin_aligned_sine_in_its_bin() {
    const BIN: usize = 64;
    let mut re = tone(FLATNESS_FFT_SIZE as u32, BIN as f32, 1.0, FLATNESS_FFT_SIZE);
    let mut im = vec![0.0f32; FLATNESS_FFT_SIZE];
    fft_in_place(&mut re, &mut im);

    let peak = (0..FLATNESS_FFT_SIZE / 2)
        .max_by(|&a, &b| re[a].hypot(im[a]).total_cmp(&re[b].hypot(im[b])))
        .unwrap();
    assert_eq!(peak, BIN);
}
