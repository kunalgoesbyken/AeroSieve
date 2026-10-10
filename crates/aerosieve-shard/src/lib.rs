//! [WebDataset] tar shard writer.
//!
//! Each sample is stored as two adjacent tar members sharing a key,
//! `{key}.wav` then `{key}.json`, so a streaming loader can group them
//! without seeking. Shards roll over at a target size.
//!
//! A shard is built in a scratch directory under `staging_dir` and renamed
//! into `output_dir` only once complete, so readers of `output_dir` never see
//! a partial tar. `staging_dir` and `output_dir` must be on the same volume.
//!
//! [WebDataset]: https://github.com/webdataset/webdataset

#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{BufWriter, Cursor, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

pub const DEFAULT_TARGET_SHARD_BYTES: u64 = 1_000_000_000;

const MANIFEST_FILE: &str = "manifest.jsonl";
const STATS_FILE: &str = "stats.json";
const TAR_BLOCK_BYTES: u64 = 512;

/// What is known about a sample besides its audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleMeta {
    pub duration_s: f32,
    pub sample_rate: u32,
    pub transcript: String,
    pub source_file: Option<String>,
    /// Caller-defined measurements, written alongside the fields above.
    #[serde(flatten)]
    pub metrics: serde_json::Map<String, serde_json::Value>,
}

impl SampleMeta {
    pub fn new(duration_s: f32, sample_rate: u32) -> Self {
        Self {
            duration_s,
            sample_rate,
            transcript: String::new(),
            source_file: None,
            metrics: serde_json::Map::new(),
        }
    }

    /// Replace `metrics` with the fields of `metrics`, which must serialize
    /// to a JSON object.
    pub fn with_metrics(mut self, metrics: &impl Serialize) -> Result<Self, ShardError> {
        self.metrics = serde_json::from_value(serde_json::to_value(metrics)?)?;
        Ok(self)
    }
}

/// A written sample: the `{key}.json` tar member, and one line of the
/// manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleRecord {
    pub key: String,
    pub shard: String,
    #[serde(flatten)]
    pub meta: SampleMeta,
}

#[derive(Debug, Clone)]
pub struct ShardConfig {
    pub staging_dir: PathBuf,
    pub output_dir: PathBuf,
    /// A shard is closed once it reaches this size.
    pub target_shard_bytes: u64,
}

