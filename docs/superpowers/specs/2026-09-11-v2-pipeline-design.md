# AeroSieve v2 — Batch Curation Pipeline

**Date:** 2026-09-11
**Status:** Approved for implementation
**Research:** [`../research/2026-09-11-pipeline-research.md`](../research/2026-09-11-pipeline-research.md)

## 1. Why

The v1 pipeline has a functional defect and a documentation gap.

**Defect.** `SieveConfig::default()` rejects 100% of frames, including clean
speech. The SNR check computes a frame RMS against a noise window taken from
that same frame; with the default `noise_window_samples: 1600` clamped by
`.min(samples.len())` to a 320-sample frame, the noise window *is* the frame, so
`snr_db == 0.0` always, which is below the 15 dB floor. Every test and benchmark
sets `noise_window_samples: 0` to disable the check, so the green suite never
exercised the shipped default. The README example prints `Passed: false`.

**Gap.** The README claims file/mic/TCP sources, hot-reloadable rules,
zero-copy `&[f32]` slices, and zero-allocation hot paths. None have
implementing code. Published benchmark numbers are 1.8–4.1x optimistic, and the
Python baseline is quoted at ~3,000 fps against a measured 14,115 fps.

v2 rebuilds the pipeline around a researched reference architecture and makes
every documented claim true.

## 2. What this is

A **batch curation pipeline**: raw audio files in, model-ready training shards
out. Throughput and retention rate are the metrics; per-frame latency is not the
product. This matches what the v1 benchmarks already measured and what
"clean training data for voice AI" actually means.

Explicitly **not** a real-time streaming gate. The v1 README framing (20 ms
frames, P99 SLA, lock-free SPSC) described a system the repo did not build.

### Reference architecture

Emilia-Pipe (arXiv 2407.05361, 101k+ hours, 6 languages) defines six stages. v2
implements four and deliberately omits two:

| # | Emilia stage | v2 |
|---|---|---|
| 1 | Standardization | yes — `aerosieve-decode` |
| 2 | Source separation (UVR-MDX-NET) | no — heavy GPU model |
| 3 | Speaker diarization (pyannote) | no — heavy GPU model |
| 4 | Fine-grained VAD segmentation | yes — `aerosieve-segment` |
| 5 | ASR (WhisperX) | no — transcripts accepted, not generated |
| 6 | Filtering | yes — `aerosieve-acoustic` |
|   | (output format) | yes — `aerosieve-shard`, WebDataset |

Omissions are documented as omissions, not implied capabilities.

## 3. Architecture

```
 source files        1 Decode          2 Segment         3 Score          4 Shard
 (wav/mp3/flac/  ->  symphonia     ->  hysteresis    ->  WADA-SNR    ->   tar shards
  ogg/mp4)           + rubato          + detector        + clip           + manifest
                     16 kHz mono       3-30 s clips      + dBFS           ~1 GB each
                     -20 dBFS                            + IQR
```

Each stage is a crate with one responsibility and a trait boundary, testable in
isolation.

### 3.1 `aerosieve-decode` (new)

Turns any supported container into canonical frames.

- **symphonia** 0.5 demux + decode (wav, mp3, flac, ogg, mp4, aac)
- **rubato** 0.16 sinc resample to **16 kHz mono f32**
- Loudness normalize to **-20 dBFS**, gain clamped to **+/-3 dB** (Emilia stage 1)
- Streams in blocks; never loads a whole file into memory

```rust
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub applied_gain_db: f32,
}
pub fn decode_file(path: &Path, cfg: &DecodeConfig) -> Result<DecodedAudio, DecodeError>;
```

### 3.2 `aerosieve-segment` (new)

Emits **variable-length speech segments**, not per-frame booleans. This is the
central correction: real VAD is a hysteresis state machine.

```rust
pub trait SpeechDetector {
    fn window_size(&self) -> usize;        // 512 @ 16 kHz for Silero
    fn probability(&mut self, window: &[f32]) -> f32;
    fn reset(&mut self);
}
```

