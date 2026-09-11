//! The hysteresis state machine that turns per-window probabilities into
//! variable-length speech segments.

use crate::detector::SpeechDetector;

/// A half-open range of samples `[start_sample, end_sample)` judged to be speech.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// First sample of the segment, inclusive.
    pub start_sample: usize,
    /// One past the last sample of the segment, exclusive.
    pub end_sample: usize,
}

impl Segment {
    /// Length in samples. Zero if the segment is degenerate.
    #[must_use]
    pub fn len_samples(&self) -> usize {
        self.end_sample.saturating_sub(self.start_sample)
    }

    /// Length in seconds at `sample_rate`. Zero if `sample_rate` is zero.
    #[must_use]
    pub fn duration_s(&self, sample_rate: u32) -> f32 {
        if sample_rate == 0 {
            return 0.0;
        }
        self.len_samples() as f32 / sample_rate as f32
    }
}

/// Tuning for the segmentation state machine.
///
/// Defaults are Silero VAD's documented `get_speech_timestamps` defaults, plus
/// the Emilia-Pipe 3-30 s clip length bounds.
#[derive(Debug, Clone)]
pub struct SegmentConfig {
    /// Sample rate of the input, in Hz. Used to convert every `_ms` / `_s`
    /// field into samples.
    pub sample_rate: u32,
    /// Probability at or above which speech is entered.
    pub threshold: f32,
    /// Probability below which speech may be exited. Conventionally
    /// `max(threshold - 0.15, 0.01)`; the gap is the hysteresis band.
    pub neg_threshold: f32,
    /// Speech runs shorter than this are discarded.
    pub min_speech_duration_ms: u32,
    /// A sub-threshold run must last at least this long to end a segment.
    /// Shorter dips are absorbed, merging the surrounding speech.
    pub min_silence_duration_ms: u32,
    /// Padding added to both ends of every surviving segment.
    pub speech_pad_ms: u32,
    /// Final segments shorter than this are dropped.
    pub min_segment_s: f32,
    /// Final segments longer than this are split.
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
    /// Config with `threshold` set and `neg_threshold` derived as
    /// `max(threshold - 0.15, 0.01)`, per Silero.
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

    fn s_to_samples(&self, s: f32) -> usize {
        if !s.is_finite() || s <= 0.0 {
            return 0;
        }
        let n = (f64::from(s) * f64::from(self.sample_rate)).round();
        if n >= usize::MAX as f64 {
            usize::MAX
        } else {
            n as usize
        }
    }
}

/// A run of consecutive sub-`neg_threshold` windows inside a raw speech run.
#[derive(Debug, Clone, Copy)]
struct SilenceRun {
    start: usize,
    end: usize,
}

impl SilenceRun {
    fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    fn midpoint(&self) -> usize {
        self.start + self.len() / 2
    }
}

/// A speech run as emitted by the state machine, before padding and splitting.
#[derive(Debug, Clone)]
struct RawRun {
    start: usize,
    end: usize,
    /// Interior silence dips that were absorbed (shorter than
    /// `min_silence_duration_ms`), kept as candidate split points.
    interior_silences: Vec<SilenceRun>,
}

