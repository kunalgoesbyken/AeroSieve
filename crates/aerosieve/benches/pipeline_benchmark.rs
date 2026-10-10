//! End-to-end and per-stage cost. Run with
//! `cargo bench --bench pipeline_benchmark`.
//!
//! `cargo run --release --example report` prints what the pipeline keeps and
//! rejects on the same corpus.

use std::hint::black_box;
use std::path::PathBuf;

use aerosieve::lexical::RuleEngine;
use aerosieve::segment::Segmenter;
use aerosieve::shard::{SampleMeta, ShardWriter};
use aerosieve::{
    DecodedAudio, EnergyDetector, Pipeline, PipelineConfig, SegmentConfig, ShardConfig,
};
use aerosieve_fixtures::{SAMPLE_RATE, glottal_speech, mixed_corpus, samples};
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use tempfile::TempDir;

const TRANSCRIPT: &str = "yeh \u{20B9}500 hai aur 5 laakh rupaye";
const CLIP_SECONDS: f32 = 5.0;

fn rules_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../aerosieve-lexical/rules/hinglish.yaml")
}

fn shard_config(dir: &TempDir) -> ShardConfig {
    ShardConfig::new(dir.path().join("staging"), dir.path().join("clean"))
}

fn bench_end_to_end(c: &mut Criterion) {
    let corpus = DecodedAudio::from_samples(mixed_corpus(samples(600.0)), SAMPLE_RATE);

    let mut group = c.benchmark_group("end_to_end");
    group.sample_size(10);
    group.throughput(Throughput::Elements(corpus.samples.len() as u64));
    group.bench_function("curate_10_minutes", |b| {
        b.iter_batched(
            || {
                let dir = TempDir::new().unwrap();
                let config =
                    PipelineConfig::new(dir.path().join("staging"), dir.path().join("clean"))
                        .with_rules(rules_path());
                (Pipeline::new(config).unwrap(), dir)
            },
            |(mut pipeline, dir)| {
                pipeline
                    .process_audio("corpus.wav".as_ref(), &corpus, Some(TRANSCRIPT))
                    .unwrap();
                (pipeline.finish().unwrap(), dir)
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

fn bench_stages(c: &mut Criterion) {
    let minute = mixed_corpus(samples(60.0));
    let clip = glottal_speech(samples(CLIP_SECONDS));
    let rules = RuleEngine::from_yaml_file(&rules_path()).unwrap();

    let mut group = c.benchmark_group("stage");
    group.bench_function("segment_one_minute", |b| {
        let mut segmenter =
            Segmenter::new(EnergyDetector::new(SAMPLE_RATE), SegmentConfig::default());
        b.iter(|| segmenter.segment(black_box(&minute)).unwrap());
    });
    group.bench_function("score_clip", |b| {
        b.iter(|| aerosieve::acoustic::score(black_box(&clip), SAMPLE_RATE))
    });
    group.bench_function("normalize_transcript", |b| {
        b.iter(|| rules.normalize(black_box(TRANSCRIPT)))
    });
    group.bench_function("shard_clip", |b| {
        b.iter_batched(
            || {
                let dir = TempDir::new().unwrap();
                (ShardWriter::new(shard_config(&dir)).unwrap(), dir)
            },
            |(mut writer, dir)| {
                let meta = SampleMeta::new(CLIP_SECONDS, SAMPLE_RATE);
                writer.write_sample(&clip, meta).unwrap();
                (writer.finish().unwrap(), dir)
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_end_to_end, bench_stages);
criterion_main!(benches);
