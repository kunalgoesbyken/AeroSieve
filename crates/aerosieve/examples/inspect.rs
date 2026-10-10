//! Show how the default filter judges your own recordings, without writing
//! any output. Use it to check the thresholds before a real run.
//!
//! ```text
//! cargo run --release --example inspect -- recording.wav another.mp3
//! ```

use std::path::Path;

use aerosieve::decode::decode_file;
use aerosieve::segment::Segmenter;
use aerosieve::{DecodeConfig, EnergyDetector, FilterPolicy, SegmentConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: inspect <audio file>...".into());
    }

    let (mut found, mut accepted) = (0, 0);
    for path in &paths {
        match inspect(Path::new(path)) {
            Ok((file_found, file_accepted)) => {
                found += file_found;
                accepted += file_accepted;
            }
            Err(error) => println!(
                "
{path}: skipped ({error})"
            ),
        }
    }
    println!("\n{accepted} of {found} clips accepted");
    Ok(())
}

/// Print one line per clip. Returns how many clips were found and accepted.
fn inspect(path: &Path) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let audio = decode_file(path, &DecodeConfig::default())?;
    let detector = EnergyDetector::new(audio.sample_rate);
    let config = SegmentConfig {
        sample_rate: audio.sample_rate,
        ..SegmentConfig::default()
    };
    let segments = Segmenter::new(detector, config).segment(&audio.samples)?;
    let policy = FilterPolicy::default();

    println!(
        "\n{} ({:.1} s, {} Hz source)",
        path.display(),
        audio.duration_s(),
        audio.source_sample_rate
    );
    println!(
        "  {:>7} {:>7} {:>8} {:>8} {:>6} {:>9}  VERDICT",
        "START", "LENGTH", "SNR dB", "dBFS", "CLIP", "FLATNESS"
    );

    let mut accepted = 0;
    for segment in &segments {
        let clip = &audio.samples[segment.start_sample..segment.end_sample];
        let metrics = aerosieve::acoustic::score(clip, audio.sample_rate);
        let verdict = policy.evaluate(&metrics);
        accepted += usize::from(verdict.is_none());

        println!(
            "  {:>6.1}s {:>6.1}s {:>8.1} {:>8.1} {:>6.3} {:>9.3}  {}",
            segment.start_sample as f32 / audio.sample_rate as f32,
            segment.duration_s(audio.sample_rate),
            metrics.wada_snr_db,
            metrics.rms_dbfs,
            metrics.clip_ratio,
            metrics.spectral_flatness,
            verdict.map_or("accept", |reason| reason.as_str()),
        );
    }
    Ok((segments.len(), accepted))
}