impl ShardConfig {
    pub fn new(staging_dir: impl Into<PathBuf>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            staging_dir: staging_dir.into(),
            output_dir: output_dir.into(),
            target_shard_bytes: DEFAULT_TARGET_SHARD_BYTES,
        }
    }

    pub fn with_target_shard_bytes(mut self, bytes: u64) -> Self {
        self.target_shard_bytes = bytes;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShardError {
    #[error("shard io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("wav encoding error: {0}")]
    Wav(#[from] hound::Error),
    #[error("metadata serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sample metadata declares a sample rate of zero")]
    ZeroSampleRate,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShardStats {
    pub samples_written: u64,
    pub shards_written: u64,
    pub total_bytes: u64,
    pub total_duration_s: f64,
}

struct OpenShard {
    name: String,
    staging_path: PathBuf,
    archive: tar::Builder<BufWriter<File>>,
    bytes: u64,
}

impl OpenShard {
    fn create(staging_dir: &Path, index: u64) -> Result<Self, ShardError> {
        let name = format!("shard-{index:06}.tar");
        let staging_path = staging_dir.join(&name);
        let archive = tar::Builder::new(BufWriter::new(File::create(&staging_path)?));
        Ok(Self {
            name,
            staging_path,
            archive,
            bytes: 0,
        })
    }

    fn append(&mut self, member_name: &str, data: &[u8]) -> Result<(), ShardError> {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_entry_type(tar::EntryType::Regular);
        self.archive.append_data(&mut header, member_name, data)?;

        let payload_blocks = (data.len() as u64).div_ceil(TAR_BLOCK_BYTES);
        self.bytes += (1 + payload_blocks) * TAR_BLOCK_BYTES;
        Ok(())
    }

    /// Finish the archive and flush it to disk, returning where it was staged.
    fn seal(self) -> Result<PathBuf, ShardError> {
        sync_and_close(self.archive.into_inner()?)?;
        Ok(self.staging_path)
    }
}

/// Writes samples into tar shards, plus `manifest.jsonl` and `stats.json`.
///
/// Nothing but completed shards reaches `output_dir` until [`finish`] is
/// called. A writer dropped without it leaves no partial files behind.
///
/// Shards and the manifest are named deterministically, so writing into an
/// `output_dir` that already holds a run replaces that run's files.
///
/// [`finish`]: ShardWriter::finish
pub struct ShardWriter {
    config: ShardConfig,
    stats: ShardStats,
    open: Option<OpenShard>,
    manifest: BufWriter<File>,
    /// Declared last so the files above are closed before it is removed.
    scratch: TempDir,
}

impl ShardWriter {
    pub fn new(config: ShardConfig) -> Result<Self, ShardError> {
        std::fs::create_dir_all(&config.staging_dir)?;
        std::fs::create_dir_all(&config.output_dir)?;
        let scratch = TempDir::new_in(&config.staging_dir)?;
        let manifest = BufWriter::new(File::create(scratch.path().join(MANIFEST_FILE))?);
        Ok(Self {
            config,
            stats: ShardStats::default(),
            open: None,
            manifest,
            scratch,
        })
    }

    /// Totals so far. `shards_written` and `total_bytes` count only shards
    /// already committed to `output_dir`.
    pub fn stats(&self) -> &ShardStats {
        &self.stats
    }

    /// Write one sample as 16-bit PCM and return the key it was assigned.
    ///
    /// Keys are sequential and zero-padded, so they sort in write order.
    pub fn write_sample(
        &mut self,
        samples: &[f32],
        meta: SampleMeta,
    ) -> Result<String, ShardError> {
        if meta.sample_rate == 0 {
            return Err(ShardError::ZeroSampleRate);
        }
        let wav = encode_wav_i16(samples, meta.sample_rate)?;
        if self
            .open
            .as_ref()
            .is_some_and(|shard| shard.bytes >= self.config.target_shard_bytes)
        {
            self.commit_open_shard()?;
        }
        let shard = match &mut self.open {
            Some(shard) => shard,
            vacant => vacant.insert(OpenShard::create(
                self.scratch.path(),
                self.stats.shards_written,
            )?),
        };

        let record = SampleRecord {
            key: format!("{:09}", self.stats.samples_written),
            shard: shard.name.clone(),
            meta,
        };
        shard.append(&format!("{}.wav", record.key), &wav)?;
        shard.append(
            &format!("{}.json", record.key),
            &serde_json::to_vec_pretty(&record)?,
        )?;

        serde_json::to_writer(&mut self.manifest, &record)?;
        self.manifest.write_all(b"\n")?;

        self.stats.samples_written += 1;
        self.stats.total_duration_s += f64::from(record.meta.duration_s);
        Ok(record.key)
    }

    /// Commit the last shard, then publish the manifest and stats.
    pub fn finish(mut self) -> Result<ShardStats, ShardError> {
        self.commit_open_shard()?;

        let Self {
            config,
            stats,
            manifest,
            scratch,
            ..
        } = self;
        sync_and_close(manifest)?;
        std::fs::rename(
            scratch.path().join(MANIFEST_FILE),
            config.output_dir.join(MANIFEST_FILE),
        )?;
        std::fs::write(
            config.output_dir.join(STATS_FILE),
            serde_json::to_vec_pretty(&stats)?,
        )?;
        Ok(stats)
    }

    fn commit_open_shard(&mut self) -> Result<(), ShardError> {
        let Some(shard) = self.open.take() else {
            return Ok(());
        };
        let destination = self.config.output_dir.join(&shard.name);
        std::fs::rename(shard.seal()?, &destination)?;

        self.stats.total_bytes += std::fs::metadata(&destination)?.len();
        self.stats.shards_written += 1;
        Ok(())
    }
}

fn sync_and_close(writer: BufWriter<File>) -> std::io::Result<()> {
    writer
        .into_inner()
        .map_err(|error| error.into_error())?
        .sync_all()
}

/// Encode as mono 16-bit PCM. Out-of-range samples saturate rather than wrap,
/// and NaN becomes silence.
fn encode_wav_i16(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, ShardError> {
    const HEADER_BYTES: usize = 44;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut bytes = Cursor::new(Vec::with_capacity(HEADER_BYTES + samples.len() * 2));
    let mut writer = hound::WavWriter::new(&mut bytes, spec)?;
    for &sample in samples {
        let scaled = sample.clamp(-1.0, 1.0) * f32::from(i16::MAX);
        writer.write_sample(scaled.round() as i16)?;
    }
    writer.finalize()?;
    Ok(bytes.into_inner())
}
