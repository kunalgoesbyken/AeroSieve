//! AeroSieve — a batch audio curation pipeline.
//!
//! Raw audio files in, model-ready [WebDataset] shards out. The stages follow
//! [Emilia-Pipe] (arXiv 2407.05361), the reference pipeline for turning
//! in-the-wild audio into speech-generation training data:
//!
//! 1. **Decode** — [`aerosieve_decode`]: any supported container to 16 kHz mono
//!    f32, loudness-normalized to −20 dBFS.
//! 2. **Segment** — [`aerosieve_segment`]: a hysteresis VAD state machine cuts
//!    the audio into variable-length speech clips.
//! 3. **Score** — [`aerosieve_acoustic`]: WADA-SNR, clipping, level, and
//!    spectral flatness per clip, filtered by an explicit policy.
//! 4. **Shard** — [`aerosieve_shard`]: accepted clips are written into tar
//!    shards with a manifest.
//!
//! Rejection is the product. Emilia reports ~29.4% retention on in-the-wild
//! audio, so [`PipelineStats::retention_rate`] is the headline metric, not
//! throughput.
//!
//! # Example
//!
//! ```no_run
//! use aerosieve::{Pipeline, PipelineConfig};
//!
//! # fn main() -> Result<(), aerosieve::PipelineError> {
//! let config = PipelineConfig::new("data/staging", "data/clean");
//! let mut pipeline = Pipeline::new(config)?;
//!
//! pipeline.process_file("recording.wav".as_ref(), Some("hello world"))?;
//!
//! let stats = pipeline.finish()?;
//! println!("retention {:.1}%", stats.retention_rate() * 100.0);
//! # Ok(())
//! # }
//! ```
//!
//! [WebDataset]: https://github.com/webdataset/webdataset
//! [Emilia-Pipe]: https://arxiv.org/abs/2407.05361

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use aerosieve_acoustic as acoustic;
pub use aerosieve_decode as decode;
pub use aerosieve_lexical as lexical;
pub use aerosieve_segment as segment;
pub use aerosieve_shard as shard;

use aerosieve_acoustic::{FilterPolicy, QualityMetrics, RejectReason};
use aerosieve_decode::{DecodeConfig, DecodedAudio};
use aerosieve_lexical::RuleEngine;
use aerosieve_segment::{EnergyDetector, Segment, SegmentConfig, Segmenter};
use aerosieve_shard::{SampleMeta, ShardConfig, ShardStats, ShardWriter};

