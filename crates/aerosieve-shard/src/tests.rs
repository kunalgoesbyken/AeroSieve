use std::fs;
use std::io::Read;

use serde_json::json;
use tempfile::TempDir;

use super::*;

const SAMPLE_RATE: u32 = 16_000;

struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    fn staging(&self) -> PathBuf {
        self.root.path().join("staging")
    }

    fn output(&self) -> PathBuf {
        self.root.path().join("output")
    }

    fn config(&self) -> ShardConfig {
        ShardConfig::new(self.staging(), self.output())
    }

    fn writer(&self) -> ShardWriter {
        ShardWriter::new(self.config()).unwrap()
    }

    fn shard_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = fs::read_dir(self.output())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "tar"))
            .collect();
        paths.sort();
        paths
    }

    fn manifest(&self) -> Vec<SampleRecord> {
        fs::read_to_string(self.output().join(MANIFEST_FILE))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn staging_is_empty(&self) -> bool {
        fs::read_dir(self.staging()).unwrap().next().is_none()
    }
}

fn meta(transcript: &str) -> SampleMeta {
    let mut meta = SampleMeta::new(0.01, SAMPLE_RATE)
        .with_metrics(&json!({ "snr_db": 21.5 }))
        .unwrap();
    meta.transcript = transcript.to_string();
    meta.source_file = Some("in.wav".to_string());
    meta
}

fn audio(len: usize) -> Vec<f32> {
    (0..len).map(|i| (i as f32 * 0.05).sin() * 0.8).collect()
}

fn members(shard: &Path) -> Vec<(String, Vec<u8>)> {
    tar::Archive::new(File::open(shard).unwrap())
        .entries()
        .unwrap()
        .map(|entry| {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            (name, data)
        })
        .collect()
}

fn member_names(shard: &Path) -> Vec<String> {
    members(shard).into_iter().map(|(name, _)| name).collect()
}

fn decode_wav(bytes: &[u8]) -> (hound::WavSpec, Vec<i16>) {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
    let samples = reader.samples::<i16>().map(Result::unwrap).collect();
    (reader.spec(), samples)
}

#[test]
fn samples_are_stored_as_adjacent_wav_json_pairs_in_key_order() {
    let workspace = Workspace::new();
    let mut writer = workspace.writer();

    let keys: Vec<String> = (0..3)
        .map(|_| writer.write_sample(&audio(160), meta("line")).unwrap())
        .collect();
    let stats = writer.finish().unwrap();

    assert_eq!(keys, ["000000000", "000000001", "000000002"]);
    assert_eq!(stats.samples_written, 3);
    assert_eq!(stats.shards_written, 1);

    let shards = workspace.shard_paths();
    assert_eq!(shards, [workspace.output().join("shard-000000.tar")]);
    assert_eq!(
        member_names(&shards[0]),
        [
            "000000000.wav",
            "000000000.json",
            "000000001.wav",
            "000000001.json",
            "000000002.wav",
            "000000002.json",
        ]
    );
    assert!(workspace.staging_is_empty());
}

#[test]
fn audio_round_trips_within_one_quantization_step() {
    let workspace = Workspace::new();
    let input = audio(2048);
    let mut writer = workspace.writer();
    writer.write_sample(&input, meta("hello")).unwrap();
    writer.finish().unwrap();

    let (_, wav) = &members(&workspace.shard_paths()[0])[0];
    let (spec, decoded) = decode_wav(wav);

    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (1, SAMPLE_RATE, 16)
    );
    assert_eq!(decoded.len(), input.len());
    let step = 1.0 / f32::from(i16::MAX);
    for (original, quantized) in input.iter().zip(decoded) {
        assert!((original - f32::from(quantized) * step).abs() <= step);
    }
}

#[test]
fn wav_is_encoded_at_the_rate_the_metadata_declares() {
    let workspace = Workspace::new();
    let mut writer = workspace.writer();
    writer
        .write_sample(&audio(441), SampleMeta::new(0.01, 44_100))
        .unwrap();
    writer.finish().unwrap();

    let (_, wav) = &members(&workspace.shard_paths()[0])[0];
    assert_eq!(decode_wav(wav).0.sample_rate, 44_100);
}

#[test]
fn out_of_range_samples_saturate_and_nan_becomes_silence() {
    let workspace = Workspace::new();
    let input = [2.0, -3.0, 1.0, -1.0, 0.0, 1e9, -1e9, f32::NAN];
    let mut writer = workspace.writer();
    writer.write_sample(&input, meta("clamp")).unwrap();
    writer.finish().unwrap();

    let (_, wav) = &members(&workspace.shard_paths()[0])[0];
    let (max, min) = (i16::MAX, -i16::MAX);
    assert_eq!(decode_wav(wav).1, [max, min, max, min, 0, max, min, 0]);
}

