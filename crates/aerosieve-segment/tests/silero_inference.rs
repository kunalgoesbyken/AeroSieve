//! Inference against the real Silero VAD model.
//!
//! Needs the `silero` feature and `models/silero_vad.onnx` (see
//! `models/README.md`). Tests that need the model pass vacuously without it.
//!
//! Silero scores every synthetic signal as non-speech, so these tests cover
//! the model binding (input sensitivity, state threading, reset) rather than
//! its decision boundary.

#![cfg(feature = "silero")]

use std::path::PathBuf;

use aerosieve_fixtures::{SAMPLE_RATE, glottal_speech, samples, white_noise};
use aerosieve_segment::{SegmentConfig, SegmentError, Segmenter, SileroDetector, SpeechDetector};

fn model_path() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/silero_vad.onnx");
    if !path.exists() {
        eprintln!("skipping: {} not present", path.display());
        return None;
    }
    Some(path)
}

fn detector() -> Option<SileroDetector> {
    let detector = SileroDetector::from_model_path(&model_path()?, SAMPLE_RATE).unwrap();
    Some(detector)
}

#[test]
fn only_the_two_silero_sample_rates_are_accepted() {
    let Some(path) = model_path() else { return };

    for (sample_rate, window) in [(8_000, 256), (16_000, 512)] {
        let detector = SileroDetector::from_model_path(&path, sample_rate).unwrap();
        assert_eq!(detector.window_size(), window);
    }
    for sample_rate in [22_050, 44_100] {
        assert!(matches!(
            SileroDetector::from_model_path(&path, sample_rate),
            Err(SegmentError::UnsupportedSampleRate(rate)) if rate == sample_rate
        ));
    }
}

#[test]
fn a_missing_model_is_reported_with_its_path() {
    let missing = PathBuf::from("definitely/not/a/model.onnx");
    assert!(matches!(
        SileroDetector::from_model_path(&missing, SAMPLE_RATE),
        Err(SegmentError::ModelNotFound(path)) if path == missing
    ));
}

#[test]
fn probabilities_are_valid_and_depend_on_the_input() {
    let Some(mut detector) = detector() else {
        return;
    };
    let window = detector.window_size();

    let inputs = [
        vec![0.0f32; window],
        vec![0.9f32; window],
        white_noise(window, 0.5, 0x9999),
    ];
    let probabilities: Vec<f32> = inputs
        .iter()
        .map(|input| {
            detector.reset();
            detector.probability(input).unwrap()
        })
        .collect();

    assert!(probabilities.iter().all(|p| (0.0..=1.0).contains(p)));
    assert!(probabilities.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn recurrent_state_is_carried_between_calls() {
    let Some(mut detector) = detector() else {
        return;
    };
    let window = vec![0.3f32; detector.window_size()];

    let sequence: Vec<f32> = (0..6)
        .map(|_| detector.probability(&window).unwrap())
        .collect();

    assert!(sequence.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn reset_restores_the_initial_state() {
    let Some(mut detector) = detector() else {
        return;
    };
    let audio = glottal_speech(samples(0.5));
    let mut windows = audio.chunks_exact(detector.window_size());
    let first_window = windows.next().unwrap();

    let before = detector.probability(first_window).unwrap();
    for window in windows {
        detector.probability(window).unwrap();
    }
    detector.reset();

    assert_eq!(detector.probability(first_window).unwrap(), before);
}

#[test]
fn segmenter_runs_the_model_over_a_whole_recording() {
    let Some(detector) = detector() else { return };
    let audio = [
        vec![0.0; samples(1.0)],
        glottal_speech(samples(5.0)),
        vec![0.0; samples(1.0)],
    ]
    .concat();

    let segments = Segmenter::new(detector, SegmentConfig::default())
        .segment(&audio)
        .unwrap();

    assert!(segments.iter().all(|s| s.end_sample <= audio.len()));
    assert!(
        segments
            .windows(2)
            .all(|pair| pair[0].end_sample <= pair[1].start_sample)
    );
}
