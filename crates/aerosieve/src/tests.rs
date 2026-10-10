use aerosieve_fixtures::{SAMPLE_RATE, glottal_speech, samples, silence};
use aerosieve_lexical::LexicalError;
use aerosieve_shard::SampleRecord;
use tempfile::TempDir;

use super::*;

const TRANSCRIPT: &str = "yeh  \u{20B9}500 hai";

struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    fn output(&self) -> PathBuf {
        self.root.path().join("clean")
    }

    fn config(&self) -> PipelineConfig {
        PipelineConfig::new(self.root.path().join("staging"), self.output())
    }

    fn manifest(&self) -> Vec<SampleRecord> {
        std::fs::read_to_string(self.output().join("manifest.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn hinglish_rules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../aerosieve-lexical/rules/hinglish.yaml")
}

fn recording() -> DecodedAudio {
    let samples = [
        silence(samples(0.25)),
        glottal_speech(samples(4.0)),
        silence(samples(0.25)),
    ]
    .concat();
    DecodedAudio::from_samples(samples, SAMPLE_RATE)
}

fn curate(config: PipelineConfig, audio: &DecodedAudio) -> (FileOutcome, PipelineStats) {
    let mut pipeline = Pipeline::new(config).unwrap();
    let outcome = pipeline
        .process_audio(Path::new("take.wav"), audio, Some(TRANSCRIPT))
        .unwrap();
    (outcome, pipeline.finish().unwrap())
}

#[test]
fn clean_speech_is_accepted_and_written_with_its_provenance() {
    let workspace = Workspace::new();

    let (outcome, stats) = curate(workspace.config(), &recording());

    assert_eq!(outcome.accepted_count(), 1);
    assert_eq!(stats.files_processed, 1);
    assert_eq!(stats.segments_accepted, 1);
    assert_eq!(stats.segments_rejected(), 0);
    assert_eq!(stats.retention_rate(), 1.0);
    assert_eq!(stats.shard.samples_written, 1);

    let manifest = workspace.manifest();
    assert_eq!(manifest.len(), 1);
    assert_eq!(manifest[0].meta.source_file.as_deref(), Some("take.wav"));
    assert_eq!(manifest[0].meta.sample_rate, SAMPLE_RATE);
    assert!(manifest[0].meta.metrics.contains_key("wada_snr_db"));
}

#[test]
fn transcripts_are_normalized_only_when_rules_are_configured() {
    let plain = Workspace::new();
    curate(plain.config(), &recording());
    assert_eq!(plain.manifest()[0].meta.transcript, TRANSCRIPT);

    let normalized = Workspace::new();
    curate(
        normalized.config().with_rules(hinglish_rules()),
        &recording(),
    );
    assert_eq!(
        normalized.manifest()[0].meta.transcript,
        "yeh 500 rupaye hai"
    );
}

#[test]
fn rejections_are_counted_by_reason() {
    let workspace = Workspace::new();
    let unreachable_level = FilterPolicy {
        min_dbfs: 6.0,
        ..FilterPolicy::default()
    };

    let (outcome, stats) = curate(
        workspace.config().with_policy(unreachable_level),
        &recording(),
    );

    assert_eq!(outcome.segments[0].rejected, Some(RejectReason::TooQuiet));
    assert_eq!(stats.rejections, [(RejectReason::TooQuiet, 1)].into());
    assert_eq!(stats.segments_rejected(), 1);
    assert_eq!(stats.retention_rate(), 0.0);
    assert!(workspace.manifest().is_empty());
}

#[test]
fn silence_yields_no_segments() {
    let workspace = Workspace::new();
    let quiet = DecodedAudio::from_samples(silence(samples(2.0)), SAMPLE_RATE);

    let (outcome, stats) = curate(workspace.config(), &quiet);

    assert!(outcome.segments.is_empty());
    assert_eq!(stats.segments_found, 0);
    assert_eq!(stats.retention_rate(), 0.0);
    assert_eq!(stats.segment_acceptance_rate(), 0.0);
}

/// Nearest-neighbour conversion of 16 kHz audio to `sample_rate`.
fn at_sample_rate(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let ratio = f64::from(SAMPLE_RATE) / f64::from(sample_rate);
    let len = (samples.len() as f64 / ratio) as usize;
    (0..len)
        .map(|index| samples[(index as f64 * ratio) as usize])
        .collect()
}

#[test]
fn files_at_any_source_rate_are_resampled_and_accepted() {
    for source_rate in [8_000u32, 32_000, 44_100, 48_000] {
        let workspace = Workspace::new();
        let path = workspace.root.path().join("take.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: source_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut wav = hound::WavWriter::create(&path, spec).unwrap();
        for sample in at_sample_rate(&recording().samples, source_rate) {
            wav.write_sample(sample).unwrap();
        }
        wav.finalize().unwrap();

        let mut pipeline = Pipeline::new(workspace.config()).unwrap();
        let outcome = pipeline.process_file(&path, None).unwrap();
        pipeline.finish().unwrap();

        assert_eq!(outcome.source_sample_rate, source_rate);
        assert!((outcome.duration_s - recording().duration_s()).abs() < 0.01);
        assert_eq!(
            outcome
                .segments
                .iter()
                .map(|s| s.rejected)
                .collect::<Vec<_>>(),
            [None],
            "{source_rate} Hz"
        );
        assert_eq!(workspace.manifest()[0].meta.sample_rate, SAMPLE_RATE);
    }
}

#[test]
fn an_undecodable_file_is_counted_as_failed() {
    let workspace = Workspace::new();
    let mut pipeline = Pipeline::new(workspace.config()).unwrap();

    let result = pipeline.process_file(Path::new("absent.wav"), None);

    assert!(matches!(result, Err(PipelineError::Decode(_))));
    assert_eq!(pipeline.stats().files_failed, 1);
    assert_eq!(pipeline.stats().files_processed, 0);
}

#[test]
fn audio_at_the_wrong_sample_rate_is_refused() {
    let workspace = Workspace::new();
    let mut pipeline = Pipeline::new(workspace.config()).unwrap();
    let audio = DecodedAudio::from_samples(silence(8_000), 8_000);

    let result = pipeline.process_audio(Path::new("narrowband.wav"), &audio, None);

    assert!(matches!(
        result,
        Err(PipelineError::SampleRateMismatch {
            audio: 8_000,
            segmenter: SAMPLE_RATE
        })
    ));
    assert_eq!(pipeline.stats().files_failed, 1);
}

#[test]
fn a_config_whose_stages_disagree_on_sample_rate_is_refused() {
    let workspace = Workspace::new();
    let mut config = workspace.config();
    config.decode.target_sample_rate = 8_000;

    assert!(matches!(
        Pipeline::new(config),
        Err(PipelineError::SampleRateMismatch { .. })
    ));
    assert!(Pipeline::new(workspace.config().with_sample_rate(8_000)).is_ok());
}

#[test]
fn a_missing_rules_file_fails_construction() {
    let workspace = Workspace::new();
    let config = workspace.config().with_rules("does-not-exist.yaml");

    assert!(matches!(
        Pipeline::new(config),
        Err(PipelineError::Lexical(LexicalError::Io(_)))
    ));
}

#[test]
fn stats_serialize_rejections_by_identifier() {
    let mut stats = PipelineStats::default();
    stats.record(1.0, Some(RejectReason::LowSnr));

    let json = serde_json::to_value(&stats).unwrap();

    assert_eq!(json["rejections"]["low_snr"], 1);
    assert_eq!(
        serde_json::from_value::<PipelineStats>(json).unwrap(),
        stats
    );
}
