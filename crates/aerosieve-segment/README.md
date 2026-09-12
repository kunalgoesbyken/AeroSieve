# aerosieve-segment

Hysteresis VAD segmentation: **variable-length speech clips, not per-frame booleans**.

Stage 2 of the AeroSieve v2 batch curation pipeline. Takes a decoded 16 kHz mono
buffer from `aerosieve-decode` and emits `Segment`s of speech bounded to
`[min_segment_s, max_segment_s]`, ready for `aerosieve-acoustic` to score.

```
decode  ->  [ segment ]  ->  score  ->  shard
            hysteresis
            3-30 s clips
```

## Why a state machine

AeroSieve v1 applied a per-frame accept/reject test. That flickers: one quiet
window in the middle of a word splits an utterance in two, and one loud window
in a pause emits a fragment. Real VAD — Silero's `get_speech_timestamps`, WebRTC,
pyannote — is a **hysteresis state machine**: a high threshold to *enter* speech,
a lower one to *leave* it, and a minimum silence duration before a dip counts as
an ending.

This crate implements those semantics, following Silero VAD's documented defaults
so behaviour is comparable whether you run the pure-DSP detector or the ONNX one.

## Algorithm

The signal is walked in non-overlapping windows of `detector.window_size()`
samples (512 at 16 kHz, matching Silero). For each window the detector returns a
probability in `0.0..=1.0`, and the machine runs:

```
                 p >= threshold
     SILENCE  ---------------------->  SPEECH
        ^                                 |
        |   sub-neg_threshold run of      |
        |   >= min_silence_duration_ms    |
        +---------------------------------+
             (segment ends where the
              silence run BEGAN)
```

1. **Enter speech** at the first window with `p >= threshold` (0.5). The segment
   starts at that window's first sample.
2. **Stay in speech** while `p >= neg_threshold` (0.35). The band between 0.35
   and 0.5 is the hysteresis: it takes a clear signal to start speech, but only a
   clear absence to stop it.
3. **Exit speech** when `p < neg_threshold` for a run lasting at least
   `min_silence_duration_ms` (100 ms). Crucially the segment ends at the **start**
   of that silence run, not at its end — otherwise every clip would carry 100 ms
   of trailing silence.
4. **Absorb short dips.** A sub-threshold run shorter than
   `min_silence_duration_ms` does not end the segment. This is what merges two
   bursts separated by a brief pause into one clip — the payoff of hysteresis over
   per-frame decisions. Absorbed dips are remembered as candidate split points.
5. **Drop short runs.** Raw runs shorter than `min_speech_duration_ms` (250 ms)
   are discarded as spurious.
6. **Pad.** Survivors are extended by `speech_pad_ms` (30 ms) on both sides,
   clamped to `[0, samples.len()]` — a segment starting at sample 0 does not
   underflow, and one ending at the last sample does not run past the buffer.
7. **Split long segments.** See below.
8. **Drop short segments.** Anything below `min_segment_s` (3 s) after padding and
   splitting is dropped.
9. **Flush at end of input.** An in-progress segment is closed at the last full
   window; if it was already inside a sub-threshold dip, the segment ends where
   that dip began.

### Splitting segments over `max_segment_s`

A segment longer than `max_segment_s` (30 s) is split recursively:

- **Preferred cut**: the **midpoint of the longest interior silence run** that
  falls strictly inside the segment — one of the dips absorbed in step 4. This is
  the natural place to break a long utterance, and it puts the cut in the middle
  of the pause rather than clipping a word.
- **Fallback**: when no usable interior silence exists (a 60 s continuous tone,
  say), a hard cut at exactly `max_segment_s` samples.

After a cut the silence list is partitioned between the two halves and each half
is re-examined, so a very long segment with several dips is split at each in turn.
Pieces that then fall below `min_segment_s` are dropped by step 8.

## Parameters

`SegmentConfig::default()` uses Silero VAD's documented `get_speech_timestamps`
defaults plus the Emilia-Pipe clip-length bounds.

| Parameter | Default | Meaning |
|---|---|---|
| `sample_rate` | `16000` | Input rate in Hz; converts every `_ms` / `_s` field to samples |
| `threshold` | `0.5` | Probability at or above which speech is **entered** |
| `neg_threshold` | `0.35` | Probability below which speech may be **exited**. Silero derives this as `max(threshold - 0.15, 0.01)` — see `SegmentConfig::with_threshold` |
| `min_speech_duration_ms` | `250` | Raw speech runs shorter than this are discarded |
| `min_silence_duration_ms` | `100` | A sub-threshold run must last this long to end a segment; shorter dips are absorbed |
| `speech_pad_ms` | `30` | Padding added to both ends of each surviving segment |
| `min_segment_s` | `3.0` | Final segments shorter than this are dropped (Emilia) |
| `max_segment_s` | `30.0` | Final segments longer than this are split (Emilia) |

## Detectors

Both implement `SpeechDetector`, so the state machine is identical either way:

```rust
pub trait SpeechDetector {
    fn window_size(&self) -> usize;
    fn probability(&mut self, window: &[f32]) -> f32;
    fn reset(&mut self);
}
```

### `EnergyDetector` — the default

Pure DSP, no models, builds on any toolchain. It computes the window's RMS level
in dBFS (via `aerosieve_acoustic::rms_dbfs`) and maps it through a logistic curve
centred on a configurable threshold:

```
p = 1 / (1 + exp(-(dbfs - threshold_dbfs) / 4.0))
```

