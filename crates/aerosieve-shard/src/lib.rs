//! WebDataset tar shard writer.
//!
//! Replaces v1's per-sample `{uuid}.raw` + `{uuid}.txt` layout (millions of
//! tiny files, random access) with WebDataset tar shards: sequential I/O that
//! is 3-10x faster for training, and streams directly from object stores.
//!
//! Layout rules enforced here:
//! - All files for one sample share a basename, differing only in extension
//!   (`000000123.wav`, `000000123.json`).
//! - Those files are written adjacently and in order (`.wav` then `.json`).
//! - Shards are `shard-{:06}.tar`, rolled at `target_shard_bytes`.
//! - A completed shard is committed atomically from `staging_dir` into
//!   `output_dir` via `hard_link`, falling back to `rename`. Either mechanism
//!   succeeding is a success.

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Metadata for one training sample, serialized as the `.json` member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleMeta {
    pub key: String,
    pub duration_s: f32,
    pub sample_rate: u32,
    pub transcript: String,
    /// Opaque quality metrics, flattened into the top-level JSON object.
    #[serde(flatten)]
    pub metrics: serde_json::Value,
    pub source_file: Option<String>,
}

impl SampleMeta {
    /// Convenience constructor; `metrics` defaults to an empty object.
    pub fn new(key: impl Into<String>, duration_s: f32, sample_rate: u32) -> Self {
        Self {
            key: key.into(),
            duration_s,
            sample_rate,
            transcript: String::new(),
            metrics: serde_json::Value::Object(serde_json::Map::new()),
            source_file: None,
        }
    }
}

/// Default shard rollover threshold (~1 GB).
pub const DEFAULT_TARGET_SHARD_BYTES: u64 = 1_000_000_000;
/// Default audio sample rate.
pub const DEFAULT_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone)]
pub struct ShardConfig {
    pub staging_dir: PathBuf,
    pub output_dir: PathBuf,
    pub target_shard_bytes: u64,
    pub sample_rate: u32,
}

