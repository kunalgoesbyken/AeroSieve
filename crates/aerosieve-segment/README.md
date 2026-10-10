# aerosieve-segment

Voice-activity segmentation: cut decoded audio into variable-length speech clips.

```rust
use aerosieve_segment::{EnergyDetector, SegmentConfig, Segmenter};

let sample_rate = 16_000;
let config = SegmentConfig { sample_rate, ..Default::default() };
let mut segmenter = Segmenter::new(EnergyDetector::new(sample_rate), config);

for segment in segmenter.segment(&samples)? {
    let clip = &samples[segment.start_sample..segment.end_sample];
    println!("{:.2} s clip", segment.duration_s(sample_rate));
}
```

## Why a state machine

A per-window speech flag flickers: one quiet window mid-word splits an utterance, and one loud window in a pause emits a fragment. `Segmenter` runs the hysteresis used by Silero's `get_speech_timestamps` instead.

```
                 p >= threshold
     SILENCE  ---------------------->  SPEECH
        ^                                 |
        |   p < neg_threshold for at      |
        |   least min_silence_duration    |
        +---------------------------------+
```

The audio is scored in consecutive 32 ms windows. Then:

1. Speech **starts** at the first window scoring at least `threshold`.
2. It **continues** while windows score at least `neg_threshold`. The gap between the two thresholds is the hysteresis band.
3. It **ends** once windows have stayed below `neg_threshold` for `min_silence_duration_ms`. The segment ends where that silence began, so clips carry no trailing silence.
4. Shorter dips are **bridged**, which is what merges two bursts separated by a brief pause.
5. Runs shorter than `min_speech_duration_ms` are dropped.
6. Survivors are padded by `speech_pad_ms` on both sides. Padding never makes two segments overlap.
7. Segments longer than `max_segment_s` are cut in the middle of their longest bridged dip, or at exactly `max_segment_s` when there is none.
8. Segments shorter than `min_segment_s` are dropped.

The result is always ordered, non-overlapping, within the buffer, and no longer than `max_segment_s`. Property tests check this against a detector that returns random probabilities.

## Configuration

Defaults are Silero's, plus the 3–30 s clip bounds used by Emilia-Pipe.

| Field | Default | Meaning |
|---|---|---|
| `sample_rate` | `16000` | Rate of the audio being segmented |
| `threshold` | `0.5` | Probability at which speech starts |
| `neg_threshold` | `0.35` | Probability below which a window counts as silence |
| `min_speech_duration_ms` | `250` | Shorter speech runs are discarded |
| `min_silence_duration_ms` | `100` | Shorter dips are bridged |
| `speech_pad_ms` | `30` | Padding on both ends |
| `min_segment_s` | `3.0` | Shorter segments are dropped |
| `max_segment_s` | `30.0` | Longer segments are split; `0` disables splitting |

`SegmentConfig::with_threshold` derives `neg_threshold` the way Silero does: `max(threshold - 0.15, 0.01)`.

## Detectors

A detector implements `SpeechDetector`: it scores one window at a time and may carry state between windows. A detector error aborts segmentation rather than being read as silence.

### `EnergyDetector` (default)

Maps the window's RMS level through a logistic curve centred on a threshold (−45 dBFS by default; change it with `EnergyDetector::with_threshold_dbfs`). No model and no dependencies.

It measures energy, not speech: loud music or machinery scores as speech.

### `SileroDetector` (feature `silero`)

Silero VAD v5 through [`ort`](https://ort.pyke.io/) (ONNX Runtime). It supports 8 kHz and 16 kHz audio only.

```bash
curl -sL -o models/silero_vad.onnx \
  https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
cargo test -p aerosieve-segment --features silero
```

On Windows this feature needs the MSVC toolchain (`cargo +stable-x86_64-pc-windows-msvc ...`), because `ort` ships no prebuilt ONNX Runtime for `windows-gnu`.

Silero scores every synthetic signal as non-speech, so `tests/silero_inference.rs` verifies the model binding (input sensitivity, state threading, reset) rather than its decision boundary. Those tests pass vacuously when the model file is absent.
