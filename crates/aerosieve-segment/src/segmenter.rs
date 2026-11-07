use std::cmp::Reverse;
use std::collections::VecDeque;

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
/// Defaults are Silero VAD's `get_speech_timestamps` defaults, except that
/// a pause must last 500 ms rather than 100 ms to end a clip, plus the
/// 3-30 s clip bounds used by Emilia-Pipe. Silero's 100 ms suits detecting
/// speech; for building clips it cuts natural speech at every word gap into
/// fragments too short to keep.
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
            min_silence_duration_ms: 500,
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
    closed: VecDeque<SpeechRun>,
}

impl Hysteresis {
    fn new(config: &SegmentConfig) -> Self {
        Self {
            enter: config.threshold,
            exit: config.neg_threshold,
            min_silence: config.ms_to_samples(config.min_silence_duration_ms),
            open: None,
            silence_start: None,
            closed: VecDeque::new(),
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

    /// Start of the earliest run not yet handed out, open or closed.
    fn earliest_start(&self) -> Option<usize> {
        self.closed
            .front()
            .or(self.open.as_ref())
            .map(|run| run.span.start_sample)
    }
}

/// Turns closed speech runs into final segments, one run at a time.
struct Shaper {
    min_speech: usize,
    pad: usize,
    max_len: usize,
    min_len: usize,
    previous_end: usize,
}

impl Shaper {
    fn new(config: &SegmentConfig) -> Self {
        Self {
            min_speech: config.ms_to_samples(config.min_speech_duration_ms),
            pad: config.ms_to_samples(config.speech_pad_ms),
            max_len: config.seconds_to_samples(config.max_segment_s),
            min_len: config.seconds_to_samples(config.min_segment_s).max(1),
            previous_end: 0,
        }
    }

    /// Drop a too-short run, otherwise pad it (never past `available`),
    /// split it to the length bounds, and append the pieces long enough to
    /// keep.
    fn shape(&mut self, run: SpeechRun, available: usize, segments: &mut Vec<Segment>) {
        if run.span.len_samples() < self.min_speech {
            return;
        }
        let padded = Segment {
            start_sample: run.span.start_sample.saturating_sub(self.pad),
            end_sample: run.span.end_sample.saturating_add(self.pad).min(available),
        };
        let mut pieces = Vec::new();
        split_to_max_len(padded, &run.dips, self.max_len, &mut pieces);
        for piece in pieces {
            let segment = Segment {
                start_sample: piece.start_sample.max(self.previous_end),
                end_sample: piece.end_sample,
            };
            if segment.len_samples() >= self.min_len {
                self.previous_end = segment.end_sample;
                segments.push(segment);
            }
        }
    }
}

/// Segments audio that arrives in pieces, emitting each clip as soon as the
/// silence that ends it has been heard.
///
/// Feeding the same audio in any chunking yields exactly the segments
/// [`Segmenter::segment`] returns for the whole buffer. A clip is emitted
/// `min_silence_duration_ms` (or `speech_pad_ms`, if longer) after its
/// speech ends.
pub struct StreamSegmenter<D: SpeechDetector> {
    detector: D,
    hysteresis: Hysteresis,
    shaper: Shaper,
    partial: Vec<f32>,
    windows_seen: usize,
    received: usize,
}

impl<D: SpeechDetector> StreamSegmenter<D> {
    /// Start a stream; the detector is reset first.
    pub fn new(mut detector: D, config: SegmentConfig) -> Self {
        detector.reset();
        Self {
            hysteresis: Hysteresis::new(&config),
            shaper: Shaper::new(&config),
            partial: Vec::with_capacity(detector.window_size()),
            detector,
            windows_seen: 0,
            received: 0,
        }
    }

    /// Feed the next samples and return the segments they completed, in
    /// order, as absolute positions in the stream.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<Segment>, SegmentError> {
        let window = self.detector.window_size();
        self.received += samples.len();
        if window == 0 {
            return Ok(Vec::new());
        }

        let mut rest = samples;
        if !self.partial.is_empty() {
            let (head, tail) = rest.split_at((window - self.partial.len()).min(rest.len()));
            self.partial.extend_from_slice(head);
            rest = tail;
            if self.partial.len() == window {
                let full = std::mem::take(&mut self.partial);
                self.observe(&full)?;
                self.partial = full;
                self.partial.clear();
            }
        }
        let mut windows = rest.chunks_exact(window);
        for chunk in &mut windows {
            self.observe(chunk)?;
        }
        self.partial.extend_from_slice(windows.remainder());

        Ok(self.release(false))
    }

    /// End the stream and return the segments still pending. A trailing
    /// partial window is not evaluated.
    pub fn finish(mut self) -> Vec<Segment> {
        self.hysteresis.close();
        self.release(true)
    }

    /// Earliest sample a segment not yet emitted can start at. Audio before
    /// it will never be part of a segment and can be discarded.
    pub fn retain_from(&self) -> usize {
        let next_window = self.windows_seen * self.detector.window_size();
        self.hysteresis
            .earliest_start()
            .unwrap_or(next_window)
            .saturating_sub(self.shaper.pad)
            .max(self.shaper.previous_end)
    }

    fn observe(&mut self, window: &[f32]) -> Result<(), SegmentError> {
        let start_sample = self.windows_seen * window.len();
        let span = Segment {
            start_sample,
            end_sample: start_sample + window.len(),
        };
        let probability = self.detector.probability(window)?;
        self.windows_seen += 1;
        self.hysteresis.observe(span, probability);
        Ok(())
    }

    /// Shape closed runs whose trailing padding has arrived, or all of them
    /// when the stream has ended.
    fn release(&mut self, ended: bool) -> Vec<Segment> {
        let mut segments = Vec::new();
        while let Some(run) = self.hysteresis.closed.front() {
            if !ended && run.span.end_sample + self.shaper.pad > self.received {
                break;
            }
            let run = self.hysteresis.closed.pop_front().expect("front exists");
            self.shaper.shape(run, self.received, &mut segments);
        }
        segments
    }
}

/// Cuts audio into speech segments using a [`SpeechDetector`].
///
/// Speech starts at the first window scoring at least `threshold` and ends
/// once windows have stayed below `neg_threshold` for
/// `min_silence_duration_ms`. Runs are then filtered by minimum duration,
/// padded, split to `max_segment_s`, and filtered by `min_segment_s`.
#[derive(Clone)]
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
        let mut stream = StreamSegmenter::new(&mut self.detector, self.config.clone());
        let mut segments = stream.push(samples)?;
        segments.extend(stream.finish());
        Ok(segments)
    }

    /// A [`StreamSegmenter`] with this configuration and a fresh copy of the
    /// detector, for audio that arrives in pieces.
    pub fn stream(&self) -> StreamSegmenter<D>
    where
        D: Clone,
    {
        StreamSegmenter::new(self.detector.clone(), self.config.clone())
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
