//! AeroSieve: a batch audio curation pipeline.
//!
//! Audio files go in; [WebDataset] shards of clean speech clips come out.
//! The stages follow [Emilia-Pipe]:
//!
//! 1. **Decode** ([`decode`]): any supported container to mono `f32` at one
//!    sample rate, loudness-normalized.
//! 2. **Segment** ([`segment`]): a hysteresis VAD cuts the audio into
//!    variable-length speech clips.
//! 3. **Score** ([`acoustic`]): each clip is measured and judged against a
//!    [`FilterPolicy`].
//! 4. **Shard** ([`shard`]): accepted clips are written to tar shards with a
//!    manifest.
//!
//! ```no_run
//! use aerosieve::{Pipeline, PipelineConfig};
//!
//! let config = PipelineConfig::new("data/staging", "data/clean");
//! let mut pipeline = Pipeline::new(config)?;
//!
//! pipeline.process_file("recording.wav".as_ref(), Some("hello world"))?;
//!
//! let stats = pipeline.finish()?;
//! println!("retention {:.1}%", stats.retention_rate() * 100.0);
//! # Ok::<(), aerosieve::PipelineError>(())
//! ```
//!
//! [WebDataset]: https://github.com/webdataset/webdataset
//! [Emilia-Pipe]: https://arxiv.org/abs/2407.05361

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use aerosieve_acoustic as acoustic;
pub use aerosieve_decode as decode;
pub use aerosieve_lexical as lexical;
pub use aerosieve_segment as segment;
pub use aerosieve_shard as shard;

pub use aerosieve_acoustic::{FilterPolicy, QualityMetrics, RejectReason};
pub use aerosieve_decode::{DecodeConfig, DecodedAudio};
pub use aerosieve_segment::{EnergyDetector, Segment, SegmentConfig, SpeechDetector};
pub use aerosieve_shard::{ShardConfig, ShardStats};

use aerosieve_lexical::RuleEngine;
use aerosieve_segment::Segmenter;
use aerosieve_shard::{SampleMeta, ShardWriter};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("decode failed: {0}")]
    Decode(#[from] aerosieve_decode::DecodeError),
    #[error("segmentation failed: {0}")]
    Segment(#[from] aerosieve_segment::SegmentError),
    #[error("shard write failed: {0}")]
    Shard(#[from] aerosieve_shard::ShardError),
    #[error("normalization rules failed to load: {0}")]
    Lexical(#[from] aerosieve_lexical::LexicalError),
    #[error("audio is {audio} Hz but the segmenter is configured for {segmenter} Hz")]
    SampleRateMismatch { audio: u32, segmenter: u32 },
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub decode: DecodeConfig,
    pub segment: SegmentConfig,
    pub policy: FilterPolicy,
    pub shard: ShardConfig,
    /// Ruleset used to normalize transcripts. `None` leaves them untouched.
    pub rules_path: Option<PathBuf>,
}

impl PipelineConfig {
    pub fn new(staging_dir: impl Into<PathBuf>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            decode: DecodeConfig::default(),
            segment: SegmentConfig::default(),
            policy: FilterPolicy::default(),
            shard: ShardConfig::new(staging_dir, output_dir),
            rules_path: None,
        }
    }

    pub fn with_rules(mut self, path: impl Into<PathBuf>) -> Self {
        self.rules_path = Some(path.into());
        self
    }

    pub fn with_policy(mut self, policy: FilterPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Decode to, and segment at, `sample_rate`.
    pub fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        self.decode.target_sample_rate = sample_rate;
        self.segment.sample_rate = sample_rate;
        self
    }
}

/// What happened to one segment.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentOutcome {
    pub segment: Segment,
    pub metrics: QualityMetrics,
    /// `None` if the segment was accepted and written.
    pub rejected: Option<RejectReason>,
}

impl SegmentOutcome {
    pub fn accepted(&self) -> bool {
        self.rejected.is_none()
    }
}

/// What happened to one input.
#[derive(Debug, Clone)]
pub struct FileOutcome {
    pub path: PathBuf,
    pub duration_s: f32,
    pub source_sample_rate: u32,
    pub applied_gain_db: f32,
    pub segments: Vec<SegmentOutcome>,
}

impl FileOutcome {
    pub fn accepted_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|outcome| outcome.accepted())
            .count()
    }
}

/// Corpus-level totals.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PipelineStats {
    pub files_processed: u64,
    pub files_failed: u64,
    pub segments_found: u64,
    pub segments_accepted: u64,
    pub rejections: BTreeMap<RejectReason, u64>,
    /// Total duration of every segment the segmenter found.
    pub speech_duration_s: f64,
    pub accepted_duration_s: f64,
    pub shard: ShardStats,
}

impl PipelineStats {
    pub fn segments_rejected(&self) -> u64 {
        self.segments_found - self.segments_accepted
    }

    /// Fraction of detected speech, by duration, that passed the filter.
    ///
    /// Weighted by duration because one 30 s clip is worth more training data
    /// than ten 2 s ones.
    pub fn retention_rate(&self) -> f64 {
        if self.speech_duration_s <= 0.0 {
            return 0.0;
        }
        self.accepted_duration_s / self.speech_duration_s
    }

    /// Fraction of segments, by count, that passed the filter.
    pub fn segment_acceptance_rate(&self) -> f64 {
        if self.segments_found == 0 {
            return 0.0;
        }
        self.segments_accepted as f64 / self.segments_found as f64
    }