#[test]
fn sidecar_json_and_manifest_line_describe_the_same_record() {
    let workspace = Workspace::new();
    let mut writer = workspace.writer();
    writer.write_sample(&audio(160), meta("नमस्ते")).unwrap();
    writer.finish().unwrap();

    let (_, json) = &members(&workspace.shard_paths()[0])[1];
    let sidecar: SampleRecord = serde_json::from_slice(json).unwrap();

    assert_eq!(workspace.manifest(), std::slice::from_ref(&sidecar));
    assert_eq!(sidecar.key, "000000000");
    assert_eq!(sidecar.shard, "shard-000000.tar");
    assert_eq!(sidecar.meta, meta("नमस्ते"));

    let raw: serde_json::Value = serde_json::from_slice(json).unwrap();
    assert_eq!(raw["snr_db"], 21.5);
}

#[test]
fn metrics_must_serialize_to_an_object() {
    let result = SampleMeta::new(1.0, SAMPLE_RATE).with_metrics(&42);
    assert!(matches!(result, Err(ShardError::Json(_))));
}

#[test]
fn shards_roll_over_without_splitting_a_sample() {
    const SAMPLES: usize = 60;
    let workspace = Workspace::new();
    let config = workspace.config().with_target_shard_bytes(50_000);
    let mut writer = ShardWriter::new(config).unwrap();
    for _ in 0..SAMPLES {
        writer.write_sample(&audio(2000), meta("s")).unwrap();
    }
    let stats = writer.finish().unwrap();

    let shards = workspace.shard_paths();
    assert!(shards.len() > 1);
    assert_eq!(shards.len() as u64, stats.shards_written);
    assert_eq!(
        stats.total_bytes,
        shards
            .iter()
            .map(|shard| fs::metadata(shard).unwrap().len())
            .sum::<u64>()
    );

    let mut keys = Vec::new();
    for (index, shard) in shards.iter().enumerate() {
        assert_eq!(
            shard.file_name().unwrap().to_string_lossy(),
            format!("shard-{index:06}.tar")
        );
        for pair in member_names(shard).chunks(2) {
            let key = pair[0].strip_suffix(".wav").unwrap().to_string();
            assert_eq!(pair[1], format!("{key}.json"));
            keys.push(key);
        }
    }
    let expected: Vec<String> = (0..SAMPLES).map(|i| format!("{i:09}")).collect();
    assert_eq!(keys, expected);

    let manifest = workspace.manifest();
    assert_eq!(manifest.len(), SAMPLES);
    assert!(
        manifest
            .iter()
            .all(|record| workspace.output().join(&record.shard).exists())
    );
    assert!(workspace.staging_is_empty());
}

#[test]
fn stats_file_matches_the_returned_stats() {
    let workspace = Workspace::new();
    let mut writer = workspace.writer();
    for _ in 0..4 {
        writer.write_sample(&audio(160), meta("x")).unwrap();
    }
    assert_eq!(writer.stats().samples_written, 4);
    let returned = writer.finish().unwrap();

    let on_disk: ShardStats =
        serde_json::from_slice(&fs::read(workspace.output().join(STATS_FILE)).unwrap()).unwrap();

    assert_eq!(on_disk, returned);
    assert!((on_disk.total_duration_s - 0.04).abs() < 1e-6);
}

#[test]
fn an_empty_run_publishes_an_empty_manifest_and_no_shard() {
    let workspace = Workspace::new();

    let stats = workspace.writer().finish().unwrap();

    assert_eq!(stats, ShardStats::default());
    assert!(workspace.shard_paths().is_empty());
    assert!(workspace.manifest().is_empty());
    assert!(workspace.staging_is_empty());
}

#[test]
fn a_writer_dropped_before_finish_publishes_only_completed_shards() {
    let workspace = Workspace::new();
    let config = workspace.config().with_target_shard_bytes(50_000);
    let mut writer = ShardWriter::new(config).unwrap();
    for _ in 0..30 {
        writer.write_sample(&audio(2000), meta("s")).unwrap();
    }
    let committed = writer.stats().shards_written;
    drop(writer);

    assert!(committed > 0);
    assert_eq!(workspace.shard_paths().len() as u64, committed);
    assert!(!workspace.output().join(MANIFEST_FILE).exists());
    assert!(workspace.staging_is_empty());
}

#[test]
fn writers_sharing_a_staging_directory_do_not_collide() {
    let workspace = Workspace::new();
    let other_output = workspace.root.path().join("other");
    let mut first = workspace.writer();
    let mut second =
        ShardWriter::new(ShardConfig::new(workspace.staging(), &other_output)).unwrap();

    first.write_sample(&audio(160), meta("first")).unwrap();
    second.write_sample(&audio(320), meta("second")).unwrap();
    first.finish().unwrap();
    second.finish().unwrap();

    assert_eq!(workspace.manifest()[0].meta.transcript, "first");
    let other_shard = other_output.join("shard-000000.tar");
    assert_eq!(decode_wav(&members(&other_shard)[0].1).1.len(), 320);
}
