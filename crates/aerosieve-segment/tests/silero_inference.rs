//! Live inference tests against the real Silero VAD model.
//!
//! These require the `silero` feature AND `models/silero_vad.onnx` on disk, so
//! they skip themselves when the model is absent rather than failing a checkout
//! that has not fetched it. Fetch it with:
//!
//! ```text
//! curl -sL -o models/silero_vad.onnx \
//!   https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
//! ```
//!
//! Run on Windows with the MSVC toolchain; `ort` ships no prebuilt ONNX
//! Runtime for windows-gnu.

#![cfg(feature = "silero")]

use std::path::PathBuf;

use aerosieve_segment::{SegmentConfig, Segmenter, SileroDetector, SpeechDetector};

const SAMPLE_RATE: u32 = 16_000;

fn model_path() -> Option<PathBuf> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../models/silero_vad.onnx");
    p.exists().then_some(p)
}

/// Skip with a visible note rather than failing when the model is absent.
macro_rules! model_or_skip {
    () => {
        match model_path() {
            Some(p) => p,
            None => {
                eprintln!("skipping: models/silero_vad.onnx not present");
                return;
            }
        }
    };
}

fn detector() -> Option<SileroDetector> {
    let path = model_path()?;
    Some(SileroDetector::from_model_path(&path, SAMPLE_RATE).expect("load model"))
}

/// Speech-like audio: glottal pulse train plus aspiration noise.
fn speech(len: usize) -> Vec<f32> {
    let period = (SAMPLE_RATE as f32 / 120.0) as usize;
    let mut state = 0x5DEECE66Du64;
    let mut noise = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        ((state.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64)
            as f32
            * 2.0
            - 1.0
    };
    (0..len)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = 0.35 + 0.65 * (std::f32::consts::TAU * 3.5 * t).sin().abs();
            let ph = (i % period) as f32 / period as f32;
            let pulse = (-6.0 * ph).exp() * (std::f32::consts::TAU * ph).sin();
            (pulse * 0.8 + noise() * 0.04) * env * 0.8
        })
        .collect()
}

/// Deterministic white noise.
fn noise(len: usize) -> Vec<f32> {
    let mut state = 0x9999u64;
    (0..len)
        .map(|_| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            ((state.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64)
                as f32
                * 2.0
                - 1.0
        })
        .collect()
}

#[test]
fn model_loads_and_reports_the_silero_window() {
    let path = model_or_skip!();
    let det = SileroDetector::from_model_path(&path, SAMPLE_RATE).expect("load");
    // Silero requires exactly 512 samples at 16 kHz.
    assert_eq!(det.window_size(), 512);
}

#[test]
fn rejects_unsupported_sample_rates() {
    let path = model_or_skip!();
    assert!(SileroDetector::from_model_path(&path, 44_100).is_err());
    assert!(SileroDetector::from_model_path(&path, 22_050).is_err());
    // Both documented rates are accepted.
    assert!(SileroDetector::from_model_path(&path, 16_000).is_ok());
    assert!(SileroDetector::from_model_path(&path, 8_000).is_ok());
}

#[test]
fn missing_model_is_an_error_not_a_panic() {
    let missing = PathBuf::from("definitely/not/a/model.onnx");
    assert!(SileroDetector::from_model_path(&missing, SAMPLE_RATE).is_err());
}

#[test]
fn inference_produces_valid_probabilities() {
    let Some(mut det) = detector() else {
        eprintln!("skipping: model not present");
        return;
    };
    for chunk in speech(SAMPLE_RATE as usize).chunks_exact(det.window_size()) {
        let p = det.probability(chunk);
        assert!(
            (0.0..=1.0).contains(&p) && p.is_finite(),
            "probability out of range: {p}"
        );
    }
}

/// Silero is a learned model trained on real speech, and it correctly declines
/// to classify synthetic signals as speech. All of DC, white noise, a 200 Hz
/// sine, and a synthetic vowel score ~0.001 here.
///
/// So this asserts the property that IS verifiable without a real recording:
/// the model is genuinely sensitive to its input rather than emitting a
/// constant. A broken tensor binding would return the same value regardless.
#[test]
fn probabilities_depend_on_the_input() {
    let Some(mut det) = detector() else {
        eprintln!("skipping: model not present");
        return;
    };
    let w = det.window_size();

    let zeros = det.probability(&vec![0.0f32; w]);
    det.reset();
    let dc = det.probability(&vec![0.9f32; w]);
    det.reset();
    let noisy = det.probability(&noise(w));

    eprintln!("silero p: zeros={zeros:.6} dc={dc:.6} noise={noisy:.6}");
    assert!(
        zeros != dc || dc != noisy,
        "identical probabilities for different inputs suggests a broken binding"
    );
    for p in [zeros, dc, noisy] {
        assert!((0.0..=1.0).contains(&p) && p.is_finite());
    }
}

/// The recurrent state must actually be threaded between calls. Feeding the
/// same window repeatedly should produce a changing sequence as the RNN
/// settles; a constant would mean `stateN` is not being fed back.
#[test]
fn recurrent_state_is_threaded_between_calls() {
    let Some(mut det) = detector() else {
        eprintln!("skipping: model not present");
        return;
    };
    let window = vec![0.3f32; det.window_size()];
    let seq: Vec<f32> = (0..6).map(|_| det.probability(&window)).collect();

    eprintln!("silero repeated-window sequence: {seq:?}");
    assert!(
        seq.windows(2).any(|p| p[0] != p[1]),
        "probability never changed across calls: state is not being carried"
    );
}

/// Recurrent state must actually be carried and cleared: the same input after
/// a reset has to reproduce the same first-window probability.
#[test]
fn reset_restores_initial_state() {
    let Some(mut det) = detector() else {
        eprintln!("skipping: model not present");
        return;
    };
    let audio = speech(SAMPLE_RATE as usize / 2);
    let w = det.window_size();

    let first = det.probability(&audio[..w]);
    // Advance the recurrent state.
    for chunk in audio.chunks_exact(w).skip(1) {
        det.probability(chunk);
    }
    det.reset();
    let after_reset = det.probability(&audio[..w]);

    assert_eq!(
        first, after_reset,
        "reset must restore the initial recurrent state"
    );
}

/// End to end: the segmenter must drive the real model without panicking and
/// must respect its invariants.
///
/// This deliberately does NOT assert that a segment is found. Silero scores
/// synthetic audio at ~0.001, so the honest expectation is zero segments;
/// asserting otherwise would be a test that only passes with a fixture the
/// model was never meant to accept. Segmentation behaviour on speech-like
/// input is covered against the EnergyDetector in the unit tests.
#[test]
fn segmenter_drives_the_real_model_safely() {
    let Some(det) = detector() else {
        eprintln!("skipping: model not present");
        return;
    };

    let mut audio = vec![0.0f32; SAMPLE_RATE as usize];
    audio.extend(speech(SAMPLE_RATE as usize * 5));
    audio.extend(std::iter::repeat_n(0.0, SAMPLE_RATE as usize));

    let cfg = SegmentConfig {
        min_segment_s: 1.0,
        ..SegmentConfig::default()
    };
    let mut seg = Segmenter::new(det, cfg);
    let segments = seg.segment(&audio);

    eprintln!("silero segments on synthetic audio: {}", segments.len());
    // Whatever comes back must be well formed.
    for pair in segments.windows(2) {
        assert!(pair[0].end_sample <= pair[1].start_sample, "segments overlap");
    }
    for s in &segments {
        assert!(s.start_sample < s.end_sample);
        assert!(s.end_sample <= audio.len());
    }
}
