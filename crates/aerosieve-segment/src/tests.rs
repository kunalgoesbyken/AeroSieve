//! Unit and property tests for the hysteresis segmenter.
//!
//! All tests run under default features (no ONNX). Randomness comes from a
//! deterministic xorshift PRNG defined here, not from a `rand` dependency.

use crate::{EnergyDetector, Segment, SegmentConfig, Segmenter, SpeechDetector};

const SR: u32 = 16_000;

/// Deterministic xorshift64* PRNG. No external dependency, reproducible runs.
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0.0, 1.0)`.
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }
}

// ---------------------------------------------------------------------------
// signal builders
// ---------------------------------------------------------------------------

/// Digital silence.
fn silence(n: usize) -> Vec<f32> {
    vec![0.0; n]
}

/// Speech-like buffer: a 140 Hz carrier with harmonics at ~0.3 amplitude,
/// comfortably above the -45 dBFS energy threshold.
fn speech(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 140.0 * std::f32::consts::TAU;
            0.30 * ((f * t).sin() + 0.5 * (2.0 * f * t).sin() + 0.25 * (3.0 * f * t).sin()) / 1.75
        })
        .collect()
}

fn ms(n: u32) -> usize {
    (n as usize * SR as usize) / 1000
}

fn default_config() -> SegmentConfig {
    SegmentConfig {
        sample_rate: SR,
        ..Default::default()
    }
}

fn segment_with(config: SegmentConfig, samples: &[f32]) -> Vec<Segment> {
    let mut s = Segmenter::new(EnergyDetector::new(SR), config);
    s.segment(samples)
}

fn assert_invariants(segs: &[Segment], total: usize) {
    let mut prev_end = 0usize;
    for (i, seg) in segs.iter().enumerate() {
        assert!(
            seg.end_sample > seg.start_sample,
            "segment {i} is empty or inverted: {seg:?}"
        );
        assert!(
            seg.end_sample <= total,
            "segment {i} exceeds buffer ({} > {total}): {seg:?}",
            seg.end_sample
        );
        assert!(
            seg.start_sample >= prev_end,
            "segment {i} overlaps predecessor (start {} < prev end {prev_end})",
            seg.start_sample
        );
        prev_end = seg.end_sample;
    }
}

// ---------------------------------------------------------------------------
// state machine behaviour
// ---------------------------------------------------------------------------

#[test]
fn silence_only_yields_no_segments() {
    let samples = silence(SR as usize * 10);
    let segs = segment_with(default_config(), &samples);
    assert!(segs.is_empty(), "expected no segments, got {segs:?}");
    assert_invariants(&segs, samples.len());
}

#[test]
fn single_burst_yields_one_segment_with_right_bounds() {
    let lead = SR as usize; // 1 s
    let burst = SR as usize * 5; // 5 s
    let mut samples = silence(lead);
    samples.extend(speech(burst));
    samples.extend(silence(SR as usize));

    let segs = segment_with(default_config(), &samples);
    assert_invariants(&segs, samples.len());
    assert_eq!(segs.len(), 1, "expected exactly one segment, got {segs:?}");

    let seg = segs[0];
    let pad = ms(30);
    let tol = 2 * 512 + pad; // two detector windows of slop plus the padding

    assert!(
        seg.start_sample.abs_diff(lead) <= tol,
        "start {} far from {lead}",
        seg.start_sample
    );
    assert!(
        seg.end_sample.abs_diff(lead + burst) <= tol,
        "end {} far from {}",
        seg.end_sample,
        lead + burst
    );
    let d = seg.duration_s(SR);
    assert!((4.5..=5.5).contains(&d), "duration {d} s not near 5 s");
}

#[test]
fn two_bursts_with_long_gap_yield_two_segments() {
    // Gap of 1 s >> min_silence_duration_ms (100 ms).
    let mut samples = silence(ms(500));
    samples.extend(speech(SR as usize * 4));
    samples.extend(silence(SR as usize));
    samples.extend(speech(SR as usize * 4));
    samples.extend(silence(ms(500)));

    let segs = segment_with(default_config(), &samples);
    assert_invariants(&segs, samples.len());
    assert_eq!(segs.len(), 2, "expected two segments, got {segs:?}");
    for seg in &segs {
        let d = seg.duration_s(SR);
        assert!((3.5..=4.5).contains(&d), "segment duration {d} s off");
    }
}

#[test]
fn two_bursts_with_short_gap_merge_into_one() {
    // Gap of 50 ms < min_silence_duration_ms (100 ms): the hysteresis payoff.
    let mut samples = silence(ms(500));
    samples.extend(speech(SR as usize * 4));
    samples.extend(silence(ms(50)));
    samples.extend(speech(SR as usize * 4));
    samples.extend(silence(ms(500)));

    let segs = segment_with(default_config(), &samples);
    assert_invariants(&segs, samples.len());
    assert_eq!(
        segs.len(),
        1,
        "short gap should merge into one segment, got {segs:?}"
    );
    let d = segs[0].duration_s(SR);
    assert!((7.5..=8.6).contains(&d), "merged duration {d} s not near 8 s");
}

#[test]
fn burst_shorter_than_min_speech_duration_is_dropped() {
    // 150 ms of speech < min_speech_duration_ms (250 ms).
    let mut samples = silence(SR as usize);
    samples.extend(speech(ms(150)));
    samples.extend(silence(SR as usize));

    let segs = segment_with(default_config(), &samples);
    assert!(segs.is_empty(), "short burst should be dropped, got {segs:?}");
}

#[test]
fn burst_between_min_speech_and_min_segment_is_dropped_at_the_end() {
    // 1 s of speech passes min_speech_duration_ms (250 ms) but fails
    // min_segment_s (3 s), so it is dropped by the final length gate.
    let mut samples = silence(SR as usize);
    samples.extend(speech(SR as usize));
    samples.extend(silence(SR as usize));

    let segs = segment_with(default_config(), &samples);
    assert!(
        segs.is_empty(),
        "1 s segment should fail min_segment_s, got {segs:?}"
    );
}

// ---------------------------------------------------------------------------
// splitting
// ---------------------------------------------------------------------------

#[test]
fn sixty_second_burst_splits_into_bounded_segments() {
    let samples = speech(SR as usize * 60);
    let cfg = default_config();
    let max_samples = (cfg.max_segment_s * SR as f32) as usize;

    let segs = segment_with(cfg, &samples);
    assert_invariants(&segs, samples.len());
    assert!(
        segs.len() >= 2,
        "60 s of speech must split, got {} segment(s)",
        segs.len()
    );
    for seg in &segs {
        assert!(
            seg.len_samples() <= max_samples,
            "segment {seg:?} exceeds max_segment_s ({} > {max_samples})",
            seg.len_samples()
        );
        assert!(
            seg.duration_s(SR) >= 3.0,
            "split produced a segment below min_segment_s: {} s",
            seg.duration_s(SR)
        );
    }
}

#[test]
fn long_burst_splits_at_the_interior_silence() {
    // 40 s of speech with a single 50 ms dip at t = 20 s. The dip is shorter
    // than min_silence_duration_ms so it does not end the segment, but it is
    // the longest interior silence and must become the split point.
    let half = SR as usize * 20;
    let dip = ms(50);
    let mut samples = speech(half);
    samples.extend(silence(dip));
    samples.extend(speech(half));

    let segs = segment_with(default_config(), &samples);
    assert_invariants(&segs, samples.len());
    assert_eq!(segs.len(), 2, "expected a split at the dip, got {segs:?}");

    let cut = segs[0].end_sample;
    let expected = half + dip / 2;
    assert!(
        cut.abs_diff(expected) <= 4 * 512,
        "split at {cut}, expected near {expected}"
    );
}

// ---------------------------------------------------------------------------
// padding and edges
// ---------------------------------------------------------------------------

#[test]
fn padding_clamps_at_both_buffer_edges() {
    // Speech occupies the entire buffer: padding must not underflow at 0 nor
    // run past the end.
    let samples = speech(SR as usize * 5);
    let segs = segment_with(default_config(), &samples);
    assert_invariants(&segs, samples.len());
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].start_sample, 0, "start must clamp to 0");
    assert!(
        segs[0].end_sample <= samples.len(),
        "end must clamp to buffer length"
    );
}

#[test]
fn padding_is_actually_applied() {
    let lead = SR as usize;
    let mut samples = silence(lead);
    samples.extend(speech(SR as usize * 5));
    samples.extend(silence(SR as usize));

    let padded = segment_with(default_config(), &samples);
    let unpadded = segment_with(
        SegmentConfig {
            speech_pad_ms: 0,
            ..default_config()
        },
        &samples,
    );

    assert_eq!(padded.len(), 1);
    assert_eq!(unpadded.len(), 1);
    assert!(
        padded[0].start_sample < unpadded[0].start_sample,
        "padding should move the start earlier: {padded:?} vs {unpadded:?}"
    );
    assert!(
        padded[0].end_sample > unpadded[0].end_sample,
        "padding should move the end later: {padded:?} vs {unpadded:?}"
    );
    assert_eq!(
        unpadded[0].start_sample - padded[0].start_sample,
        ms(30),
        "start padding should equal speech_pad_ms"
    );
}

#[test]
fn empty_input_yields_no_segments() {
    let segs = segment_with(default_config(), &[]);
    assert!(segs.is_empty());
}

#[test]
fn input_shorter_than_one_window_does_not_panic() {
    for n in [1usize, 2, 100, 511] {
        let segs = segment_with(default_config(), &speech(n));
        assert!(segs.is_empty(), "n={n} produced {segs:?}");
        let segs = segment_with(default_config(), &silence(n));
        assert!(segs.is_empty(), "n={n} silence produced {segs:?}");
    }
}

#[test]
fn exactly_one_window_does_not_panic() {
    let segs = segment_with(default_config(), &speech(512));
    assert_invariants(&segs, 512);
}

// ---------------------------------------------------------------------------
// detector behaviour
// ---------------------------------------------------------------------------

#[test]
fn energy_detector_saturates_for_speech_and_silence() {
    let mut d = EnergyDetector::new(SR);
    assert_eq!(d.window_size(), 512);

    let loud = speech(512);
    let p_loud = d.probability(&loud);
    assert!(p_loud > 0.95, "loud audio scored {p_loud}, expected ~1.0");

    let quiet = silence(512);
    let p_quiet = d.probability(&quiet);
    assert!(p_quiet < 0.05, "silence scored {p_quiet}, expected ~0.0");
}

#[test]
fn energy_detector_threshold_is_configurable() {
    let quiet: Vec<f32> = (0..512).map(|i| 0.001 * (i as f32 * 0.1).sin()).collect();
    let mut strict = EnergyDetector::new(SR);
    let mut lenient = EnergyDetector::with_threshold_dbfs(SR, -80.0);
    assert!(lenient.probability(&quiet) > strict.probability(&quiet));
}

#[test]
fn energy_detector_uses_256_samples_at_8k() {
    assert_eq!(EnergyDetector::new(8_000).window_size(), 256);
    assert_eq!(EnergyDetector::new(16_000).window_size(), 512);
}

#[test]
fn segment_helpers() {
    let s = Segment {
        start_sample: 1_000,
        end_sample: 17_000,
    };
    assert_eq!(s.len_samples(), 16_000);
    assert!((s.duration_s(16_000) - 1.0).abs() < f32::EPSILON);
    assert_eq!(s.duration_s(0), 0.0);
}

#[test]
fn default_config_matches_silero_documented_defaults() {
    let c = SegmentConfig::default();
    assert_eq!(c.sample_rate, 16_000);
    assert!((c.threshold - 0.5).abs() < f32::EPSILON);
    assert!((c.neg_threshold - 0.35).abs() < 1e-6);
    assert_eq!(c.min_speech_duration_ms, 250);
    assert_eq!(c.min_silence_duration_ms, 100);
    assert_eq!(c.speech_pad_ms, 30);
    assert!((c.min_segment_s - 3.0).abs() < f32::EPSILON);
    assert!((c.max_segment_s - 30.0).abs() < f32::EPSILON);

    // neg_threshold derivation: max(threshold - 0.15, 0.01)
    assert!((SegmentConfig::with_threshold(0.5).neg_threshold - 0.35).abs() < 1e-6);
    assert!((SegmentConfig::with_threshold(0.1).neg_threshold - 0.01).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// property tests
// ---------------------------------------------------------------------------

/// Detector that ignores the audio entirely and replays a deterministic
/// pseudo-random probability sequence.
///
/// Decoupling the probabilities from the samples is the point: the state
/// machine's invariants must hold for *any* detector behaviour, including one
/// that flips between speech and silence every window. Seeding from proptest's
/// generated value keeps each case reproducible, and `reset` rewinds the
/// sequence so `segment` remains idempotent.
struct MockDetector {
    seed: u64,
    rng: XorShift,
    window: usize,
}

impl MockDetector {
    fn new(seed: u64, window: usize) -> Self {
        Self {
            seed,
            rng: XorShift::new(seed),
            window,
        }
    }
}

impl SpeechDetector for MockDetector {
    fn window_size(&self) -> usize {
        self.window
    }

    fn probability(&mut self, _window: &[f32]) -> f32 {
        self.rng.next_f32()
    }

    fn reset(&mut self) {
        self.rng = XorShift::new(self.seed);
    }
}

#[test]
fn mock_detector_is_deterministic_across_resets() {
    let mut d = MockDetector::new(42, 512);
    let buf = [0.0f32; 512];
    let first: Vec<f32> = (0..16).map(|_| d.probability(&buf)).collect();
    d.reset();
    let second: Vec<f32> = (0..16).map(|_| d.probability(&buf)).collect();
    assert_eq!(first, second);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    /// For arbitrary audio and arbitrary detector behaviour, emitted segments
    /// are ordered by start, non-overlapping, non-empty, and inside the buffer.
    #[test]
    fn segments_are_ordered_disjoint_and_in_bounds(
        len in 0usize..40_000,
        seed in proptest::prelude::any::<u64>(),
        amplitude in 0.0f32..1.0,
    ) {
        let mut noise = XorShift::new(seed ^ 0xA5A5_A5A5);
        let samples: Vec<f32> = (0..len)
            .map(|_| (noise.next_f32() * 2.0 - 1.0) * amplitude)
            .collect();

        let mut s = Segmenter::new(
            MockDetector::new(seed, 512),
            SegmentConfig {
                sample_rate: SR,
                // Loosen the length gates so the property exercises many
                // segments rather than filtering almost everything away.
                min_segment_s: 0.05,
                max_segment_s: 0.5,
                ..Default::default()
            },
        );
        let segs = s.segment(&samples);

        let mut prev_end = 0usize;
        for seg in &segs {
            proptest::prop_assert!(seg.end_sample > seg.start_sample);
            proptest::prop_assert!(seg.end_sample <= samples.len());
            proptest::prop_assert!(seg.start_sample >= prev_end);
            prev_end = seg.end_sample;
        }
    }

    /// The same property with the shipped default config, and with real
    /// energy-driven probabilities rather than a mock.
    #[test]
    fn energy_detector_segments_hold_invariants(
        blocks in proptest::collection::vec((0.0f32..0.6, 1usize..8), 0..40),
    ) {
        let mut samples: Vec<f32> = Vec::new();
        for (amp, quarter_seconds) in blocks {
            let n = quarter_seconds * (SR as usize / 4);
            let start = samples.len();
            samples.extend((0..n).map(|i| {
                let t = (start + i) as f32 / SR as f32;
                amp * (220.0 * std::f32::consts::TAU * t).sin()
            }));
        }

        let segs = segment_with(default_config(), &samples);
        let max_samples = (30.0 * SR as f32) as usize;

        let mut prev_end = 0usize;
        for seg in &segs {
            proptest::prop_assert!(seg.end_sample > seg.start_sample);
            proptest::prop_assert!(seg.end_sample <= samples.len());
            proptest::prop_assert!(seg.start_sample >= prev_end);
            proptest::prop_assert!(seg.len_samples() <= max_samples);
            prev_end = seg.end_sample;
        }
    }

    /// Segmentation is idempotent: the detector reset at the top of `segment`
    /// means a second call on the same buffer returns the same segments.
    #[test]
    fn segmentation_is_repeatable(
        len in 0usize..20_000,
        seed in proptest::prelude::any::<u64>(),
    ) {
        let mut noise = XorShift::new(seed);
        let samples: Vec<f32> = (0..len).map(|_| noise.next_f32() * 2.0 - 1.0).collect();

        let mut s = Segmenter::new(MockDetector::new(seed, 512), default_config());
        let a = s.segment(&samples);
        let b = s.segment(&samples);
        proptest::prop_assert_eq!(a, b);
    }
}
