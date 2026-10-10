//! Curate one synthetic recording and print what survived.
//!
//! ```text
//! cargo run --release --example basic
//! ```

use std::path::PathBuf;

use aerosieve::{DecodedAudio, Pipeline, PipelineConfig};
use aerosieve_fixtures::{SAMPLE_RATE, glottal_speech, samples, silence};

const OUTPUT_DIR: &str = "data/clean";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rules =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../aerosieve-lexical/rules/hinglish.yaml");
    let config = PipelineConfig::new("data/staging", OUTPUT_DIR).with_rules(rules);
    let mut pipeline = Pipeline::new(config)?;

    let recording = [
        silence(samples(0.25)),
        glottal_speech(samples(6.0)),
        silence(samples(0.25)),
    ]
    .concat();
    let outcome = pipeline.process_audio(
        "synthetic.wav".as_ref(),
        &DecodedAudio::from_samples(recording, SAMPLE_RATE),
        Some("yeh \u{20B9}500 hai aur 5 laakh rupaye"),
    )?;

    println!("Segments found: {}", outcome.segments.len());
    for (index, outcome) in outcome.segments.iter().enumerate() {
        let verdict = match outcome.rejected {
            None => "accepted".to_string(),
            Some(reason) => format!("rejected ({reason})"),
        };
        println!(
            "  [{index}] {:.2}s  snr {:>6.1} dB  level {:>6.1} dBFS  flatness {:.3}  -> {verdict}",
            outcome.segment.duration_s(SAMPLE_RATE),
            outcome.metrics.wada_snr_db,
            outcome.metrics.rms_dbfs,
            outcome.metrics.spectral_flatness,
        );
    }

    let stats = pipeline.finish()?;
    println!(
        "\nAccepted {}/{} segments, retention {:.1}% of speech duration",
        stats.segments_accepted,
        stats.segments_found,
        stats.retention_rate() * 100.0
    );
    println!(
        "Wrote {} sample(s) across {} shard(s) to {OUTPUT_DIR}",
        stats.shard.samples_written, stats.shard.shards_written
    );
    Ok(())
}
