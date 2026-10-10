use aerosieve_fixtures::{Rng, SAMPLE_RATE, samples, silence, tone};
use proptest::prelude::*;

use crate::{EnergyDetector, Segment, SegmentConfig, SegmentError, Segmenter, SpeechDetector};

const WINDOW: usize = 512;

fn speech(len: usize) -> Vec<f32> {
    tone(SAMPLE_RATE, 140.0, 0.3, len)
}

fn ms(duration: u32) -> usize {
    samples(duration as f32 / 1000.0)
}

fn concat(parts: &[Vec<f32>]) -> Vec<f32> {
    parts.concat()
}

fn segment_with(config: SegmentConfig, audio: &[f32]) -> Vec<Segment> {
    let segments = Segmenter::new(EnergyDetector::new(SAMPLE_RATE), config)
        .segment(audio)
        .unwrap();
    assert_well_formed(&segments, audio.len());
    segments
}

fn segment(audio: &[f32]) -> Vec<Segment> {
    segment_with(SegmentConfig::default(), audio)
}

fn assert_well_formed(segments: &[Segment], total_samples: usize) {
    let mut previous_end = 0;
    for segment in segments {
        assert!(
            segment.start_sample >= previous_end,
            "overlap at {segment:?}"
        );
        assert!(
            segment.end_sample > segment.start_sample,
            "empty {segment:?}"
        );
        assert!(
            segment.end_sample <= total_samples,
            "out of bounds {segment:?}"
        );
        previous_end = segment.end_sample;
    }
}

/// Replays a seeded probability sequence, ignoring the audio, so the state
/// machine is exercised independently of any real detector's behaviour.
struct ScriptedDetector {
    seed: u64,
    rng: Rng,
}

impl ScriptedDetector {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            rng: Rng::new(seed),
        }
    }
}

impl SpeechDetector for ScriptedDetector {
    fn window_size(&self) -> usize {
        WINDOW
    }

    fn probability(&mut self, _window: &[f32]) -> Result<f32, SegmentError> {
        Ok(self.rng.uniform())
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
    }
}

/// Length bounds small enough that random detector output produces segments.
fn short_clip_config() -> SegmentConfig {
    SegmentConfig {
        min_segment_s: 0.05,
        max_segment_s: 0.5,
        ..SegmentConfig::default()
    }
}

struct FailingDetector;

impl SpeechDetector for FailingDetector {
    fn window_size(&self) -> usize {
        WINDOW
    }

    fn probability(&mut self, _window: &[f32]) -> Result<f32, SegmentError> {
        Err(SegmentError::Onnx("inference failed".into()))
    }

    fn reset(&mut self) {}
}

#[test]
fn silence_yields_no_segments() {
    assert!(segment(&silence(samples(10.0))).is_empty());
}

#[test]
fn a_single_burst_yields_one_segment_at_its_bounds() {
    let (lead, burst) = (samples(1.0), samples(5.0));
    let audio = concat(&[silence(lead), speech(burst), silence(samples(1.0))]);

    let segments = segment(&audio);

    let tolerance = 2 * WINDOW + ms(SegmentConfig::default().speech_pad_ms);
    assert_eq!(segments.len(), 1);
    assert!(segments[0].start_sample.abs_diff(lead) <= tolerance);
    assert!(segments[0].end_sample.abs_diff(lead + burst) <= tolerance);
}

#[test]
fn a_long_gap_separates_two_segments() {
    let audio = concat(&[
        silence(ms(500)),
        speech(samples(4.0)),
        silence(samples(1.0)),
        speech(samples(4.0)),
        silence(ms(500)),
    ]);

    let segments = segment(&audio);

    assert_eq!(segments.len(), 2);
    for segment in segments {
        assert!((3.5..=4.5).contains(&segment.duration_s(SAMPLE_RATE)));
    }
}

#[test]
fn a_gap_shorter_than_min_silence_is_bridged() {
    let audio = concat(&[
        silence(ms(500)),
        speech(samples(4.0)),
        silence(ms(50)),
        speech(samples(4.0)),
        silence(ms(500)),
    ]);

    let segments = segment(&audio);

    assert_eq!(segments.len(), 1);
    assert!((7.5..=8.6).contains(&segments[0].duration_s(SAMPLE_RATE)));
}

#[test]
fn bursts_below_the_length_floors_are_dropped() {
    let shorter_than_min_speech = ms(150);
    let shorter_than_min_segment = samples(1.0);

    for burst in [shorter_than_min_speech, shorter_than_min_segment] {
        let audio = concat(&[silence(samples(1.0)), speech(burst), silence(samples(1.0))]);
        assert!(segment(&audio).is_empty(), "{burst} samples");
    }
}

#[test]
fn continuous_speech_is_split_to_the_length_bounds() {
    let config = SegmentConfig::default();
    let segments = segment(&speech(samples(60.0)));

    assert!(segments.len() >= 2);
    for segment in segments {
        let duration = segment.duration_s(SAMPLE_RATE);
        assert!((config.min_segment_s..=config.max_segment_s).contains(&duration));
    }
}

#[test]
fn a_long_segment_is_cut_at_its_bridged_silence() {
    let (half, dip) = (samples(20.0), ms(50));
    let audio = concat(&[speech(half), silence(dip), speech(half)]);

    let segments = segment(&audio);

    assert_eq!(segments.len(), 2);
    assert!(segments[0].end_sample.abs_diff(half + dip / 2) <= 4 * WINDOW);
    assert_eq!(segments[0].end_sample, segments[1].start_sample);
}

