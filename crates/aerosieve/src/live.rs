use aerosieve_acoustic::FULL_SCALE;
use aerosieve_segment::StreamSegmenter;

use crate::{EnergyDetector, FilterPolicy, PipelineError, Segment, SegmentOutcome, SpeechDetector};

/// Segments and scores audio that arrives in pieces, such as a microphone
/// or a call, judging each clip as soon as the silence that ends it arrives.
///
/// Fed the same samples in any chunking, it reaches the same verdicts as
/// [`Inspector::inspect_audio`](crate::Inspector::inspect_audio) on the
/// whole buffer. Samples are judged as received: there is no loudness
/// normalization, and full scale is 1.0. Only the audio of the utterance in
/// progress is held in memory.
pub struct LiveInspector<D: SpeechDetector = EnergyDetector> {
    segmenter: StreamSegmenter<D>,
    recent: Recent,
}

/// The tail of the stream that a pending clip may still need, and the
/// policy that judges clips cut from it.
struct Recent {
    policy: FilterPolicy,
    sample_rate: u32,
    samples: Vec<f32>,
    first_sample: usize,
}

impl<D: SpeechDetector> LiveInspector<D> {
    pub(crate) fn new(
        segmenter: StreamSegmenter<D>,
        policy: FilterPolicy,
        sample_rate: u32,
    ) -> Self {
        Self {
            segmenter,
            recent: Recent {
                policy,
                sample_rate,
                samples: Vec::new(),
                first_sample: 0,
            },
        }
    }

    /// Feed the next samples and return the clips they completed, in order.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<SegmentOutcome>, PipelineError> {
        self.recent.samples.extend_from_slice(samples);
        let outcomes = self.recent.judge(self.segmenter.push(samples)?);
        self.recent.discard_before(self.segmenter.retain_from());
        Ok(outcomes)
    }

    /// End the stream and judge the clips still pending.
    pub fn finish(self) -> Vec<SegmentOutcome> {
        self.recent.judge(self.segmenter.finish())
    }

    /// Seconds of audio received so far.
    pub fn heard_s(&self) -> f32 {
        let heard = self.recent.first_sample + self.recent.samples.len();
        heard as f32 / self.recent.sample_rate as f32
    }

    /// Seconds of audio held because a clip not yet judged may need it:
    /// roughly the utterance in progress. Bounded only by how long speakers
    /// go without pausing, so servers should cap it.
    pub fn held_s(&self) -> f32 {
        self.recent.samples.len() as f32 / self.recent.sample_rate as f32
    }

    /// Rate the clips are expressed in; pushed audio must be at this rate.
    pub fn sample_rate(&self) -> u32 {
        self.recent.sample_rate
    }
}

impl Recent {
    fn judge(&self, segments: Vec<Segment>) -> Vec<SegmentOutcome> {
        segments
            .into_iter()
            .map(|segment| {
                let start = segment.start_sample - self.first_sample;
                let clip = &self.samples[start..start + segment.len_samples()];
                crate::judge(&self.policy, segment, clip, self.sample_rate, FULL_SCALE)
            })
            .collect()
    }

    fn discard_before(&mut self, sample: usize) {
        let stale = sample
            .saturating_sub(self.first_sample)
            .min(self.samples.len());
        self.samples.drain(..stale);
        self.first_sample += stale;
    }
}
