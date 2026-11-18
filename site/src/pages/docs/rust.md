---
layout: ../../layouts/Docs.astro
---

## Install

```bash
cargo add aerosieve
```

| Feature | Default | Adds |
| :--- | :--- | :--- |
| `parallel` | yes | `Pipeline::process_files`, which uses every core through rayon. Disable default features to build without rayon. |
| `silero` | no | `SileroDetector`, a Silero VAD speech detector run through ONNX Runtime. See [Silero VAD](#silero-vad). |

The stage crates are re-exported as `aerosieve::{acoustic, decode, lexical, segment, shard}`, along with `FilterPolicy`, `QualityMetrics`, `RejectReason`, `DecodeConfig`, `SegmentConfig` and `EnergyDetector`.

## Pipeline

`Pipeline` is the batch path: decode, segment, score, shard.

```rust
use std::path::Path;

use aerosieve::{Pipeline, PipelineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = PipelineConfig::new("data/staging", "data/run-1")
        .with_rules("crates/aerosieve-lexical/rules/hinglish.yaml");
    let mut pipeline = Pipeline::new(config)?;

    let outcome = pipeline.process_file(Path::new("recording.wav"), Some("yeh ₹500 hai"))?;
    println!("{} of {} clips accepted", outcome.accepted_count(), outcome.segments.len());

    let stats = pipeline.finish()?;
    println!("kept {:.1}% of detected speech", stats.retention_rate() * 100.0);
    Ok(())
}
```

- `PipelineConfig::new(staging_dir, output_dir)` uses the defaults for every stage. `with_rules`, `with_policy` and `with_sample_rate` adjust them. Its fields `decode`, `segment`, `policy`, `shard` and `rules_path` are public.
- `process_file(path, transcript)` decodes a file and curates it. `process_audio(source, &DecodedAudio, transcript)` takes audio that is already decoded.
- The transcript describes the whole input. It is normalized by the rules, if any, and attached to every accepted clip. It is not aligned to individual clips.
- Each call returns a `FileOutcome`: `path`, `duration_s`, `sample_rate`, `source_sample_rate`, `applied_gain_db` and `segments`. Every segment is a `SegmentOutcome` with its `segment` (sample positions), `metrics`, and `rejected`, which is `None` when the clip was accepted.
- `finish()` commits the last shard and returns `PipelineStats`: `files_processed`, `files_failed`, `segments_found`, `segments_accepted`, `rejections` by reason, `speech_duration_s`, `accepted_duration_s` and the `shard` totals. `retention_rate()` is the share of detected speech, by duration, that was kept.
- The decode and segment stages must agree on a sample rate. `with_sample_rate` sets both, and a mismatch is a `PipelineError::SampleRateMismatch`.
- The output directory must be fresh. A writer refuses one that already holds shards, a manifest or stats.

### Many files

`process_files` curates many files on every core, with the same result and the same shards as calling `process_file` on each in order. It needs the `parallel` feature, and a detector that is `Clone + Send + Sync`.

```rust
use std::path::PathBuf;

use aerosieve::{Pipeline, PipelineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut pipeline = Pipeline::new(PipelineConfig::new("data/staging", "data/run-2"))?;

    let inputs: Vec<(PathBuf, Option<String>)> = vec![
        (PathBuf::from("a.wav"), Some("hello".to_string())),
        (PathBuf::from("b.mp3"), None),
    ];
    for ((path, _), result) in inputs.iter().zip(pipeline.process_files(&inputs)) {
        if let Err(error) = result {
            eprintln!("{}: skipped ({error})", path.display());
        }
    }

    pipeline.finish()?;
    Ok(())
}
```

It returns one `Result<FileOutcome, PipelineError>` per input, in input order. A file that fails is counted in `files_failed` and the rest continue.

## Inspector

`Inspector` decodes, segments and scores without writing anything. `Pipeline` wraps one.

```rust
use std::path::Path;

use aerosieve::{DecodeConfig, FilterPolicy, Inspector, SegmentConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let policy = FilterPolicy { min_snr_db: 15.0, ..FilterPolicy::default() };
    let mut inspector = Inspector::new(DecodeConfig::default(), SegmentConfig::default(), policy)?;

    let outcome = inspector.inspect_file(Path::new("recording.wav"))?;
    for clip in &outcome.segments {
        let verdict = clip.rejected.map_or("accept", |reason| reason.as_str());
        println!(
            "{:.1}s {:.1}s snr {:.1} dB {}",
            clip.segment.start_sample as f32 / outcome.sample_rate as f32,
            clip.segment.duration_s(outcome.sample_rate),
            clip.metrics.wada_snr_db,
            verdict,
        );
    }
    Ok(())
}
```

`inspect_audio(source, &DecodedAudio)` does the same for audio you decoded yourself. `SegmentOutcome::summary(sample_rate)` returns a `ClipSummary` in seconds with the verdict as a string, which serializes to the dict shape the [Python bindings](/docs/python#return-value) return.

### Pre-cut utterances

`score_whole` scores audio that is already one utterance as a single clip, without segmenting it. It keeps audio matched to its transcript and does not drop anything under the segmenter's minimum length. Pauses and edge silence count towards the metrics, and digital silence or an empty buffer is reported as `RejectReason::Silence` instead of producing no clip.

```rust
use std::path::Path;

use aerosieve::decode::decode_file;
use aerosieve::{DecodeConfig, FilterPolicy, Inspector, SegmentConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inspector = Inspector::new(DecodeConfig::default(), SegmentConfig::default(), FilterPolicy::default())?;

    let path = Path::new("utterance.wav");
    let audio = decode_file(path, &DecodeConfig::default())?;
    let outcome = inspector.score_whole(path, &audio)?;
    println!("{:?}", outcome.segments[0].rejected);
    Ok(())
}
```

## LiveInspector

`LiveInspector` judges audio that arrives in pieces, such as a microphone or a call. Each clip is judged as soon as the pause that ends it arrives, and the verdicts match those of `inspect_audio` on the whole buffer, whatever the chunking. Get one from `Inspector::live()`.

```rust
use aerosieve::{DecodeConfig, FilterPolicy, Inspector, SegmentConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inspector = Inspector::new(DecodeConfig::default(), SegmentConfig::default(), FilterPolicy::default())?;
    let mut live = inspector.live();

    let chunks: Vec<Vec<f32>> = vec![vec![0.0; 320]];
    for chunk in &chunks {
        for clip in live.push(chunk)? {
            println!("{:?} at {:.2}s", clip.rejected, live.heard_s());
        }
    }
    for clip in live.finish() {
        println!("{:?}", clip.rejected);
    }
    Ok(())
}
```

- `push(&[f32])` takes mono samples at `sample_rate()`, 16 kHz by default, and returns the clips they completed, in order.
- `finish()` ends the stream and judges the clips still pending.
- `heard_s()` is the seconds of audio received. `held_s()` is the seconds held because an unjudged clip may need them, roughly the utterance in progress. It is bounded only by how long speakers go without pausing, so a server should cap it.
- Samples are judged as received: there is no loudness leveling and full scale is 1.0.

The [streaming server](/docs/streaming) is this type behind a WebSocket.

## Using the stages directly

Each stage is its own crate, re-exported from `aerosieve`.

```rust
use aerosieve::acoustic::{FULL_SCALE, FilterPolicy, score};

fn judge(samples: &[f32]) {
    let metrics = score(samples, 16_000, FULL_SCALE);
    match FilterPolicy::default().evaluate(&metrics) {
        None => println!("accepted"),
        Some(reason) => println!("rejected: {reason}"),
    }
}
```

`score(samples, sample_rate, full_scale)` measures a mono `f32` buffer and `FilterPolicy::evaluate` returns the first failed check. `full_scale` is the level the recording's full scale now sits at: `FULL_SCALE` for untouched audio, lower if the buffer was turned down after recording, so that leveling cannot hide clipping.

## Silero VAD

By default speech is detected from signal level, so loud music counts as speech and long pauses in a noisy recording may not split a clip. For a real speech detector, build with `--features silero` and pass a `SileroDetector` to `Pipeline::with_detector`.

```bash
curl -sL -o models/silero_vad.onnx \
  https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
```

```rust
use aerosieve::segment::SileroDetector;
use aerosieve::{Pipeline, PipelineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = PipelineConfig::new("data/staging", "data/run-3");
    let detector =
        SileroDetector::from_model_path("models/silero_vad.onnx".as_ref(), config.segment.sample_rate)?;
    let mut pipeline = Pipeline::with_detector(config, detector)?;
    pipeline.process_file("recording.wav".as_ref(), None)?;
    pipeline.finish()?;
    Ok(())
}
```

`Inspector::with_detector` takes the same detector. Silero supports 8 kHz and 16 kHz audio only. On Windows the feature needs the MSVC toolchain, because the `ort` crate ships no prebuilt ONNX Runtime for `windows-gnu`. The command-line tool does not expose Silero.

Anything that implements `SpeechDetector` can be passed the same way.

## Segmentation settings

`PipelineConfig::segment` is a `SegmentConfig`. The defaults are Silero's, plus the 3 to 30 s clip bounds used by Emilia-Pipe.

| Field | Default | Meaning |
| :--- | ---: | :--- |
| `sample_rate` | `16000` | Rate of the audio being segmented |
| `threshold` | `0.5` | Speech probability at which speech starts |
| `neg_threshold` | `0.35` | Probability below which a window counts as silence |
| `min_speech_duration_ms` | `250` | Shorter speech runs are discarded |
| `min_silence_duration_ms` | `500` | Shorter pauses are bridged. Silero uses 100, which cuts speech at every word gap |
| `speech_pad_ms` | `30` | Padding on both ends of a clip |
| `min_segment_s` | `3.0` | Shorter clips are dropped |
| `max_segment_s` | `30.0` | Longer clips are split in the middle of their longest pause. `0` disables splitting |

The full API is on docs.rs, linked at the top of this page.
