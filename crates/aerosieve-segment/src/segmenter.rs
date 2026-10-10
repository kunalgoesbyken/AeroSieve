use std::cmp::Reverse;

use crate::detector::SpeechDetector;
use crate::error::SegmentError;

/// A half-open range of samples, `[start_sample, end_sample)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// First sample, inclusive.
    pub start_sample: usize,
    /// One past the last sample.
    pub end_sample: usize,
}

impl Segment {
    /// Length in samples.
    #[must_use]
    pub fn len_samples(&self) -> usize {
        self.end_sample.saturating_sub(self.start_sample)
    }

    /// Length in seconds at `sample_rate`; zero if `sample_rate` is zero.
    #[must_use]
    pub fn duration_s(&self, sample_rate: u32) -> f32 {
        if sample_rate == 0 {
            return 0.0;
        }
        self.len_samples() as f32 / sample_rate as f32
    }

    fn midpoint(&self) -> usize {
        self.start_sample + self.len_samples() / 2
    }

    fn strictly_contains(&self, inner: &Segment) -> bool {
        inner.start_sample > self.start_sample && inner.end_sample < self.end_sample
    }
}

/// Tuning for [`Segmenter`].
///
/// Defaults are Silero VAD's `get_speech_timestamps` defaults, plus the
/// 3-30 s clip bounds used by Emilia-Pipe.
#[derive(Debug, Clone)]
pub struct SegmentConfig {
    /// Sample rate of the audio being segmented, in Hz.
    pub sample_rate: u32,
    /// Probability at or above which speech starts.
    pub threshold: f32,
    /// Probability below which a window counts as silence once speech has
    /// started. The gap to `threshold` is the hysteresis band.
    pub neg_threshold: f32,
    /// Speech runs shorter than this are discarded.
    pub min_speech_duration_ms: u32,
    /// Silence must last this long to end a segment; shorter dips are bridged.
    pub min_silence_duration_ms: u32,
    /// Padding added to both ends of every segment.
    pub speech_pad_ms: u32,
    /// Segments shorter than this are dropped.
    pub min_segment_s: f32,
    /// Segments longer than this are split. Zero disables splitting.
    pub max_segment_s: f32,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            threshold: 0.5,
            neg_threshold: 0.35,
            min_speech_duration_ms: 250,
            min_silence_duration_ms: 100,
            speech_pad_ms: 30,
            min_segment_s: 3.0,
            max_segment_s: 30.0,
        }
    }
}

impl SegmentConfig {
    /// Defaults with `threshold` replaced and `neg_threshold` derived from it
    /// the way Silero does: `max(threshold - 0.15, 0.01)`.
    #[must_use]
    pub fn with_threshold(threshold: f32) -> Self {
        Self {
            threshold,
            neg_threshold: (threshold - 0.15).max(0.01),
            ..Self::default()
        }
    }

    fn ms_to_samples(&self, ms: u32) -> usize {
        (u64::from(ms) * u64::from(self.sample_rate) / 1000) as usize
    }

    fn seconds_to_samples(&self, seconds: f32) -> usize {
        (f64::from(seconds) * f64::from(self.sample_rate)).round() as usize
    }
}

/// A stretch of speech found by the state machine, with the brief silences
/// it bridged. Those are where a too-long segment is later cut.
struct SpeechRun {
    span: Segment,
    dips: Vec<Segment>,
}

/// Two-threshold state machine over per-window probabilities.
struct Hysteresis {
    enter: f32,
    exit: f32,
    min_silence: usize,
    open: Option<SpeechRun>,
    silence_start: Option<usize>,
    closed: Vec<SpeechRun>,
}

impl Hysteresis {
    fn new(config: &SegmentConfig) -> Self {
        Self {
            enter: config.threshold,
            exit: config.neg_threshold,
            min_silence: config.ms_to_samples(config.min_silence_duration_ms),
            open: None,
            silence_start: None,
            closed: Vec::new(),
        }
    }

    fn observe(&mut self, window: Segment, probability: f32) {
        let Some(run) = self.open.as_mut() else {
            if probability >= self.enter {
                self.open = Some(SpeechRun {
                    span: window,
                    dips: Vec::new(),
                });
            }
            return;
        };

        if probability >= self.exit {
            if let Some(dip_start) = self.silence_start.take() {
                run.dips.push(Segment {
                    start_sample: dip_start,
                    end_sample: window.start_sample,
                });
            }
            run.span.end_sample = window.end_sample;
            return;
        }

        let silence_start = *self.silence_start.get_or_insert(window.start_sample);
        if window.end_sample - silence_start >= self.min_silence {
            self.close();
        }
    }