    fn record(&mut self, duration_s: f32, rejected: Option<RejectReason>) {
        self.segments_found += 1;
        self.speech_duration_s += f64::from(duration_s);
        match rejected {
            Some(reason) => *self.rejections.entry(reason).or_default() += 1,
            None => {
                self.segments_accepted += 1;
                self.accepted_duration_s += f64::from(duration_s);
            }
        }
    }
}

/// The curation pipeline, generic over how speech is detected.
pub struct Pipeline<D: SpeechDetector = EnergyDetector> {
    config: PipelineConfig,
    segmenter: Segmenter<D>,
    rules: Option<RuleEngine>,
    writer: ShardWriter,
    stats: PipelineStats,
}

impl Pipeline {
    /// Build a pipeline that detects speech by signal level.
    pub fn new(config: PipelineConfig) -> Result<Self, PipelineError> {
        let detector = EnergyDetector::new(config.segment.sample_rate);
        Self::with_detector(config, detector)
    }
}

impl<D: SpeechDetector> Pipeline<D> {
    /// Build a pipeline around `detector`, which must expect audio at
    /// `config.segment.sample_rate`.
    ///
    /// Fails if the ruleset cannot be loaded: silently skipping normalization
    /// would corrupt every transcript in the corpus.
    pub fn with_detector(config: PipelineConfig, detector: D) -> Result<Self, PipelineError> {
        ensure_matching_rates(config.decode.target_sample_rate, &config.segment)?;
        let rules = config
            .rules_path
            .as_deref()
            .map(RuleEngine::from_yaml_file)
            .transpose()?;
        let segmenter = Segmenter::new(detector, config.segment.clone());
        let writer = ShardWriter::new(config.shard.clone())?;

        Ok(Self {
            config,
            segmenter,
            rules,
            writer,
            stats: PipelineStats::default(),
        })
    }

    /// Decode a file and curate it. See [`Pipeline::process_audio`].
    pub fn process_file(
        &mut self,
        path: &Path,
        transcript: Option<&str>,
    ) -> Result<FileOutcome, PipelineError> {
        let audio = aerosieve_decode::decode_file(path, &self.config.decode)
            .inspect_err(|_| self.stats.files_failed += 1)?;
        self.process_audio(path, &audio, transcript)
    }

    /// Segment, score, and shard audio that is already decoded.
    ///
    /// `transcript` describes the whole input and is attached, normalized, to
    /// every accepted segment; it is not aligned to individual segments.
    /// `source` is recorded as each sample's provenance.
    pub fn process_audio(
        &mut self,
        source: &Path,
        audio: &DecodedAudio,
        transcript: Option<&str>,
    ) -> Result<FileOutcome, PipelineError> {
        let outcome = self.curate(source, audio, transcript);
        let counter = match outcome {
            Ok(_) => &mut self.stats.files_processed,
            Err(_) => &mut self.stats.files_failed,
        };
        *counter += 1;
        outcome
    }

    pub fn stats(&self) -> &PipelineStats {
        &self.stats
    }

    /// Commit the final shard and return corpus totals.
    pub fn finish(self) -> Result<PipelineStats, PipelineError> {
        let shard = self.writer.finish()?;
        Ok(PipelineStats {
            shard,
            ..self.stats
        })
    }

    fn curate(
        &mut self,
        source: &Path,
        audio: &DecodedAudio,
        transcript: Option<&str>,
    ) -> Result<FileOutcome, PipelineError> {
        ensure_matching_rates(audio.sample_rate, &self.config.segment)?;
        let transcript = self.normalize(transcript.unwrap_or_default());
        let source_file = source.to_string_lossy();

        let segments = self.segmenter.segment(&audio.samples)?;
        let mut outcomes = Vec::with_capacity(segments.len());
        for segment in segments {
            let clip = &audio.samples[segment.start_sample..segment.end_sample];
            let duration_s = segment.duration_s(audio.sample_rate);
            let metrics = aerosieve_acoustic::score(clip, audio.sample_rate);
            let rejected = self.config.policy.evaluate(&metrics);

            if rejected.is_none() {
                let mut meta =
                    SampleMeta::new(duration_s, audio.sample_rate).with_metrics(&metrics)?;
                meta.transcript = transcript.clone();
                meta.source_file = Some(source_file.to_string());
                self.writer.write_sample(clip, meta)?;
            }
            self.stats.record(duration_s, rejected);
            outcomes.push(SegmentOutcome {
                segment,
                metrics,
                rejected,
            });
        }

        Ok(FileOutcome {
            path: source.to_path_buf(),
            duration_s: audio.duration_s(),
            source_sample_rate: audio.source_sample_rate,
            applied_gain_db: audio.applied_gain_db,
            segments: outcomes,
        })
    }

    fn normalize(&self, transcript: &str) -> String {
        match &self.rules {
            Some(rules) => rules.normalize(transcript).text,
            None => transcript.to_string(),
        }
    }
}

fn ensure_matching_rates(audio: u32, segment: &SegmentConfig) -> Result<(), PipelineError> {
    if audio == segment.sample_rate {
        return Ok(());
    }
    Err(PipelineError::SampleRateMismatch {
        audio,
        segmenter: segment.sample_rate,
    })
}
