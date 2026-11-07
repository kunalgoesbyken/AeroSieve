use std::path::{Path, PathBuf};

use aerosieve::DecodeConfig;
use aerosieve::decode::{DecodeError, decode_file};

pub const SAMPLE_RATE: u32 = 16_000;
pub const REFERENCE_DBFS: f32 = -20.0;
const MIN_UTTERANCE_S: f32 = 3.0;
const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "flac", "mp3", "ogg", "m4a"];

/// One clean recording, at [`SAMPLE_RATE`] and [`REFERENCE_DBFS`].
pub struct Utterance {
    pub name: String,
    pub samples: Vec<f32>,
}

/// Every audio file directly inside `dir` long enough to yield a clip,
/// sorted by name so runs are reproducible.
pub fn load(dir: &Path, limit: Option<usize>) -> Result<Vec<Utterance>, DecodeError> {
    let limit = limit.unwrap_or(usize::MAX);
    let config = DecodeConfig {
        normalize: false,
        ..DecodeConfig::default()
    };
    let mut utterances = Vec::new();
    for path in audio_files(dir) {
        if utterances.len() == limit {
            break;
        }
        let audio = decode_file(&path, &config)?;
        if audio.duration_s() < MIN_UTTERANCE_S {
            continue;
        }
        utterances.push(Utterance {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            samples: at_level(audio.samples, REFERENCE_DBFS),
        });
    }
    Ok(utterances)
}

fn audio_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        })
        .collect();
    paths.sort();
    paths
}

pub fn mean_power(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / samples.len() as f64
}

pub fn at_level(samples: Vec<f32>, dbfs: f32) -> Vec<f32> {
    let power = mean_power(&samples);
    if power == 0.0 {
        return samples;
    }
    let gain = (10f64.powf(f64::from(dbfs) / 20.0) / power.sqrt()) as f32;
    samples.into_iter().map(|s| s * gain).collect()
}