/// Hysteresis VAD segmenter.
///
/// Walks the signal in `detector.window_size()` windows, runs a two-threshold
/// state machine over the resulting probabilities, then post-processes the raw
/// speech runs: minimum-duration filtering, padding, long-segment splitting and
/// a final minimum-length drop. See the crate README for the full algorithm.
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

    /// Borrow the detector.
    pub fn detector(&self) -> &D {
        &self.detector
    }

    /// Segment `samples` into speech clips.
    ///
    /// Resets the detector first, so repeated calls are independent. The
    /// returned segments are ordered by `start_sample`, non-overlapping, have
    /// `end_sample > start_sample`, and satisfy `end_sample <= samples.len()`.
    pub fn segment(&mut self, samples: &[f32]) -> Vec<Segment> {
        self.detector.reset();

        let window = self.detector.window_size();
        if window == 0 || samples.len() < window {
            // Nothing can be evaluated: a partial trailing window is never
            // enough evidence to declare speech.
            return Vec::new();
        }

        let raw = self.run_state_machine(samples, window);
        self.finalize(raw, samples.len())
    }

    /// Phase 1 - two-threshold hysteresis over window probabilities.
    fn run_state_machine(&mut self, samples: &[f32], window: usize) -> Vec<RawRun> {
        let min_silence = self.config.ms_to_samples(self.config.min_silence_duration_ms);

        let mut runs: Vec<RawRun> = Vec::new();
        let mut current: Option<RawRun> = None;
        // Start of the ongoing sub-neg_threshold run, if any, while in speech.
        let mut silence_start: Option<usize> = None;

        let mut offset = 0usize;
        while offset + window <= samples.len() {
            let p = self.detector.probability(&samples[offset..offset + window]);
            let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
            let win_end = offset + window;

            match current.as_mut() {
                None => {
                    if p >= self.config.threshold {
                        current = Some(RawRun {
                            start: offset,
                            end: win_end,
                            interior_silences: Vec::new(),
                        });
                        silence_start = None;
                    }
                }
                Some(run) => {
                    if p < self.config.neg_threshold {
                        let s_start = *silence_start.get_or_insert(offset);
                        if win_end - s_start >= min_silence {
                            // Sustained silence: the segment ended where the
                            // silence run *began*, not where it ends.
                            run.end = s_start.max(run.start);
                            // Drop absorbed dips that now fall outside the run.
                            run.interior_silences.retain(|s| s.end <= run.end);
                            let finished = current.take().expect("in speech");
                            runs.push(finished);
                            silence_start = None;
                        }
                    } else {
                        if let Some(s_start) = silence_start.take() {
                            // Dip too short to end the segment: absorb it, but
                            // remember it as a candidate split point.
                            run.interior_silences.push(SilenceRun {
                                start: s_start,
                                end: offset,
                            });
                        }
                        // Above neg_threshold keeps speech alive (hysteresis);
                        // only >= threshold can start it.
                        run.end = win_end;
                    }
                }
            }

            offset += window;
        }

        // Flush an in-progress segment at end of input.
        if let Some(mut run) = current.take() {
            if let Some(s_start) = silence_start {
                // Trailing silence that never reached min_silence still should
                // not be counted as speech.
                run.end = s_start.max(run.start);
                run.interior_silences.retain(|s| s.end <= run.end);
            }
            if run.end > run.start {
                runs.push(run);
            }
        }

        runs
    }

    /// Phase 2 - min-speech filter, padding, splitting, min-length drop.
    fn finalize(&self, raw: Vec<RawRun>, total: usize) -> Vec<Segment> {
        let min_speech = self.config.ms_to_samples(self.config.min_speech_duration_ms);
        let pad = self.config.ms_to_samples(self.config.speech_pad_ms);
        let max_len = self.config.s_to_samples(self.config.max_segment_s);
        let min_len = self.config.s_to_samples(self.config.min_segment_s);

        let mut out: Vec<Segment> = Vec::new();

        for run in raw {
            if run.end <= run.start || run.end - run.start < min_speech {
                continue;
            }

            // Pad, clamped to the buffer. Saturating subtraction keeps a
            // segment starting at sample 0 from underflowing.
            let start = run.start.saturating_sub(pad);
            let end = run.end.saturating_add(pad).min(total);
            if end <= start {
                continue;
            }

            // Interior silences are absolute sample offsets already; keep only
            // those strictly inside the padded segment.
            let silences: Vec<SilenceRun> = run
                .interior_silences
                .iter()
                .filter(|s| s.start > start && s.end < end && s.len() > 0)
                .copied()
                .collect();

            let padded = Segment {
                start_sample: start,
                end_sample: end,
            };
            self.split_long(padded, &silences, max_len, &mut out);
        }

        // Emitted in increasing start order by construction; enforce
        // non-overlap so the invariant holds even for adversarial configs
        // (e.g. a pad larger than the gap between two runs), then apply the
        // final length gate.
        out.sort_by_key(|s| (s.start_sample, s.end_sample));
        let mut deduped: Vec<Segment> = Vec::with_capacity(out.len());
        for mut seg in out {
            if let Some(prev) = deduped.last()
                && seg.start_sample < prev.end_sample
            {
                seg.start_sample = prev.end_sample;
            }
            if seg.end_sample > seg.start_sample
                && seg.end_sample <= total
                && seg.len_samples() >= min_len
            {
                deduped.push(seg);
            }
        }
        deduped
    }

    /// Recursively split `seg` until every piece is at most `max_len` samples.
    ///
    /// Preference order for a cut point:
    /// 1. The midpoint of the **longest interior silence run** that falls
    ///    strictly inside the segment and leaves a non-empty piece on each
    ///    side - the natural place to break a long utterance.
    /// 2. Failing that, a hard cut at exactly `max_len` samples.
    ///
    /// After a cut the silence list is partitioned between the two halves and
    /// each half is re-examined, so a very long segment with several dips is
    /// split at each of them in turn.
    fn split_long(
        &self,
        seg: Segment,
        silences: &[SilenceRun],
        max_len: usize,
        out: &mut Vec<Segment>,
    ) {
        if max_len == 0 || seg.len_samples() <= max_len {
            out.push(seg);
            return;
        }

        // Candidate cut: longest interior silence strictly inside the segment.
        let cut = silences
            .iter()
            .filter(|s| s.start > seg.start_sample && s.end < seg.end_sample)
            .max_by_key(|s| (s.len(), std::cmp::Reverse(s.start)))
            .map(SilenceRun::midpoint)
            .filter(|&c| c > seg.start_sample && c < seg.end_sample)
            // Hard split when there is no usable interior silence.
            .unwrap_or(seg.start_sample + max_len);

        let cut = cut.clamp(seg.start_sample + 1, seg.end_sample - 1);

        let left = Segment {
            start_sample: seg.start_sample,
            end_sample: cut,
        };
        let right = Segment {
            start_sample: cut,
            end_sample: seg.end_sample,
        };

        let (left_sil, right_sil): (Vec<SilenceRun>, Vec<SilenceRun>) =
            silences.iter().partition(|s| s.end <= cut);

        self.split_long(left, &left_sil, max_len, out);
        self.split_long(right, &right_sil, max_len, out);
    }
}