    /// End the open run where its last non-silent window ended.
    fn close(&mut self) {
        self.silence_start = None;
        self.closed.extend(self.open.take());
    }

    fn finish(mut self) -> Vec<SpeechRun> {
        self.close();
        self.closed
    }
}

/// Cuts audio into speech segments using a [`SpeechDetector`].
///
/// Speech starts at the first window scoring at least `threshold` and ends
/// once windows have stayed below `neg_threshold` for
/// `min_silence_duration_ms`. Runs are then filtered by minimum duration,
/// padded, split to `max_segment_s`, and filtered by `min_segment_s`.
pub struct Segmenter<D: SpeechDetector> {
    detector: D,
    config: SegmentConfig,
}

impl<D: SpeechDetector> Segmenter<D> {
    /// Build a segmenter from a detector and a config.
    pub fn new(detector: D, config: SegmentConfig) -> Self {
        Self { detector, config }
    }

    /// The configuration in use.
    pub fn config(&self) -> &SegmentConfig {
        &self.config
    }

    /// Segment `samples` into speech clips.
    ///
    /// The result is ordered, non-overlapping, and within `samples`. A
    /// trailing partial window is not evaluated. The detector is reset first,
    /// so calls are independent.
    pub fn segment(&mut self, samples: &[f32]) -> Result<Vec<Segment>, SegmentError> {
        self.detector.reset();
        let window = self.detector.window_size();
        if window == 0 {
            return Ok(Vec::new());
        }

        let mut hysteresis = Hysteresis::new(&self.config);
        for (index, chunk) in samples.chunks_exact(window).enumerate() {
            let start_sample = index * window;
            let span = Segment {
                start_sample,
                end_sample: start_sample + window,
            };
            hysteresis.observe(span, self.detector.probability(chunk)?);
        }

        Ok(self.shape(hysteresis.finish(), samples.len()))
    }

    /// Turn raw speech runs into final segments.
    fn shape(&self, runs: Vec<SpeechRun>, total_samples: usize) -> Vec<Segment> {
        let config = &self.config;
        let min_speech = config.ms_to_samples(config.min_speech_duration_ms);
        let pad = config.ms_to_samples(config.speech_pad_ms);
        let max_len = config.seconds_to_samples(config.max_segment_s);
        let min_len = config.seconds_to_samples(config.min_segment_s).max(1);

        let mut pieces = Vec::new();
        for run in runs {
            if run.span.len_samples() < min_speech {
                continue;
            }
            let padded = Segment {
                start_sample: run.span.start_sample.saturating_sub(pad),
                end_sample: run.span.end_sample.saturating_add(pad).min(total_samples),
            };
            split_to_max_len(padded, &run.dips, max_len, &mut pieces);
        }

        let mut segments = Vec::with_capacity(pieces.len());
        let mut previous_end = 0;
        for piece in pieces {
            let segment = Segment {
                start_sample: piece.start_sample.max(previous_end),
                end_sample: piece.end_sample,
            };
            if segment.len_samples() >= min_len {
                previous_end = segment.end_sample;
                segments.push(segment);
            }
        }
        segments
    }
}

/// Append `segment` to `pieces`, cut so that no piece exceeds `max_len`.
///
/// Each cut lands in the middle of the longest bridged silence inside the
/// piece being cut, or at exactly `max_len` when there is none.
fn split_to_max_len(segment: Segment, dips: &[Segment], max_len: usize, pieces: &mut Vec<Segment>) {
    let mut pending = vec![segment];
    while let Some(piece) = pending.pop() {
        if max_len == 0 || piece.len_samples() <= max_len {
            pieces.push(piece);
            continue;
        }
        let cut = dips
            .iter()
            .filter(|dip| piece.strictly_contains(dip))
            .max_by_key(|dip| (dip.len_samples(), Reverse(dip.start_sample)))
            .map_or(piece.start_sample + max_len, Segment::midpoint);

        pending.push(Segment {
            start_sample: cut,
            end_sample: piece.end_sample,
        });
        pending.push(Segment {
            start_sample: piece.start_sample,
            end_sample: cut,
        });
    }
}