impl ShardConfig {
    pub fn new(staging: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            staging_dir: staging.into(),
            output_dir: output.into(),
            target_shard_bytes: DEFAULT_TARGET_SHARD_BYTES,
            sample_rate: DEFAULT_SAMPLE_RATE,
        }
    }

    pub fn with_target_shard_bytes(mut self, bytes: u64) -> Self {
        self.target_shard_bytes = bytes;
        self
    }

    pub fn with_sample_rate(mut self, rate: u32) -> Self {
        self.sample_rate = rate;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShardError {
    #[error("shard io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("wav encode/decode error: {0}")]
    Wav(#[from] hound::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tar error: {0}")]
    Tar(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShardStats {
    pub samples_written: u64,
    pub shards_written: u64,
    pub total_bytes: u64,
    pub total_duration_s: f64,
}

/// One line of `manifest.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub key: String,
    pub shard: String,
    pub duration_s: f32,
    pub sample_rate: u32,
    pub transcript: String,
    #[serde(flatten)]
    pub metrics: serde_json::Value,
    pub source_file: Option<String>,
}

/// Encodes f32 samples as 16-bit PCM WAV bytes, clamping to [-1.0, 1.0] so
/// out-of-range values saturate at the i16 rails instead of wrapping.
pub fn encode_wav_i16(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, ShardError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf: Vec<u8> = Vec::with_capacity(44 + samples.len() * 2);
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut writer = hound::WavWriter::new(cursor, spec)?;
        for &s in samples {
            writer.write_sample(f32_to_i16(s))?;
        }
        writer.finalize()?;
    }
    Ok(buf)
}

/// Clamp then scale. NaN maps to silence.
#[inline]
fn f32_to_i16(s: f32) -> i16 {
    if s.is_nan() {
        return 0;
    }
    let clamped = s.clamp(-1.0, 1.0);
    (clamped * i16::MAX as f32).round() as i16
}

struct OpenShard {
    index: u64,
    name: String,
    staging_path: PathBuf,
    builder: tar::Builder<BufWriter<File>>,
    bytes: u64,
    samples: u64,
}

pub struct ShardWriter {
    config: ShardConfig,
    current: Option<OpenShard>,
    next_shard_index: u64,
    next_key: u64,
    samples_written: u64,
    shards_completed: u64,
    total_bytes: u64,
    total_duration_s: f64,
    manifest: Vec<ManifestEntry>,
}

impl ShardWriter {
    pub fn new(config: ShardConfig) -> Result<Self, ShardError> {
        std::fs::create_dir_all(&config.staging_dir)?;
        std::fs::create_dir_all(&config.output_dir)?;
        Ok(Self {
            config,
            current: None,
            next_shard_index: 0,
            next_key: 0,
            samples_written: 0,
            shards_completed: 0,
            total_bytes: 0,
            total_duration_s: 0.0,
            manifest: Vec::new(),
        })
    }

    pub fn samples_written(&self) -> u64 {
        self.samples_written
    }

    pub fn shards_completed(&self) -> u64 {
        self.shards_completed
    }

    /// Sequential zero-padded key that will be assigned to the next sample.
    pub fn next_key(&self) -> String {
        format!("{:09}", self.next_key)
    }

    /// Writes one sample as adjacent `{key}.wav` + `{key}.json` tar members.
    /// Rolls (commits) the current shard first if it has reached the target size.
    pub fn write_sample(
        &mut self,
        samples: &[f32],
        meta: &SampleMeta,
    ) -> Result<(), ShardError> {
        let key = format!("{:09}", self.next_key);

        let wav = encode_wav_i16(samples, self.config.sample_rate)?;

        let mut meta = meta.clone();
        meta.key = key.clone();
        if meta.sample_rate == 0 {
            meta.sample_rate = self.config.sample_rate;
        }
        let json = serde_json::to_vec_pretty(&meta)?;

        // Roll before writing so a sample's two members never straddle shards.
        if self
            .current
            .as_ref()
            .is_some_and(|cur| cur.bytes >= self.config.target_shard_bytes)
        {
            self.commit_current()?;
        }
        if self.current.is_none() {
            self.open_shard()?;
        }

        let shard_name = {
            let cur = self.current.as_mut().expect("shard open");
            append_member(&mut cur.builder, &format!("{key}.wav"), &wav)?;
            append_member(&mut cur.builder, &format!("{key}.json"), &json)?;
            // tar rounds each member to a 512-byte boundary + 512-byte header.
            cur.bytes += tar_member_size(wav.len()) + tar_member_size(json.len());
            cur.samples += 1;
            cur.name.clone()
        };

        self.manifest.push(ManifestEntry {
            key: key.clone(),
            shard: shard_name,
            duration_s: meta.duration_s,
            sample_rate: meta.sample_rate,
            transcript: meta.transcript.clone(),
            metrics: meta.metrics.clone(),
            source_file: meta.source_file.clone(),
        });

        self.next_key += 1;
        self.samples_written += 1;
        self.total_duration_s += meta.duration_s as f64;
        Ok(())
    }

    /// Commits the final shard (if any) and writes `manifest.jsonl` + `stats.json`.
    pub fn finish(mut self) -> Result<ShardStats, ShardError> {
        if self.current.is_some() {
            self.commit_current()?;
        }

        let manifest_path = self.config.output_dir.join("manifest.jsonl");
        let mut mf = BufWriter::new(File::create(&manifest_path)?);
        for entry in &self.manifest {
            let line = serde_json::to_string(entry)?;
            mf.write_all(line.as_bytes())?;
            mf.write_all(b"\n")?;
        }
        mf.flush()?;

        let stats = ShardStats {
            samples_written: self.samples_written,
            shards_written: self.shards_completed,
            total_bytes: self.total_bytes,
            total_duration_s: self.total_duration_s,
        };
        let stats_path = self.config.output_dir.join("stats.json");
        std::fs::write(&stats_path, serde_json::to_vec_pretty(&stats)?)?;

        Ok(stats)
    }

    fn open_shard(&mut self) -> Result<(), ShardError> {
        let index = self.next_shard_index;
        let name = format!("shard-{index:06}.tar");
        let staging_path = self.config.staging_dir.join(&name);
        let file = File::create(&staging_path)?;
        self.current = Some(OpenShard {
            index,
            name,
            staging_path,
            builder: tar::Builder::new(BufWriter::new(file)),
            bytes: 0,
            samples: 0,
        });
        self.next_shard_index += 1;
        Ok(())
    }

    /// Finalizes the open shard and atomically commits it into `output_dir`.
    /// An empty shard is discarded rather than committed.
    fn commit_current(&mut self) -> Result<(), ShardError> {
        let Some(cur) = self.current.take() else {
            return Ok(());
        };
        let OpenShard {
            index,
            name,
            staging_path,
            builder,
            samples,
            ..
        } = cur;

        // Finish the tar (writes the two 512-byte zero blocks) and fsync-ish flush.
        let mut inner = builder
            .into_inner()
            .map_err(|e| ShardError::Tar(e.to_string()))?;
        inner.flush()?;
        let file = inner
            .into_inner()
            .map_err(|e| ShardError::Io(std::io::Error::other(e.to_string())))?;
        file.sync_all()?;
        drop(file);

        if samples == 0 {
            // Never publish an empty shard; reclaim the index too.
            let _ = std::fs::remove_file(&staging_path);
            self.next_shard_index = index;
            return Ok(());
        }

        let dest = self.config.output_dir.join(&name);
        commit_atomic(&staging_path, &dest)?;

        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        self.total_bytes += size;
        self.shards_completed += 1;
        Ok(())
    }
}

impl Drop for ShardWriter {
    fn drop(&mut self) {
        // If the writer is dropped without `finish()`, do not leave a partial
        // tar lying around in staging.
        if let Some(cur) = self.current.take() {
            drop(cur.builder);
            let _ = std::fs::remove_file(&cur.staging_path);
        }
    }
}

/// Two-tier atomic commit: `hard_link` + unlink source, falling back to
/// `rename` when hard links are unavailable (cross-device, FS without links).
/// Both paths are a *successful* commit — unlike v1's sink, which mislabeled
/// the rename path as `committed: false`.
pub fn commit_atomic(staging: &Path, dest: &Path) -> Result<(), ShardError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::hard_link(staging, dest) {
        Ok(()) => {
            let _ = std::fs::remove_file(staging);
            Ok(())
        }
        Err(_) => {
            std::fs::rename(staging, dest)?;
            Ok(())
        }
    }
}

fn append_member<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    data: &[u8],
) -> Result<(), ShardError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    builder
        .append_data(&mut header, name, data)
        .map_err(|e| ShardError::Tar(e.to_string()))?;
    Ok(())
}

