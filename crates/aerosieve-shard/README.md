# aerosieve-shard

WebDataset tar shard writer with atomic commit semantics.

This crate is the v2 output stage. It replaces v1's per-sample
`{uuid}.raw` + `{uuid}.txt` files with **WebDataset** tar shards.

## Why shards beat per-sample files

Per-sample files are the worst case for training I/O: a corpus of a few million
utterances becomes a few million tiny files read in random order. Every sample
costs a metadata lookup plus a seek, and on object storage every sample costs a
separate HTTP request.

WebDataset packs samples into large tar files that the loader reads
**sequentially**. Measured sequential throughput is **3-10x** higher than random
access over many small files, and shards stream directly from object stores
(S3/GCS) with no local extraction step. PyTorch's `webdataset` /
`torchdata` loaders consume this format natively.

The one tradeoff: updating a single sample means rewriting its shard. For a
write-once training corpus that is the right tradeoff.

## Layout

A shard is a plain, uncompressed tar. Files belonging to one training sample
share a basename and differ only in extension, and they are **adjacent and in
order** inside the archive — that adjacency is what lets a streaming loader
group a sample without seeking.

```
output/
  shard-000000.tar
  shard-000001.tar
  shard-000002.tar
  manifest.jsonl
  stats.json
```

Listing one shard (`tar tvf shard-000000.tar`):

```
-rw-r--r-- 0/0  64044  000000000.wav
-rw-r--r-- 0/0    218  000000000.json
-rw-r--r-- 0/0  51244  000000001.wav
-rw-r--r-- 0/0    204  000000001.json
-rw-r--r-- 0/0  96044  000000002.wav
-rw-r--r-- 0/0    231  000000002.json
...
```

Rules this crate enforces:

- Keys are zero-padded sequential (`format!("{:09}", n)`) so lexical order is
  numeric order — shards stream in write order.
- `.wav` is always emitted immediately before its `.json`; a sample's two
  members never straddle a shard boundary (rollover is checked *before* the
  pair is written).
- Shards are named `shard-{:06}.tar` and roll at `target_shard_bytes`
  (default ~1 GB), sized from the real on-tape cost of each member
  (512-byte header + payload padded to 512).
- Audio is 16-bit mono PCM WAV written by `hound`. f32 input is clamped to
  `[-1.0, 1.0]` **before** scaling to i16, so an out-of-range `2.0` saturates at
  `+32767` instead of wrapping to a negative value. `NaN` maps to silence.

### `000000000.json`

`SampleMeta` serializes with `metrics` flattened into the top level, so quality
metrics sit alongside the fixed fields:

```json
{
  "key": "000000000",
  "duration_s": 4.002,
  "sample_rate": 16000,
  "transcript": "the quick brown fox",
  "snr_db": 21.5,
  "clipping": 0.0,
  "source_file": "raw/episode-17.opus"
}
```

### `manifest.jsonl`

One JSON object per line per accepted sample, with the shard it landed in —
enough to build a train/val split or look a sample up without opening any tar.

```json
{"key":"000000000","shard":"shard-000000.tar","duration_s":4.002,"sample_rate":16000,"transcript":"the quick brown fox","snr_db":21.5,"clipping":0.0,"source_file":"raw/episode-17.opus"}
```

### `stats.json`

The returned `ShardStats`: `samples_written`, `shards_written`, `total_bytes`,
`total_duration_s`.

## Atomic commit semantics

A shard is built in `staging_dir` and only appears in `output_dir` once it is
complete. On rollover (and on `finish()`), the writer:

1. Finalizes the tar (both trailing zero blocks), flushes, and `sync_all()`s.
2. Commits the file into `output_dir` with a two-tier scheme inherited from
   `aerosieve-sink`:
   - `hard_link(staging, output)` then unlink the staging entry — the source is
     never in a half-moved state.
   - If hard links are unavailable (cross-device, or a filesystem without
     them), fall back to `rename(staging, output)`, which is atomic within a
     volume.

A reader scanning `output_dir` therefore only ever sees whole, valid tars.
A crash mid-shard leaves a partial file in staging, which is never published;
dropping a `ShardWriter` without calling `finish()` removes it.

**Both commit paths are a success.** v1's sink set `WriteResult.committed =
false` on the successful `rename` fallback, which made a good commit look like
a failure. `commit_atomic` here returns `Ok(())` for either mechanism and only
errors when the commit genuinely did not happen.

An empty run publishes no shard: `finish()` with zero samples discards the
unused staging file, writes an empty `manifest.jsonl`, and writes `stats.json`
with zero counts.

## Usage

```rust
use aerosieve_shard::{ShardConfig, ShardWriter, SampleMeta};

let mut writer = ShardWriter::new(
    ShardConfig::new("work/staging", "out/shards")
)?;

for (audio, transcript) in accepted_samples {
    let mut meta = SampleMeta::new("", audio.len() as f32 / 16_000.0, 16_000);
    meta.transcript = transcript;
    meta.metrics = serde_json::json!({ "snr_db": 21.5 });
    writer.write_sample(&audio, &meta)?;   // key is assigned by the writer
}

let stats = writer.finish()?;
println!("{} samples across {} shards", stats.samples_written, stats.shards_written);
```

## Tests

```
cargo test -p aerosieve-shard
```

Coverage includes a full **round-trip**: known f32 audio is written, the shard
is committed, the tar is re-read with the `tar` crate, the `.wav` member is
decoded with `hound`, and every sample is asserted to be within one i16 LSB
(3.05e-5) of the input. Measured max error over a 2048-sample sine is
**1.53e-5** (half an LSB, i.e. pure rounding). Other tests cover member
adjacency and ordering, multi-shard rollover with global key ordering,
manifest/stats consistency, the empty run, and clamping of out-of-range f32
input.
