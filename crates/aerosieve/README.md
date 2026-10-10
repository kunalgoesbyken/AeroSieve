# aerosieve

The curation pipeline: decode, segment, score, shard.

```rust
use aerosieve::{Pipeline, PipelineConfig};

let config = PipelineConfig::new("data/staging", "data/clean")
    .with_rules("crates/aerosieve-lexical/rules/hinglish.yaml");
let mut pipeline = Pipeline::new(config)?;

let outcome = pipeline.process_file("recording.wav".as_ref(), Some("yeh ₹500 hai"))?;
println!("{} of {} segments accepted", outcome.accepted_count(), outcome.segments.len());

let stats = pipeline.finish()?;
println!("retention {:.1}%", stats.retention_rate() * 100.0);
```

- `process_file` decodes a file first; `process_audio` takes samples that are already decoded.
- Each call returns a `FileOutcome` listing every segment with its metrics and, if it was rejected, the reason.
- `Inspector` does the same decoding, segmenting, and scoring without writing anything.
- `finish` commits the last shard and returns `PipelineStats`, including rejections counted by reason.
- `Pipeline::new` detects speech by signal level. `Pipeline::with_detector` accepts any `SpeechDetector`, such as `SileroDetector` under the `silero` feature.

The decode and segment stages must agree on a sample rate; `PipelineConfig::with_sample_rate` sets both, and a mismatch is an error.

The stage crates are re-exported as `aerosieve::{acoustic, decode, lexical, segment, shard}`.

## Examples and benchmarks

```bash
cargo run --release --example basic
cargo run --release --example report
cargo bench --bench pipeline_benchmark
```