/// Anything that can go wrong running the pipeline.
///
/// Note what is *absent*: there is no variant for "rules failed to load, so we
/// carried on with no rules". v1 did exactly that behind an `eprintln!`, which
/// meant a typo in a path silently disabled normalization for an entire corpus.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("decode failed: {0}")]
    Decode(#[from] aerosieve_decode::DecodeError),
    #[error("shard write failed: {0}")]
    Shard(#[from] aerosieve_shard::ShardError),
    #[error("rule engine failed: {0}")]
    Lexical(#[from] aerosieve_lexical::LexicalError),
}

/// How the pipeline is configured end to end.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub decode: DecodeConfig,
    pub segment: SegmentConfig,
    pub policy: FilterPolicy,
    pub shard: ShardConfig,
    /// Path to a normalization ruleset. `None` passes transcripts through
    /// untouched — an explicit choice, never a silent fallback.
    pub rules_path: Option<PathBuf>,
    /// Energy detector threshold, in dBFS.
    pub energy_threshold_dbfs: f32,
}

impl PipelineConfig {
    /// Defaults for everything, with the two directories you must choose.
    pub fn new(staging: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            decode: DecodeConfig::default(),
            segment: SegmentConfig::default(),
            policy: FilterPolicy::default(),
            shard: ShardConfig::new(staging, output),
            rules_path: None,
            energy_threshold_dbfs: aerosieve_segment::DEFAULT_ENERGY_THRESHOLD_DBFS,
        }
    }

    /// Load and apply normalization rules from `path`.
    pub fn with_rules(mut self, path: impl Into<PathBuf>) -> Self {
        self.rules_path = Some(path.into());
        self
    }

    /// Override the acceptance policy.
    pub fn with_policy(mut self, policy: FilterPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// What happened to one segment.
#[derive(Debug, Clone)]
pub struct SegmentOutcome {
    pub segment: Segment,
    pub metrics: QualityMetrics,
    /// `None` means accepted.
    pub rejected: Option<RejectReason>,
}

impl SegmentOutcome {
    pub fn accepted(&self) -> bool {
        self.rejected.is_none()
    }
}

/// What happened to one input file.
#[derive(Debug, Clone)]
pub struct FileOutcome {
    pub path: PathBuf,
    pub source_sample_rate: u32,
    pub source_duration_s: f32,
    pub applied_gain_db: f32,
    pub segments: Vec<SegmentOutcome>,
}

impl FileOutcome {
    pub fn accepted_count(&self) -> usize {
        self.segments.iter().filter(|s| s.accepted()).count()
    }
}

/// Corpus-level totals.
///
/// `retention_rate` is the number to quote. Emilia-Pipe reports 29.4% on
/// in-the-wild audio; a curation pipeline that keeps everything is not
/// filtering.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PipelineStats {
    pub files_processed: u64,
    pub files_failed: u64,
    pub segments_found: u64,
    pub segments_accepted: u64,
    pub segments_rejected: u64,
    /// Rejections by reason, keyed by the `RejectReason` name.
    pub rejections: BTreeMap<String, u64>,
    pub input_duration_s: f64,
    pub accepted_duration_s: f64,
    pub shard: ShardStats,
}

impl PipelineStats {
    /// Fraction of discovered speech duration that survived filtering.
    ///
    /// Duration-weighted rather than a segment count: one 30 s clip surviving
    /// matters more than ten 3 s clips being dropped.
    pub fn retention_rate(&self) -> f64 {
        if self.input_duration_s <= 0.0 {
            return 0.0;
        }
        self.accepted_duration_s / self.input_duration_s
    }

    /// Fraction of segments accepted, by count.
    pub fn segment_acceptance_rate(&self) -> f64 {
        if self.segments_found == 0 {
            return 0.0;
        }
        self.segments_accepted as f64 / self.segments_found as f64
    }
}

/// The curation pipeline.
pub struct Pipeline {
    config: PipelineConfig,
    segmenter: Segmenter<EnergyDetector>,
    engine: Option<RuleEngine>,
    writer: ShardWriter,
    stats: PipelineStats,
}

impl Pipeline {
    /// Build a pipeline.
    ///
    /// Fails loudly if the ruleset cannot be loaded. A pipeline that silently
    /// stops normalizing is worse than one that refuses to start.
    pub fn new(config: PipelineConfig) -> Result<Self, PipelineError> {
        let engine = match config.rules_path.as_deref() {
            Some(path) => Some(RuleEngine::from_yaml_file(path)?),
            None => None,
        };

        let detector = EnergyDetector::with_threshold_dbfs(
            config.segment.sample_rate,
            config.energy_threshold_dbfs,
        );
        let segmenter = Segmenter::new(detector, config.segment.clone());
        let writer = ShardWriter::new(config.shard.clone())?;

        Ok(Self {
            config,
            segmenter,
            engine,
            writer,
            stats: PipelineStats::default(),
        })
    }

    /// Decode, segment, score, and shard one file.
    ///
    /// `transcript` is attached to every accepted segment from this file.
    /// AeroSieve does not generate transcripts — see the README on ASR being
    /// out of scope.
    pub fn process_file(
        &mut self,
        path: &Path,
        transcript: Option<&str>,
    ) -> Result<FileOutcome, PipelineError> {
        let audio = match aerosieve_decode::decode_file(path, &self.config.decode) {
            Ok(audio) => audio,
            Err(e) => {
                self.stats.files_failed += 1;
                return Err(e.into());
            }
        };
        self.stats.files_processed += 1;
        self.process_audio(path, audio, transcript)
    }

    /// Same as [`Pipeline::process_file`] for audio already in memory.
    pub fn process_audio(
        &mut self,
        source: &Path,
        audio: DecodedAudio,
        transcript: Option<&str>,
    ) -> Result<FileOutcome, PipelineError> {
        let sample_rate = audio.sample_rate;
        let normalized = transcript.map(|t| self.normalize(t)).unwrap_or_default();
        let source_name = source.to_string_lossy().into_owned();

        let segments = self.segmenter.segment(&audio.samples);
        let mut outcomes = Vec::with_capacity(segments.len());

        for segment in segments {
            let clip = &audio.samples[segment.start_sample..segment.end_sample];
            let metrics = aerosieve_acoustic::score(clip);
            let rejected = self.config.policy.evaluate(&metrics);
            let duration_s = segment.duration_s(sample_rate);

            self.stats.segments_found += 1;
            self.stats.input_duration_s += duration_s as f64;

            match rejected {
                Some(reason) => {
                    self.stats.segments_rejected += 1;
                    *self
                        .stats
                        .rejections
                        .entry(format!("{reason:?}"))
                        .or_insert(0) += 1;
                }
                None => {
                    let key = self.writer.next_key();
                    let mut meta = SampleMeta::new(key, duration_s, sample_rate);
                    meta.transcript = normalized.clone();
                    meta.source_file = Some(source_name.clone());
                    meta.metrics = serde_json::to_value(metrics)
                        .unwrap_or(serde_json::Value::Null);

                    self.writer.write_sample(clip, &meta)?;
                    self.stats.segments_accepted += 1;
                    self.stats.accepted_duration_s += duration_s as f64;
                }
            }

            outcomes.push(SegmentOutcome {
                segment,
                metrics,
                rejected,
            });
        }

        Ok(FileOutcome {
            path: source.to_path_buf(),
            source_sample_rate: audio.source_sample_rate,
            source_duration_s: audio.duration_s,
            applied_gain_db: audio.applied_gain_db,
            segments: outcomes,
        })
    }

    fn normalize(&self, text: &str) -> String {
        match &self.engine {
            Some(engine) => engine.normalize(text).normalized,
            None => text.to_string(),
        }
    }

    /// Stats so far.
    pub fn stats(&self) -> &PipelineStats {
        &self.stats
    }

    /// Commit the final shard and return corpus totals.
    pub fn finish(mut self) -> Result<PipelineStats, PipelineError> {
        let shard = self.writer.finish()?;
        self.stats.shard = shard;
        Ok(self.stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("aerosieve-pipe-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config(base: &Path) -> PipelineConfig {
        let mut cfg = PipelineConfig::new(base.join("staging"), base.join("clean"));
        // Fixtures are short; the 3 s floor would drop everything.
        cfg.segment.min_segment_s = 0.1;
        cfg
    }

    /// Speech-like audio with realistic *amplitude statistics*, not just a
    /// realistic spectrum.
    ///
    /// This matters more than it looks. WADA-SNR models clean speech as
    /// Gamma(0.4) — sparse and impulsive — so a sum of pure harmonics reads as
    /// −20 dB "noise" despite being mathematically noiseless, and its spectral
    /// flatness of 8e-5 is correctly classified `Tonal`. A glottal pulse train
    /// plus a little aspiration noise reproduces both properties real speech
    /// has: peaky in time, broadband in frequency.
    fn speech(len: usize) -> Vec<f32> {
        let sr = 16_000.0f32;
        let period = (sr / 120.0) as usize; // 120 Hz f0
        let mut state = 0x5DEECE66Du64;
        let mut noise = move || {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            ((state.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64
                / (1u64 << 53) as f64) as f32
                * 2.0
                - 1.0
        };
        (0..len)
            .map(|i| {
                let t = i as f32 / sr;
                let env = 0.35 + 0.65 * (std::f32::consts::TAU * 3.5 * t).sin().abs();
                let ph = (i % period) as f32 / period as f32;
                let pulse = (-6.0 * ph).exp() * (std::f32::consts::TAU * ph).sin();
                (pulse * 0.8 + noise() * 0.04) * env * 0.8
            })
            .collect()
    }

    fn audio_of(samples: Vec<f32>) -> DecodedAudio {
        let duration_s = samples.len() as f32 / 16_000.0;
        DecodedAudio {
            samples,
            sample_rate: 16_000,
            source_sample_rate: 16_000,
            source_channels: 1,
            applied_gain_db: 0.0,
            duration_s,
        }
    }

    #[test]
    fn speech_is_accepted_and_written() {
        let base = temp_dir("accept");
        let mut p = Pipeline::new(config(&base)).unwrap();

        let mut samples = vec![0.0f32; 4_000];
        samples.extend(speech(24_000));
        samples.extend(vec![0.0f32; 4_000]);

        let out = p
            .process_audio(Path::new("test.wav"), audio_of(samples), Some("hello"))
            .unwrap();
        assert!(out.accepted_count() > 0, "expected speech to be accepted");

        let stats = p.finish().unwrap();
        assert!(stats.segments_accepted > 0);
        assert!(stats.retention_rate() > 0.0);
        assert_eq!(stats.shard.samples_written, stats.segments_accepted);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn silence_produces_no_segments() {
        let base = temp_dir("silence");
        let mut p = Pipeline::new(config(&base)).unwrap();

        let out = p
            .process_audio(Path::new("s.wav"), audio_of(vec![0.0; 32_000]), None)
            .unwrap();
        assert_eq!(out.segments.len(), 0);

        let stats = p.finish().unwrap();
        assert_eq!(stats.segments_found, 0);
        assert_eq!(stats.shard.samples_written, 0);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_rules_file_is_an_error_not_a_silent_fallback() {
        // v1 printed a warning and continued with an empty engine, so a typo
        // in a path silently disabled normalization for a whole corpus.
        let base = temp_dir("rules");
        let cfg = config(&base).with_rules(base.join("does-not-exist.yaml"));
        assert!(matches!(
            Pipeline::new(cfg),
            Err(PipelineError::Lexical(_))
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn transcripts_are_normalized_when_rules_are_loaded() {
        let base = temp_dir("norm");
        let mut rules = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        rules.push("../aerosieve-lexical/rules/default.yaml");

        let p = Pipeline::new(config(&base).with_rules(rules)).unwrap();
        assert_eq!(p.normalize("yeh \u{20B9}500 hai"), "yeh 500 rupaye hai");

        let _ = p.finish();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn transcripts_pass_through_without_rules() {
        let base = temp_dir("passthru");
        let p = Pipeline::new(config(&base)).unwrap();
        assert_eq!(p.normalize("yeh \u{20B9}500 hai"), "yeh \u{20B9}500 hai");
        let _ = p.finish();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rejections_are_recorded_by_reason() {
        let base = temp_dir("reject");
        let mut cfg = config(&base);
        // Force every segment to fail on level.
        cfg.policy.min_dbfs = 6.0;
        let mut p = Pipeline::new(cfg).unwrap();

        let mut samples = vec![0.0f32; 4_000];
        samples.extend(speech(24_000));
        samples.extend(vec![0.0f32; 4_000]);
        p.process_audio(Path::new("r.wav"), audio_of(samples), None)
            .unwrap();

        let stats = p.finish().unwrap();
        assert!(stats.segments_rejected > 0);
        assert!(!stats.rejections.is_empty());
        assert_eq!(stats.segments_accepted, 0);
        assert_eq!(stats.retention_rate(), 0.0);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn empty_corpus_finishes_cleanly() {
        let base = temp_dir("empty");
        let p = Pipeline::new(config(&base)).unwrap();
        let stats = p.finish().unwrap();
        assert_eq!(stats.files_processed, 0);
        assert_eq!(stats.retention_rate(), 0.0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_input_file_is_reported_and_counted() {
        let base = temp_dir("missing");
        let mut p = Pipeline::new(config(&base)).unwrap();
        let err = p.process_file(Path::new("nope.wav"), None);
        assert!(err.is_err());
        assert_eq!(p.stats().files_failed, 1);
        let _ = p.finish();
        let _ = std::fs::remove_dir_all(&base);
    }
}
