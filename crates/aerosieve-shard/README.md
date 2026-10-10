# aerosieve-shard

[WebDataset](https://github.com/webdataset/webdataset) tar shard writer with atomic commits.

Per-sample files are the worst case for training I/O: millions of small files read in random order, each a separate request on object storage. WebDataset packs samples into large tar files that a loader reads sequentially and can stream straight from a bucket.

```rust
use aerosieve_shard::{SampleMeta, ShardConfig, ShardWriter};

let mut writer = ShardWriter::new(ShardConfig::new("work/staging", "out/shards"))?;

for (audio, transcript) in accepted_samples {
    let mut meta = SampleMeta::new(audio.len() as f32 / 16_000.0, 16_000)
        .with_metrics(&serde_json::json!({ "snr_db": 21.5 }))?;
    meta.transcript = transcript;
    let key = writer.write_sample(&audio, meta)?;
}

let stats = writer.finish()?;
println!("{} samples across {} shards", stats.samples_written, stats.shards_written);
```

## Layout

```
out/shards/
  shard-000000.tar
  shard-000001.tar
  manifest.jsonl
  stats.json
```

Inside a shard, each sample is two adjacent members sharing a key:

```
000000000.wav
000000000.json
000000001.wav
000000001.json
```

- Keys are sequential and zero-padded, so they sort in write order across shards.
- `.wav` always comes immediately before its `.json`, and the pair never straddles two shards.
- A shard is closed once it reaches `target_shard_bytes` (1 GB by default).
- Audio is mono 16-bit PCM at the rate given in the sample's metadata. Out-of-range samples saturate instead of wrapping, and NaN becomes silence.

`{key}.json` and each line of `manifest.jsonl` hold the same record, with `metrics` flattened into the top level:

```json
{"key":"000000000","shard":"shard-000000.tar","duration_s":4.002,"sample_rate":16000,"transcript":"the quick brown fox","source_file":"raw/episode-17.opus","snr_db":21.5}
```

`stats.json` holds the `ShardStats` returned by `finish`.

## Commit semantics

A shard is built in a private scratch directory under `staging_dir`, flushed to disk, and then renamed into `output_dir`. A reader of `output_dir` therefore only ever sees complete tars. The manifest and stats are published by `finish`.

- `staging_dir` and `output_dir` must be on the same volume for the rename to be atomic.
- A writer dropped without `finish` removes its scratch directory. Shards it had already completed remain in `output_dir`, without a manifest.
- Several writers can share one `staging_dir`.
- A run with no samples publishes an empty manifest and no shard.
- Shard and manifest names are deterministic, so writing into an `output_dir` that already holds a run replaces that run's files. Use a fresh directory per run.