| `dbfs - threshold_dbfs` | `p` |
|---|---|
| −12 dB | 0.047 |
| −6 dB | 0.18 |
| 0 dB | 0.50 |
| +6 dB | 0.82 |
| +12 dB | 0.95 |

The 4 dB softness makes the transition across the 0.35/0.50 hysteresis band span
roughly 2.5 dB — wide enough to tolerate a single quiet window, narrow enough that
voiced audio saturates near 1.0 and true silence collapses to 0.0 (digital silence
gives `-inf` dBFS, hence exactly 0.0).

The default midpoint is **−45 dBFS**, far below the pipeline's −20 dBFS
normalisation target, so normalised speech reads ≈1.0 while noise-floor-only
passages read ≈0.0. Override with `EnergyDetector::with_threshold_dbfs`.

**Caveat:** this is an *energy* detector, not a speech/noise classifier. Loud
non-speech — music, machinery, applause — reads as speech. The research baseline
puts deterministic energy VAD well below Silero on noisy material (WebRTC 58% vs
Silero 87% on UrbanSound8K). Enable `silero` when that distinction matters.

Window size is 512 samples at 16 kHz (256 at 8 kHz), matching Silero so the two
detectors produce comparable boundaries.

### `SileroDetector` — opt-in, feature `silero`

Silero VAD v5 through `ort` (ONNX Runtime). Accepts **exactly** 512 samples at
16 kHz or 256 at 8 kHz, carries the LSTM state `[2, 1, 128]` between calls, and
runs in under 1 ms per chunk on one CPU thread. `reset()` zeroes the state.

```rust
let detector = SileroDetector::from_model_path(Path::new("silero_vad.onnx"), 16_000)?;
```

> **`silero` is opt-in and does not build on the windows-gnu toolchain.**
>
> `ort` ships prebuilt ONNX Runtime binaries only for `x86_64-pc-windows-msvc`.
> On a GNU-toolchain Windows host `cargo build --features silero` fails with
> *"no prebuilt binaries available for target x86_64-pc-windows-gnu"*. Building
> this feature on Windows requires the **MSVC toolchain**
> (`rustup default stable-x86_64-pc-windows-msvc`) — or supplying your own
> ONNX Runtime via `ORT_LIB_LOCATION`. Linux and macOS are unaffected.
>
> The crate's default features are empty precisely so `cargo build` works for
> anyone who clones the repo. ML is a genuine opt-in upgrade, not a hard
> dependency.

**Verification status:** compiled and run against the real model on
`stable-x86_64-pc-windows-msvc`. `tests/silero_inference.rs` holds 8 live
inference tests — model loading, sample-rate validation, probability bounds,
input sensitivity, recurrent-state threading, and reset semantics. They skip
themselves when `models/silero_vad.onnx` is absent, so a checkout without the
weights still passes.

```bash
curl -sL -o models/silero_vad.onnx \
  https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
cargo +stable-x86_64-pc-windows-msvc test -p aerosieve-segment --features silero
```

**Silero and synthetic audio.** Measured against the real model, every synthetic
signal scores ~0.001: DC, white noise, a 200 Hz sine, and a synthetic vowel are
all "not speech". That is the model behaving correctly — it was trained to
separate real speech from everything else — but it means synthetic fixtures
cannot exercise its decision boundary. The live tests therefore assert what *is*
verifiable without a recording: probabilities vary with input (a broken tensor
binding would return a constant), and the `stateN` output is genuinely fed back
as the next call's `state`. Use `EnergyDetector` for tests on generated audio.

## Usage

```rust
use aerosieve_segment::{EnergyDetector, SegmentConfig, Segmenter};

let sample_rate = 16_000;
let config = SegmentConfig { sample_rate, ..Default::default() };
let mut segmenter = Segmenter::new(EnergyDetector::new(sample_rate), config);

for segment in segmenter.segment(&samples) {
    let clip = &samples[segment.start_sample..segment.end_sample];
    println!("{:.2} s clip", segment.duration_s(sample_rate));
    // -> aerosieve_acoustic::score(clip)
}
```

`segment` resets the detector first, so repeated calls are independent and
idempotent.

## Guarantees

Every returned `Vec<Segment>` satisfies, for any detector and any input:

- ordered by `start_sample`
- non-overlapping
- `end_sample > start_sample`
- `end_sample <= samples.len()`
- `len_samples() <= max_segment_s * sample_rate`

These are enforced by `proptest` against a mock detector driven by a
deterministic pseudo-random probability sequence, so the invariants are checked
independently of whether the detector behaves sensibly.

## Errors

`SegmentError` covers only the optional ONNX path — model not found, unsupported
sample rate, ONNX Runtime failure. Segmenting an in-memory buffer is infallible.

## Tests

```sh
cargo test -p aerosieve-segment          # default features, no ONNX
```

Covers: silence-only, single burst bounds, two bursts split by a long gap, two
bursts merged across a short gap, sub-`min_speech_duration_ms` rejection, 60 s
burst splitting, split-at-interior-silence, padding applied and clamped at both
edges, empty input, sub-window input, plus the property tests above.

## References

- Silero VAD — <https://github.com/snakers4/silero-vad> (defaults, 512-sample constraint)
- Emilia / Emilia-Pipe — [arXiv 2407.05361](https://arxiv.org/pdf/2407.05361) (3–30 s clips, stage ordering)
- VAD benchmark — <https://github.com/Anwarvic/VAD_Benchmark>
- `ort` — <https://ort.pyke.io/>
