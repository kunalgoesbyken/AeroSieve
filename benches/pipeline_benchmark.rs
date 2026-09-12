//! AeroSieve benchmark suite.
//!
//! The headline unit is **hours of audio curated per minute of wall clock**,
//! matching how Emilia-Pipe reports throughput (2.5 raw hours/min on 8x RTX
//! 4090) so the numbers are directly comparable. Per-stage microbenchmarks sit
//! underneath to show where the time goes.
//!
//! Every number printed here is produced by this file. If the README quotes a
//! figure, `cargo bench --bench pipeline_benchmark` must reproduce it.

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use aerosieve::{Pipeline, PipelineConfig};
use aerosieve_acoustic::{score, spectral_flatness, wada_snr_db, FilterPolicy};
use aerosieve_decode::DecodedAudio;
use aerosieve_lexical::RuleEngine;
use aerosieve_segment::{EnergyDetector, SegmentConfig, Segmenter};
use aerosieve_shard::{SampleMeta, ShardConfig, ShardWriter};

const SAMPLE_RATE: u32 = 16_000;
/// One minute of audio. Long enough for per-file overhead to be negligible.
const CLIP_SAMPLES: usize = SAMPLE_RATE as usize * 60;

fn rules_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../aerosieve-lexical/rules/default.yaml"
    ))
}

fn bench_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aerosieve-bench-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Speech-like audio: a glottal pulse train plus aspiration noise.
///
/// Amplitude statistics matter as much as the spectrum here. WADA-SNR assumes
/// clean speech is Gamma(0.4) distributed, so benchmarking on pure harmonics
/// would measure the scorer on input it is designed to reject and the
/// segmenter on a signal no VAD would ever see.
fn speech(len: usize) -> Vec<f32> {
    let period = (SAMPLE_RATE as f32 / 120.0) as usize;
    let mut state = 0x5DEECE66Du64;
    let mut noise = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        ((state.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64)
            as f32
            * 2.0
            - 1.0
    };
    (0..len)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = 0.35 + 0.65 * (std::f32::consts::TAU * 3.5 * t).sin().abs();
            let ph = (i % period) as f32 / period as f32;
            let pulse = (-6.0 * ph).exp() * (std::f32::consts::TAU * ph).sin();
            (pulse * 0.8 + noise() * 0.04) * env * 0.8
        })
        .collect()
}

/// A deliberately mixed corpus: clean speech, plus the degradations a real
/// in-the-wild scrape is full of.
///
/// Benchmarking on uniformly clean audio would report 100% retention, which
/// measures nothing -- the point of a curation pipeline is what it throws away.
/// Emilia-Pipe keeps ~29.4% of in-the-wild audio.
fn recording(total_samples: usize) -> Vec<f32> {
    let run = SAMPLE_RATE as usize * 6;
    let gap = SAMPLE_RATE as usize / 2;
    let mut state = 0xC0FFEEu64;
    let mut rnd = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        ((state.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64)
            as f32
            * 2.0
            - 1.0
    };

    let mut out = Vec::with_capacity(total_samples);
    let mut kind = 0usize;
    while out.len() < total_samples {
        out.extend(std::iter::repeat_n(0.0, gap.min(total_samples - out.len())));
        if out.len() >= total_samples {
            break;
        }
        let take = run.min(total_samples - out.len());
        let block: Vec<f32> = match kind % 5 {
            // Two of five runs are usable speech.
            0 | 1 => speech(take),
            // Driven into the rails.
            2 => speech(take).iter().map(|x| (x * 6.0).clamp(-1.0, 1.0)).collect(),
            // A sustained tone: hold music, a test signal, a hum.
            3 => (0..take).map(|i| (i as f32 * 0.1).sin() * 0.5).collect(),
            // Broadband noise: traffic, crowd, a bad mic.
            _ => (0..take).map(|_| rnd() * 0.2).collect(),
        };
        out.extend(block);
        kind += 1;
    }
    out
}

