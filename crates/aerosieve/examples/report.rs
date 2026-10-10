//! Print what the default filter keeps and rejects, and how fast.
//!
//! ```text
//! cargo run --release --example report
//! ```

use std::time::Instant;

use aerosieve::acoustic::score;
use aerosieve::{DecodedAudio, FilterPolicy, Pipeline, PipelineConfig};
use aerosieve_fixtures::{
    SAMPLE_RATE, glottal_speech, mixed_corpus, overdrive, samples, silence, tone, white_noise,
};
use tempfile::TempDir;

const CORPUS_MINUTES: f32 = 10.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    print_filter_verdicts();
    print_corpus_run()
}

fn print_filter_verdicts() {
    let len = samples(5.0);
    let speech = glottal_speech(len);
    let quiet: Vec<f32> = speech.iter().map(|sample| sample * 0.02).collect();
    let cases = [
        ("clean speech", speech.clone()),
        ("digital silence", silence(len)),
        ("quiet speech", quiet),
        ("clipped speech", overdrive(&speech, 6.0)),
        ("pure tone", tone(SAMPLE_RATE, 255.0, 0.5, len)),
        ("white noise", white_noise(len, 0.1, 0x1234_5678)),
    ];

    let policy = FilterPolicy::default();
    println!(
        "{:<18} {:>8} {:>8} {:>9}  VERDICT",
        "INPUT", "SNR dB", "dBFS", "FLATNESS"
    );
    for (name, audio) in cases {
        let metrics = score(&audio, SAMPLE_RATE);
        let verdict = policy
            .evaluate(&metrics)
            .map_or("accept", |reason| reason.as_str());
        println!(
            "{name:<18} {:>8.1} {:>8.1} {:>9.3}  {verdict}",
            metrics.wada_snr_db, metrics.rms_dbfs, metrics.spectral_flatness
        );
    }
}

fn print_corpus_run() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let config = PipelineConfig::new(dir.path().join("staging"), dir.path().join("clean"));
    let mut pipeline = Pipeline::new(config)?;
    let corpus =
        DecodedAudio::from_samples(mixed_corpus(samples(CORPUS_MINUTES * 60.0)), SAMPLE_RATE);

    let started = Instant::now();
    pipeline.process_audio("corpus.wav".as_ref(), &corpus, None)?;
    let stats = pipeline.finish()?;
    let elapsed = started.elapsed();

    let audio_hours = f64::from(corpus.duration_s()) / 3600.0;
    let hours_per_minute = audio_hours / elapsed.as_secs_f64() * 60.0;

    println!("\n{CORPUS_MINUTES} minute mixed corpus, curated in {elapsed:.0?}");
    println!("  audio hours per wall-clock minute  {hours_per_minute:.1}");
    println!(
        "  segments found                     {}",
        stats.segments_found
    );
    println!(
        "  segments accepted                  {}",
        stats.segments_accepted
    );
    println!(
        "  retention of detected speech       {:.1}%",
        stats.retention_rate() * 100.0
    );
    for (reason, count) in &stats.rejections {
        println!("  rejected: {reason:<24} {count}");
    }
    Ok(())
}
