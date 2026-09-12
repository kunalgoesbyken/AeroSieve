//! End-to-end walkthrough: generate a short recording, run it through the
//! pipeline, and print what survived.
//!
//! Run with:
//! ```text
//! cargo run --example basic
//! ```

use std::path::PathBuf;

use aerosieve::{Pipeline, PipelineConfig};

const SAMPLE_RATE: u32 = 16_000;

/// A synthetic recording: silence, then six seconds of speech, then silence.
///
/// Six seconds because `SegmentConfig` defaults to a 3 s minimum clip length,
/// following Emilia-Pipe's 3-30 s training-clip window.
///
/// The speech is a glottal pulse train plus a little aspiration noise. That
/// combination matters — WADA-SNR models clean speech as a sparse, impulsive
/// (Gamma-distributed) signal, so a sum of pure sine harmonics would be scored
/// as noise and rejected as tonal, however clean it looks on a scope.
fn recording() -> Vec<f32> {
    let period = (SAMPLE_RATE as f32 / 120.0) as usize; // 120 Hz pitch
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

    let mut out = vec![0.0f32; SAMPLE_RATE as usize / 4];
    for i in 0..SAMPLE_RATE as usize * 6 {
        let t = i as f32 / SAMPLE_RATE as f32;
        let envelope = 0.35 + 0.65 * (std::f32::consts::TAU * 3.5 * t).sin().abs();
        let phase = (i % period) as f32 / period as f32;
        let pulse = (-6.0 * phase).exp() * (std::f32::consts::TAU * phase).sin();
        out.push((pulse * 0.8 + noise() * 0.04) * envelope * 0.8);
    }
    out.extend(std::iter::repeat_n(0.0, SAMPLE_RATE as usize / 4));
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rules = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    rules.push("../aerosieve-lexical/rules/default.yaml");

    let config = PipelineConfig::new("data/staging", "data/clean").with_rules(rules);
    let mut pipeline = Pipeline::new(config)?;

    let audio = aerosieve::decode::DecodedAudio {
        duration_s: recording().len() as f32 / SAMPLE_RATE as f32,
        samples: recording(),
        sample_rate: SAMPLE_RATE,
        source_sample_rate: SAMPLE_RATE,
        source_channels: 1,
        applied_gain_db: 0.0,
    };

    let outcome = pipeline.process_audio(
        "synthetic.wav".as_ref(),
        audio,
        Some("yeh \u{20B9}500 hai aur 5 laakh rupaye"),
    )?;

    println!("Segments found: {}", outcome.segments.len());
    for (i, seg) in outcome.segments.iter().enumerate() {
        let verdict = match seg.rejected {
            None => "accepted".to_string(),
            Some(reason) => format!("rejected ({})", reason.as_str()),
        };
        println!(
            "  [{i}] {:.2}s  snr {:>6.1} dB  level {:>6.1} dBFS  flatness {:.3}  -> {verdict}",
            seg.segment.duration_s(SAMPLE_RATE),
            seg.metrics.wada_snr_db,
            seg.metrics.rms_dbfs,
            seg.metrics.spectral_flatness,
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
        "Wrote {} sample(s) across {} shard(s) to data/clean",
        stats.shard.samples_written, stats.shard.shards_written
    );

    Ok(())
}
