# Models

Model weights are **not committed** — they are fetched on demand.

## Silero VAD

Required by the optional `silero` feature. MIT licensed, ~2.3 MB.

```bash
curl -sL -o models/silero_vad.onnx \
  https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
```

Source: https://github.com/snakers4/silero-vad

The live inference tests in `crates/aerosieve-segment/tests/silero_inference.rs`
skip themselves if this file is absent, so a checkout without it still passes.