fn audio_of(samples: Vec<f32>) -> DecodedAudio {
    DecodedAudio {
        duration_s: samples.len() as f32 / SAMPLE_RATE as f32,
        samples,
        sample_rate: SAMPLE_RATE,
        source_sample_rate: SAMPLE_RATE,
        source_channels: 1,
        applied_gain_db: 0.0,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 1. End-to-end curation throughput, and what it keeps
// ─────────────────────────────────────────────────────────────────────────
fn bench_end_to_end(c: &mut Criterion) {
    let minutes = 10;
    let audio: Vec<f32> = recording(CLIP_SAMPLES * minutes);
    let audio_hours = audio.len() as f64 / SAMPLE_RATE as f64 / 3600.0;

    let base = bench_dir("e2e");
    let config = PipelineConfig::new(base.join("staging"), base.join("clean"))
        .with_rules(rules_path());
    let mut pipeline = Pipeline::new(config).expect("pipeline");

    let t0 = Instant::now();
    pipeline
        .process_audio(
            "bench.wav".as_ref(),
            audio_of(audio.clone()),
            Some("yeh \u{20B9}500 hai aur 5 laakh rupaye"),
        )
        .expect("process");
    let stats = pipeline.finish().expect("finish");
    let elapsed = t0.elapsed();

    let hours_per_min = audio_hours / elapsed.as_secs_f64() * 60.0;

    println!("\n  1. End-to-end curation ({minutes} min of audio, file sink)");
    println!("  ──────────────────────────────────────────────────────────────");
    println!("  {:<28} {:>12.2}", "audio in (hours)", audio_hours);
    println!("  {:<28} {:>12.2?}", "wall clock", elapsed);
    println!("  {:<28} {:>12.1}", "HOURS PER MINUTE", hours_per_min);
    println!("  {:<28} {:>12}", "segments found", stats.segments_found);
    println!("  {:<28} {:>12}", "segments accepted", stats.segments_accepted);
    println!(
        "  {:<28} {:>11.1}%",
        "RETENTION (by duration)",
        stats.retention_rate() * 100.0
    );
    if !stats.rejections.is_empty() {
        println!("  rejections by reason:");
        for (reason, n) in &stats.rejections {
            println!("    {reason:<24} {n:>12}");
        }
    }
    println!("  ──────────────────────────────────────────────────────────────");
    let _ = std::fs::remove_dir_all(&base);

    let mut g = c.benchmark_group("end_to_end");
    g.sample_size(10);
    g.throughput(Throughput::Elements(audio.len() as u64));
    g.bench_function("curate_10min", |b| {
        b.iter_batched(
            || {
                let dir = std::env::temp_dir()
                    .join(format!("aerosieve-bench-iter-{}", uuid_like()));
                let cfg = PipelineConfig::new(dir.join("staging"), dir.join("clean"))
                    .with_rules(rules_path());
                (dir, Pipeline::new(cfg).expect("pipeline"))
            },
            |(dir, mut p)| {
                p.process_audio("b.wav".as_ref(), audio_of(audio.clone()), Some("hi"))
                    .expect("process");
                let _ = p.finish();
                let _ = std::fs::remove_dir_all(&dir);
            },
            criterion::BatchSize::PerIteration,
        );
    });
    g.finish();
}

fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────
// 2. Per-stage cost
// ─────────────────────────────────────────────────────────────────────────
fn bench_stages(c: &mut Criterion) {
    let one_minute = recording(CLIP_SAMPLES);
    let clip = speech(SAMPLE_RATE as usize * 5);

    let mut g = c.benchmark_group("segment");
    g.throughput(Throughput::Elements(one_minute.len() as u64));
    g.bench_function("one_minute", |b| {
        let mut seg = Segmenter::new(
            EnergyDetector::new(SAMPLE_RATE),
            SegmentConfig::default(),
        );
        b.iter(|| black_box(seg.segment(black_box(&one_minute)).len()));
    });
    g.finish();

    let mut g = c.benchmark_group("score");
    g.throughput(Throughput::Elements(clip.len() as u64));
    g.bench_function("five_second_clip", |b| {
        b.iter(|| black_box(score(black_box(&clip))))
    });
    g.bench_function("wada_snr", |b| {
        b.iter(|| black_box(wada_snr_db(black_box(&clip))))
    });
    g.bench_function("spectral_flatness", |b| {
        b.iter(|| black_box(spectral_flatness(black_box(&clip))))
    });
    g.finish();

    let engine = RuleEngine::from_yaml_file(&rules_path()).expect("rules");
    let sentence = "yeh \u{20B9}500 hai aur 5 laakh rupaye";
    let mut g = c.benchmark_group("normalize");
    g.bench_function("hinglish_sentence", |b| {
        b.iter(|| black_box(engine.normalize(black_box(sentence))))
    });
    g.bench_function("no_match", |b| {
        b.iter(|| black_box(engine.normalize(black_box("the quick brown fox jumps"))))
    });
    g.finish();

    let mut g = c.benchmark_group("shard");
    g.throughput(Throughput::Elements(1));
    g.bench_function("write_one_clip", |b| {
        b.iter_batched(
            || {
                let dir = std::env::temp_dir()
                    .join(format!("aerosieve-bench-shard-{}", uuid_like()));
                let w = ShardWriter::new(ShardConfig::new(
                    dir.join("staging"),
                    dir.join("clean"),
                ))
                .expect("writer");
                (dir, w)
            },
            |(dir, mut w)| {
                let mut meta = SampleMeta::new("k", 5.0, SAMPLE_RATE);
                w.write_sample(&clip, &mut meta).expect("write");
                let _ = w.finish();
                let _ = std::fs::remove_dir_all(&dir);
            },
            criterion::BatchSize::PerIteration,
        );
    });
    g.finish();
}

// ─────────────────────────────────────────────────────────────────────────
// 3. Filtering behaviour: what the policy actually rejects
// ─────────────────────────────────────────────────────────────────────────
fn report_filter_behaviour(_c: &mut Criterion) {
    let policy = FilterPolicy::default();
    let len = SAMPLE_RATE as usize * 5;

    let mut noise_state = 0x1234_5678u64;
    let mut rnd = move || {
        noise_state ^= noise_state >> 12;
        noise_state ^= noise_state << 25;
        noise_state ^= noise_state >> 27;
        ((noise_state.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64
            / (1u64 << 53) as f64) as f32
            * 2.0
            - 1.0
    };

    let cases: Vec<(&str, Vec<f32>)> = vec![
        ("clean speech", speech(len)),
        ("digital silence", vec![0.0; len]),
        (
            "quiet speech (-45 dBFS)",
            speech(len).iter().map(|x| x * 0.02).collect(),
        ),
        (
            "clipped speech",
            speech(len).iter().map(|x| (x * 6.0).clamp(-1.0, 1.0)).collect(),
        ),
        (
            "pure tone",
            (0..len)
                .map(|i| (i as f32 * 0.1).sin() * 0.5)
                .collect(),
        ),
        ("white noise", (0..len).map(|_| rnd() * 0.2).collect()),
    ];

    println!("\n  2. Filter behaviour (FilterPolicy::default)");
    println!("  ──────────────────────────────────────────────────────────────");
    println!(
        "  {:<24} {:>8} {:>8} {:>9} {:>12}",
        "INPUT", "SNR dB", "dBFS", "FLATNESS", "VERDICT"
    );
    for (name, samples) in cases {
        let m = score(&samples);
        let verdict = match policy.evaluate(&m) {
            None => "accept".to_string(),
            Some(r) => format!("{}", r.as_str()),
        };
        println!(
            "  {:<24} {:>8.1} {:>8.1} {:>9.4} {:>12}",
            name, m.wada_snr_db, m.rms_dbfs, m.spectral_flatness, verdict
        );
    }
    println!("  ──────────────────────────────────────────────────────────────");
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3))
        .sample_size(20);
    targets = bench_end_to_end, bench_stages, report_filter_behaviour,
);
criterion_main!(benches);
