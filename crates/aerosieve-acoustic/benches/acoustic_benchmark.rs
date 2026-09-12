//! Throughput benchmarks for `aerosieve-acoustic`.
//!
//! All fixtures are generated with a deterministic xorshift PRNG so runs are
//! comparable across machines and invocations. The headline buffer is 16,000
//! samples: one second of 16 kHz mono audio, the canonical unit for this
//! pipeline.
//!
//! Run with `cargo bench -p aerosieve-acoustic`.

use std::f32::consts::PI;

use aerosieve_acoustic::{clip_ratio, rms_dbfs, score, spectral_flatness, wada_snr_db};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

const SR: f32 = 16_000.0;
/// One second of 16 kHz mono audio.
const ONE_SECOND: usize = 16_000;

/// Deterministic xorshift64* PRNG — same generator the unit tests use, so the
/// benchmark and the tests exercise identical fixtures.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn uniform(&mut self) -> f32 {
        (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32
    }

    fn normal(&mut self) -> f32 {
        let u1 = self.uniform().max(1e-9);
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

/// Harmonic voiced excitation with a syllabic envelope and breath noise.
/// Representative of what the scorer actually sees in the pipeline.
fn speech_like(n: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let f0 = 130.0 + 12.0 * (2.0 * PI * 1.7 * t).sin();
        let phase = 2.0 * PI * f0 * t;
        let voiced = phase.sin()
            + 0.55 * (2.0 * phase).sin()
            + 0.32 * (3.0 * phase).sin()
            + 0.20 * (4.0 * phase).sin()
            + 0.12 * (5.0 * phase).sin();
        let env = {
            let e = 0.5 - 0.5 * (2.0 * PI * 4.0 * t).cos();
            e * e
        };
        out.push(0.18 * env * voiced + 0.01 * rng.normal() * env.max(0.05));
    }
    out
}

/// `score()` on one second of audio — the composite the pipeline calls.
fn bench_score(c: &mut Criterion) {
    let buf = speech_like(ONE_SECOND, 0xACE);
    let mut group = c.benchmark_group("score");
    group.throughput(Throughput::Elements(ONE_SECOND as u64));
    group.bench_function("1s_16khz", |b| {
        b.iter(|| score(black_box(&buf)));
    });
    group.finish();
}

/// WADA-SNR alone: a single pass computing two accumulators plus a `ln` per
/// non-zero sample, then one binary search.
fn bench_wada_snr(c: &mut Criterion) {
    let buf = speech_like(ONE_SECOND, 0xACE);
    let mut group = c.benchmark_group("wada_snr");
    group.throughput(Throughput::Elements(ONE_SECOND as u64));
    group.bench_function("1s_16khz", |b| {
        b.iter(|| wada_snr_db(black_box(&buf)));
    });
    group.finish();
}

/// Spectral flatness alone: the FFT path, and the dominant cost in `score()`.
fn bench_spectral_flatness(c: &mut Criterion) {
    let buf = speech_like(ONE_SECOND, 0xACE);
    let mut group = c.benchmark_group("spectral_flatness");
    group.throughput(Throughput::Elements(ONE_SECOND as u64));
    group.bench_function("1s_16khz", |b| {
        b.iter(|| spectral_flatness(black_box(&buf)));
    });
    group.finish();
}

/// The two cheap linear-scan metrics, for comparison against the two above.
fn bench_level_metrics(c: &mut Criterion) {
    let buf = speech_like(ONE_SECOND, 0xACE);
    let mut group = c.benchmark_group("level_metrics");
    group.throughput(Throughput::Elements(ONE_SECOND as u64));
    group.bench_function("rms_dbfs", |b| b.iter(|| rms_dbfs(black_box(&buf))));
    group.bench_function("clip_ratio", |b| b.iter(|| clip_ratio(black_box(&buf))));
    group.finish();
}

/// How `score()` scales with segment length. The pipeline emits 3-30 s
/// segments, so these are the sizes that matter in production.
fn bench_score_by_duration(c: &mut Criterion) {
    let mut group = c.benchmark_group("score_by_duration");
    for secs in [1usize, 3, 10, 30] {
        let buf = speech_like(ONE_SECOND * secs, 0xACE);
        group.throughput(Throughput::Elements(buf.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(format!("{secs}s")), &buf, |b, buf| {
            b.iter(|| score(black_box(buf)));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_score,
    bench_wada_snr,
    bench_spectral_flatness,
    bench_level_metrics,
    bench_score_by_duration,
);
criterion_main!(benches);