#[test]
fn splitting_can_be_disabled() {
    let config = SegmentConfig {
        max_segment_s: 0.0,
        ..SegmentConfig::default()
    };
    assert_eq!(segment_with(config, &speech(samples(60.0))).len(), 1);
}

#[test]
fn padding_extends_both_ends_and_clamps_to_the_buffer() {
    let audio = concat(&[
        silence(samples(1.0)),
        speech(samples(5.0)),
        silence(samples(1.0)),
    ]);
    let unpadded = SegmentConfig {
        speech_pad_ms: 0,
        ..SegmentConfig::default()
    };

    let padded = segment(&audio)[0];
    let bare = segment_with(unpadded, &audio)[0];
    let pad = ms(SegmentConfig::default().speech_pad_ms);

    assert_eq!(bare.start_sample - padded.start_sample, pad);
    assert_eq!(padded.end_sample - bare.end_sample, pad);

    let edge_to_edge = segment(&speech(samples(5.0)));
    assert_eq!(edge_to_edge[0].start_sample, 0);
}

#[test]
fn padding_wider_than_a_gap_does_not_make_segments_overlap() {
    let config = SegmentConfig {
        speech_pad_ms: 400,
        ..SegmentConfig::default()
    };
    let audio = concat(&[speech(samples(4.0)), silence(ms(300)), speech(samples(4.0))]);

    assert_eq!(segment_with(config, &audio).len(), 2);
}

#[test]
fn input_shorter_than_one_window_yields_no_segments() {
    for len in [0, 1, 100, WINDOW - 1] {
        assert!(segment(&speech(len)).is_empty(), "{len} samples");
    }
}

#[test]
fn detector_errors_abort_segmentation() {
    let mut segmenter = Segmenter::new(FailingDetector, SegmentConfig::default());
    let result = segmenter.segment(&speech(samples(1.0)));
    assert!(matches!(result, Err(SegmentError::Onnx(_))));
}

#[test]
fn energy_detector_separates_speech_from_silence() {
    let mut detector = EnergyDetector::new(SAMPLE_RATE);
    assert!(detector.probability(&speech(WINDOW)).unwrap() > 0.95);
    assert_eq!(detector.probability(&silence(WINDOW)).unwrap(), 0.0);
}

#[test]
fn energy_detector_threshold_is_configurable() {
    let faint = tone(SAMPLE_RATE, 255.0, 0.001, WINDOW);
    let mut default = EnergyDetector::new(SAMPLE_RATE);
    let mut lenient = EnergyDetector::with_threshold_dbfs(SAMPLE_RATE, -80.0);
    assert!(lenient.probability(&faint).unwrap() > default.probability(&faint).unwrap());
}

#[test]
fn energy_detector_window_tracks_the_sample_rate() {
    for (sample_rate, window) in [(8_000, 256), (16_000, 512), (48_000, 1536)] {
        assert_eq!(EnergyDetector::new(sample_rate).window_size(), window);
    }
}

#[test]
fn segment_reports_its_length() {
    let segment = Segment {
        start_sample: 1_000,
        end_sample: 17_000,
    };
    assert_eq!(segment.len_samples(), 16_000);
    assert_eq!(segment.duration_s(16_000), 1.0);
    assert_eq!(segment.duration_s(0), 0.0);
}

#[test]
fn with_threshold_derives_the_exit_threshold_like_silero() {
    assert!((SegmentConfig::with_threshold(0.5).neg_threshold - 0.35).abs() < 1e-6);
    assert!((SegmentConfig::with_threshold(0.1).neg_threshold - 0.01).abs() < 1e-6);
    assert_eq!(
        SegmentConfig::default().neg_threshold,
        SegmentConfig::with_threshold(SegmentConfig::default().threshold).neg_threshold
    );
}

proptest! {
    #[test]
    fn any_detector_output_yields_well_formed_bounded_segments(
        len in 0usize..40_000,
        seed in any::<u64>(),
    ) {
        let config = short_clip_config();
        let max_len = samples(config.max_segment_s);

        let segments = Segmenter::new(ScriptedDetector::new(seed), config)
            .segment(&silence(len))
            .unwrap();

        assert_well_formed(&segments, len);
        prop_assert!(segments.iter().all(|segment| segment.len_samples() <= max_len));
    }

    #[test]
    fn energy_driven_segments_respect_the_default_bounds(
        blocks in prop::collection::vec((0.0f32..0.6, 1usize..8), 0..40),
    ) {
        let audio: Vec<f32> = blocks
            .into_iter()
            .flat_map(|(amplitude, quarter_seconds)| {
                tone(SAMPLE_RATE, 220.0, amplitude, quarter_seconds * samples(0.25))
            })
            .collect();
        let max_len = samples(SegmentConfig::default().max_segment_s);

        let segments = segment(&audio);

        prop_assert!(segments.iter().all(|segment| segment.len_samples() <= max_len));
    }

    #[test]
    fn segmentation_is_repeatable(len in 0usize..20_000, seed in any::<u64>()) {
        let audio = silence(len);
        let mut segmenter = Segmenter::new(ScriptedDetector::new(seed), short_clip_config());
        prop_assert_eq!(segmenter.segment(&audio).unwrap(), segmenter.segment(&audio).unwrap());
    }
}