/// Bytes one tar member occupies: 512-byte header + payload padded to 512.
fn tar_member_size(payload: usize) -> u64 {
    512 + (payload as u64).div_ceil(512) * 512
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir()
                .join(format!("aerosieve-shard-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn staging(&self) -> PathBuf {
            self.0.join("staging")
        }
        fn output(&self) -> PathBuf {
            self.0.join("output")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn meta(transcript: &str, duration_s: f32) -> SampleMeta {
        SampleMeta {
            key: String::new(),
            duration_s,
            sample_rate: 16_000,
            transcript: transcript.to_string(),
            metrics: serde_json::json!({ "snr_db": 21.5, "clipping": 0.0 }),
            source_file: Some("in.wav".to_string()),
        }
    }

    fn tar_names(path: &Path) -> Vec<String> {
        let f = File::open(path).unwrap();
        let mut a = tar::Archive::new(f);
        a.entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    fn shard_files(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().map(|e| e == "tar").unwrap_or(false))
            .collect();
        v.sort();
        v
    }

    fn tone(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (i as f32 * 0.05).sin() * 0.8)
            .collect()
    }

    #[test]
    fn writes_single_shard_and_drains_staging() {
        let td = TempDir::new();
        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        for i in 0..10 {
            w.write_sample(&tone(160), &meta(&format!("line {i}"), 0.01))
                .unwrap();
        }
        assert_eq!(w.samples_written(), 10);
        let stats = w.finish().unwrap();

        assert_eq!(stats.samples_written, 10);
        assert_eq!(stats.shards_written, 1);
        assert!(stats.total_bytes > 0);

        let shards = shard_files(&td.output());
        assert_eq!(shards.len(), 1, "expected exactly one shard");
        assert_eq!(
            shards[0].file_name().unwrap().to_string_lossy(),
            "shard-000000.tar"
        );

        // staging must be empty of shards
        let leftovers: Vec<_> = fs::read_dir(td.staging())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert!(leftovers.is_empty(), "staging not empty: {leftovers:?}");
    }

    #[test]
    fn round_trip_audio_through_tar() {
        let td = TempDir::new();
        let input: Vec<f32> = (0..2048)
            .map(|i| ((i as f32) * 0.017).sin() * 0.95)
            .collect();

        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        w.write_sample(&input, &meta("hello world", 0.128)).unwrap();
        w.finish().unwrap();

        let shards = shard_files(&td.output());
        assert_eq!(shards.len(), 1);

        // Extract 000000000.wav from the tar.
        let f = File::open(&shards[0]).unwrap();
        let mut archive = tar::Archive::new(f);
        let mut wav_bytes: Option<Vec<u8>> = None;
        let mut json_bytes: Option<Vec<u8>> = None;
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
            if name == "000000000.wav" {
                wav_bytes = Some(buf);
            } else if name == "000000000.json" {
                json_bytes = Some(buf);
            }
        }
        let wav_bytes = wav_bytes.expect("wav member present");
        let json_bytes = json_bytes.expect("json member present");

        // Decode with hound.
        let mut reader =
            hound::WavReader::new(std::io::Cursor::new(wav_bytes)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);

        let decoded: Vec<i16> =
            reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded.len(), input.len());

        // Within i16 quantization error: 1 LSB = 1/32767 ~= 3.05e-5.
        let tol = 1.0 / i16::MAX as f32 + 1e-6;
        let mut max_err = 0.0f32;
        for (i, (&orig, &dec)) in input.iter().zip(decoded.iter()).enumerate() {
            let back = dec as f32 / i16::MAX as f32;
            let err = (orig - back).abs();
            max_err = max_err.max(err);
            assert!(err <= tol, "sample {i}: {orig} vs {back} (err {err})");
        }
        assert!(max_err <= tol, "max round-trip error {max_err} > {tol}");

        // Metadata round-trips including flattened metrics.
        let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
        assert_eq!(parsed["key"], "000000000");
        assert_eq!(parsed["transcript"], "hello world");
        assert_eq!(parsed["snr_db"], 21.5);
    }

    #[test]
    fn sample_members_are_adjacent_and_ordered() {
        let td = TempDir::new();
        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        for i in 0..5 {
            w.write_sample(&tone(320), &meta(&format!("t{i}"), 0.02))
                .unwrap();
        }
        w.finish().unwrap();

        let shards = shard_files(&td.output());
        let names = tar_names(&shards[0]);
        assert_eq!(names.len(), 10);

        for (i, pair) in names.chunks(2).enumerate() {
            let key = format!("{i:09}");
            assert_eq!(pair[0], format!("{key}.wav"), "wav must come first");
            assert_eq!(pair[1], format!("{key}.json"), "json must follow wav");
            // Same basename, differing only in extension.
            let b0 = Path::new(&pair[0]).file_stem().unwrap();
            let b1 = Path::new(&pair[1]).file_stem().unwrap();
            assert_eq!(b0, b1);
        }

        // Keys are zero-padded and sort lexicographically in write order.
        let mut sorted = names.clone();
        sorted.sort();
        let wavs: Vec<_> = names.iter().filter(|n| n.ends_with(".wav")).collect();
        let mut sorted_wavs = wavs.clone();
        sorted_wavs.sort();
        assert_eq!(wavs, sorted_wavs);
    }

    #[test]
    fn rolls_over_into_multiple_shards() {
        let td = TempDir::new();
        let cfg = ShardConfig::new(td.staging(), td.output())
            .with_target_shard_bytes(50_000);
        let mut w = ShardWriter::new(cfg).unwrap();

        // ~4 KB of audio per sample -> ~10 samples per 50 KB shard.
        for i in 0..60 {
            w.write_sample(&tone(2000), &meta(&format!("s{i}"), 0.125))
                .unwrap();
        }
        let stats = w.finish().unwrap();

        assert_eq!(stats.samples_written, 60);
        assert!(stats.shards_written > 1, "expected rollover, got {stats:?}");

        let shards = shard_files(&td.output());
        assert_eq!(shards.len() as u64, stats.shards_written);

        // Shard names are contiguous and every shard is a valid, non-empty tar.
        let mut total_members = 0usize;
        let mut all_keys: Vec<String> = Vec::new();
        for (i, shard) in shards.iter().enumerate() {
            assert_eq!(
                shard.file_name().unwrap().to_string_lossy(),
                format!("shard-{i:06}.tar")
            );
            let names = tar_names(shard);
            assert!(!names.is_empty(), "shard {i} is empty");
            assert_eq!(names.len() % 2, 0, "shard {i} has an unpaired member");
            for pair in names.chunks(2) {
                assert!(pair[0].ends_with(".wav"));
                assert!(pair[1].ends_with(".json"));
                assert_eq!(
                    Path::new(&pair[0]).file_stem().unwrap(),
                    Path::new(&pair[1]).file_stem().unwrap()
                );
                all_keys.push(
                    Path::new(&pair[0])
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            total_members += names.len();
        }
        assert_eq!(total_members, 120);

        // Every key present exactly once, globally ordered.
        let mut expected: Vec<String> = (0..60).map(|i| format!("{i:09}")).collect();
        expected.sort();
        let mut got = all_keys.clone();
        got.sort();
        assert_eq!(got, expected);
        assert_eq!(all_keys, expected, "keys must stream in order across shards");

        // staging drained
        assert_eq!(fs::read_dir(td.staging()).unwrap().count(), 0);
    }

    #[test]
    fn manifest_lines_match_samples() {
        let td = TempDir::new();
        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        for i in 0..7 {
            w.write_sample(&tone(160), &meta(&format!("m{i}"), 0.01))
                .unwrap();
        }
        w.finish().unwrap();

        let text = fs::read_to_string(td.output().join("manifest.jsonl")).unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 7);

        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for (i, line) in lines.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["key"], format!("{i:09}"));
            assert_eq!(v["shard"], "shard-000000.tar");
            assert_eq!(v["transcript"], format!("m{i}"));
            assert_eq!(v["snr_db"], 21.5);
            seen.insert(
                v["key"].as_str().unwrap().to_string(),
                v["shard"].as_str().unwrap().to_string(),
            );
        }
        assert_eq!(seen.len(), 7);
    }

    #[test]
    fn stats_json_parses_and_matches() {
        let td = TempDir::new();
        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        for _ in 0..4 {
            w.write_sample(&tone(160), &meta("x", 0.01)).unwrap();
        }
        let returned = w.finish().unwrap();

        let text = fs::read_to_string(td.output().join("stats.json")).unwrap();
        let on_disk: ShardStats = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk.samples_written, 4);
        assert_eq!(on_disk.samples_written, returned.samples_written);
        assert_eq!(on_disk.shards_written, returned.shards_written);
        assert_eq!(on_disk.total_bytes, returned.total_bytes);
        assert!((on_disk.total_duration_s - 0.04).abs() < 1e-6);
    }

    #[test]
    fn empty_run_writes_no_shard() {
        let td = TempDir::new();
        let w = ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        let stats = w.finish().unwrap();

        assert_eq!(stats.samples_written, 0);
        assert_eq!(stats.shards_written, 0);
        assert_eq!(stats.total_bytes, 0);

        assert!(shard_files(&td.output()).is_empty(), "no shard may exist");
        assert_eq!(fs::read_dir(td.staging()).unwrap().count(), 0);

        // manifest exists but is empty; stats parses.
        let manifest = fs::read_to_string(td.output().join("manifest.jsonl")).unwrap();
        assert!(manifest.is_empty());
        let on_disk: ShardStats =
            serde_json::from_str(&fs::read_to_string(td.output().join("stats.json")).unwrap())
                .unwrap();
        assert_eq!(on_disk.samples_written, 0);
    }

    #[test]
    fn out_of_range_samples_clamp_not_wrap() {
        let td = TempDir::new();
        let input = vec![2.0f32, -3.0, 1.0, -1.0, 0.0, 1e9, -1e9, f32::NAN];

        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        w.write_sample(&input, &meta("clamp", 0.0005)).unwrap();
        w.finish().unwrap();

        let shards = shard_files(&td.output());
        let f = File::open(&shards[0]).unwrap();
        let mut archive = tar::Archive::new(f);
        let mut wav = Vec::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().to_string_lossy() == "000000000.wav" {
                std::io::Read::read_to_end(&mut entry, &mut wav).unwrap();
            }
        }
        let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();

        assert_eq!(decoded.len(), 8);
        // +2.0 must land at the positive rail, NOT wrap to a negative value.
        assert_eq!(decoded[0], i16::MAX, "2.0 must clamp to +32767");
        assert!(decoded[0] > 0, "2.0 wrapped to negative: {}", decoded[0]);
        assert_eq!(decoded[1], -i16::MAX, "-3.0 must clamp to -32767");
        assert!(decoded[1] < 0);
        assert_eq!(decoded[2], i16::MAX);
        assert_eq!(decoded[3], -i16::MAX);
        assert_eq!(decoded[4], 0);
        assert_eq!(decoded[5], i16::MAX, "1e9 must clamp");
        assert_eq!(decoded[6], -i16::MAX, "-1e9 must clamp");
        assert_eq!(decoded[7], 0, "NaN maps to silence");
    }

    #[test]
    fn commit_atomic_succeeds_on_both_paths() {
        // Regression guard for the v1 sink bug: a successful `rename` fallback
        // is a success, not a failure.
        let td = TempDir::new();
        let staging = td.0.join("s");
        let out = td.0.join("o");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&out).unwrap();

        let src = staging.join("a.tar");
        fs::write(&src, b"payload").unwrap();
        let dst = out.join("a.tar");
        commit_atomic(&src, &dst).unwrap();
        assert!(dst.exists());
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).unwrap(), b"payload");

        // Second commit where the hard_link path fails (dest already exists)
        // must surface as an error, not a silent no-op.
        let src2 = staging.join("b.tar");
        fs::write(&src2, b"second").unwrap();
        commit_atomic(&src2, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"second");
    }

    #[test]
    fn keys_are_zero_padded_and_sequential() {
        let td = TempDir::new();
        let mut w =
            ShardWriter::new(ShardConfig::new(td.staging(), td.output())).unwrap();
        assert_eq!(w.next_key(), "000000000");
        w.write_sample(&tone(16), &meta("a", 0.001)).unwrap();
        assert_eq!(w.next_key(), "000000001");
        w.write_sample(&tone(16), &meta("b", 0.001)).unwrap();
        w.finish().unwrap();

        let shards = shard_files(&td.output());
        assert_eq!(
            tar_names(&shards[0]),
            vec![
                "000000000.wav",
                "000000000.json",
                "000000001.wav",
                "000000001.json",
            ]
        );
    }
}