Two implementations:
- `EnergyDetector` — pure DSP (dBFS + WADA-SNR), **default**, no ONNX
- `SileroDetector` — Silero VAD via `ort`, **feature `silero`**

State machine parameters (Silero documented defaults):

| Parameter | Value |
|---|---|
| `threshold` (enter speech) | 0.5 |
| `neg_threshold` (exit speech) | `max(threshold - 0.15, 0.01)` = 0.35 |
| `min_speech_duration_ms` | 250 |
| `min_silence_duration_ms` | 100 |
| `speech_pad_ms` | 30 |
| `min_segment_s` / `max_segment_s` | 3 / 30 |

Segments exceeding `max_segment_s` split at the longest interior silence.

### 3.3 `aerosieve-acoustic` (rework)

Deterministic quality scoring. Pure math, no models, no config that rejects
everything.

- **WADA-SNR** — Waveform Amplitude Distribution Analysis (Kim & Stern,
  Interspeech 2008). Models clean speech as Gamma(shape 0.4) and noise as
  Gaussian; estimates SNR from the amplitude distribution of a single buffer
  with no reference. Less bias and less variance across noise types than NIST
  STNR. Implemented as a table lookup over the G(x) statistic.
- **Clip ratio** — fraction of `|x| >= 0.999`
- **dBFS** — RMS level
- **Spectral flatness** — music/tone discrimination

```rust
pub struct QualityMetrics {
    pub wada_snr_db: f32,
    pub rms_dbfs: f32,
    pub clip_ratio: f32,
    pub spectral_flatness: f32,
}
pub fn score(samples: &[f32]) -> QualityMetrics;
```

Filtering is a separate, explicit policy over metrics — never hidden in the
scorer:

```rust
pub struct FilterPolicy {
    pub min_snr_db: f32,
    pub max_clip_ratio: f32,
    pub min_dbfs: f32,
    pub max_dbfs: f32,
    pub duration_iqr_k: f32,
}
```

Defaults **must accept clean speech**. This is enforced by test, not by comment.

Corpus-level **IQR outlier rejection** on character-duration (Emilia stage 6)
runs as a second pass once all segments are scored.

### 3.4 `aerosieve-shard` (new)

**WebDataset** tar shards — the standard training-data format. Sequential I/O is
3–10x faster than random access over millions of small files, and shards stream
directly from object storage.

- Samples grouped by basename: `{key}.wav` + `{key}.json`
- Shards roll at ~1 GB, named `shard-{:06}.tar`
- Atomic commit **at shard granularity**, reusing the v1 staging → `hard_link` →
  `rename` logic (this part of v1 is genuinely good and is kept)
- `manifest.jsonl` — one row per accepted sample with all quality metrics
- `stats.json` — corpus totals, retention rate, metric distributions

### 3.5 `aerosieve-lexical` (rework)

v1 applies rules as an ordered chain of `replace_all`, so rule *N* rewrites rule
*N-1* output and corruptions cascade. NeMo ITN work (arXiv 2104.05055) uses
hand-written WFST grammars precisely because the task has "extremely low
tolerance to unrecoverable errors."

v2 uses a **single-pass, leftmost-longest, non-overlapping** matcher: find all
candidate matches, resolve conflicts by (position, length, priority), apply once
to disjoint spans. No rule can see the output of another.

Full WFST is out of scope; single-pass disjoint application captures most of the
correctness win at a fraction of the complexity.

### 3.6 `aerosieve-ring` (keep)

Kept, repositioned as the decode-to-segment handoff. Cross-thread work
distribution uses **crossbeam-channel** bounded queues — bounded gives natural
backpressure; unbounded risks OOM.

## 4. Data flow

```
 for each input file (worker pool, crossbeam-bounded):
   decode  -> DecodedAudio
   segment -> Vec<Segment>
   for each segment:
     score -> QualityMetrics
     policy.accept(metrics)? -> normalize transcript -> shard writer
                              : record typed rejection reason
 corpus pass: IQR duration outlier rejection
 finalize: commit shards, write manifest.jsonl + stats.json
```

