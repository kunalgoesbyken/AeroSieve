//! Per-metric cost on speech-like audio. Run with
//! `cargo bench -p aerosieve-acoustic`.

use std::hint::black_box;

use aerosieve_acoustic::{clip_ratio, rms_dbfs, score, spectral_flatness, wada_snr_db};
use aerosieve_fixtures::{SAMPLE_RATE, harmonic_speech, samples};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

const SEED: u64 = 0xACE;

fn bench_metrics(c: &mut Criterion) {
    let clip = harmonic_speech(samples(1.0), SEED);
    let mut group = c.benchmark_group("one_second");
    group.throughput(Throughput::Elements(clip.len() as u64));
    group.bench_function("score", |b| b.iter(|| score(black_box(&clip), SAMPLE_RATE)));
    group.bench_function("wada_snr", |b| b.iter(|| wada_snr_db(black_box(&clip))));
    group.bench_function("spectral_flatness", |b| {
        b.iter(|| spectral_flatness(black_box(&clip), SAMPLE_RATE))
    });
    group.bench_function("rms_dbfs", |b| b.iter(|| rms_dbfs(black_box(&clip))));
    group.bench_function("clip_ratio", |b| b.iter(|| clip_ratio(black_box(&clip))));
    group.finish();
}

fn bench_score_by_duration(c: &mut Criterion) {
    let mut group = c.benchmark_group("score_by_duration");
    for seconds in [1u8, 3, 10, 30] {
        let clip = harmonic_speech(samples(f32::from(seconds)), SEED);
        group.throughput(Throughput::Elements(clip.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(seconds), &clip, |b, clip| {
            b.iter(|| score(black_box(clip), SAMPLE_RATE));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_metrics, bench_score_by_duration);
criterion_main!(benches);