Every rejection records a typed reason. **Retention rate is the headline
metric** — Emilia reports 29.4%, and rejection is the product.

## 5. Error handling

- `thiserror` per crate; no `Box<dyn Error>` in library APIs
- **No silent fallbacks.** The v1 missing-rules-file path (`eprintln!` then an
  empty engine) is replaced by a hard error. A pipeline that silently stops
  normalizing is worse than one that fails.
- `WriteResult.committed` reports true commit status (v1 returns `false` on the
  successful rename path)
- Per-file failures are collected and reported, never abort the corpus run

## 6. Testing

| Layer | Approach |
|---|---|
| Fixtures | Deterministically generated `.wav`: clean sine, speech-like, clipped, silence, music-like, DC offset |
| **Regression** | **`SieveConfig::default()` accepts clean speech** — the bug that motivated v2 |
| Decode | Round-trip: known wav, decode, assert sample rate / channels / dBFS |
| Segment | `proptest` — segments non-overlapping, ordered, within duration bounds, total <= input |
| WADA-SNR | Synthesize known SNR (clean + scaled Gaussian), assert estimate within tolerance |
| Lexical | Overlapping-rule cases that demonstrably corrupt under the v1 chain |
| Shard | Write N, read back, byte-identical, manifest count matches |
| E2E | Fixture corpus through pipeline, assert retention rate and shard integrity |

## 7. Benchmarks

Re-baselined and **honest**:
- Per-stage throughput (decode / segment / score / shard) on realistic audio
- End-to-end **hours-of-audio per minute** — the Emilia unit, directly comparable
- **Retention rate** and metric distributions before/after
- Throughput "Total Frames" column corrected (v1 lists 1,000,000 for all worker
  counts; actual runs are 125k/250k/500k/1M)
- Python baseline re-measured (**14,115 fps**, not ~3,000) and its Windows
  `UnicodeEncodeError` crash fixed

Every number in the README must be reproducible by the command the README gives.

## 8. Build configuration

`ort` ships prebuilt ONNX Runtime binaries only for `x86_64-pc-windows-msvc`; a
GNU-toolchain build fails with *"no prebuilt binaries available for target
x86_64-pc-windows-gnu"* (verified by spike).

Therefore:
- **Default build** — pure DSP, `EnergyDetector`, no ONNX, any toolchain
- **`--features silero`** — ONNX VAD, requires MSVC toolchain, documented in README

This keeps `cargo build` working for anyone who clones, preserves the
deterministic-Rust identity as the default, and makes ML a genuine opt-in
upgrade rather than a hard dependency.

## 9. README corrections (part of the work, not a follow-up)

| Claim | Action |
|---|---|
| "files, microphones, or TCP sockets" | File ingestion is real; mic/TCP removed until built |
| "Hot-reloadable rules" | Removed — no implementing code exists |
| "`AudioChunk` carries `&[f32]` slices" | Corrected — it owns `Vec<f32>` |
| "zero-allocation hot paths" | Corrected to what is true: no per-frame audio copies, flat RSS |
| "hard_link -> rename -> copy" | Corrected — two tiers exist |
| Latency/throughput tables | Re-measured |
| Python baseline ~3,000 fps / ~110x | Corrected to 14,115 fps / ~25x |
| Basic Usage example | Must run and print the documented output; becomes a doc-test |

## 10. Out of scope

Source separation, speaker diarization, ASR generation, real-time streaming,
mic/TCP sources, full WFST grammars, GPU execution providers. Each is a
defensible future milestone; none is implied as present.

## 11. Success criteria

1. `cargo build` succeeds on the default (GNU) toolchain
2. `cargo test --workspace` passes, including the default-config regression test
3. `cargo run --example basic` prints the output the README documents
4. A fixture corpus produces valid WebDataset shards readable by a standard loader
5. Retention rate and quality metrics are reported per run
6. Every README number reproduces from the command the README gives
7. No claim in any README lacks implementing code
