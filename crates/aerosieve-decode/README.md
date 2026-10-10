# aerosieve-decode

Standardization: any audio file in, mono `f32` at one sample rate and one loudness out.

1. **Demux and decode** with [symphonia](https://github.com/pdeljanov/Symphonia). The container is identified by probing its bytes; the file extension is only a hint.
2. **Downmix** by averaging channels.
3. **Resample** with [rubato](https://github.com/HEnquist/rubato)'s `SincFixedIn`. Skipped when the source is already at the target rate.
4. **Normalize** RMS level toward the target, with the applied gain clamped.

```rust
use std::path::Path;
use aerosieve_decode::{decode_file, DecodeConfig};

let audio = decode_file(Path::new("interview.mp3"), &DecodeConfig::default())?;
println!(
    "{:.2}s from {} Hz / {} ch, gain {:+.2} dB",
    audio.duration_s(),
    audio.source_sample_rate,
    audio.source_channels,
    audio.applied_gain_db,
);
```

`decode_reader` does the same for any `Read + Seek` source, such as an in-memory buffer.

## Configuration

| Field | Default | Meaning |
|---|---|---|
| `target_sample_rate` | `16_000` | Output rate |
| `target_dbfs` | `-20.0` | RMS loudness target |
| `max_gain_db` | `3.0` | Largest gain applied in either direction |
| `normalize` | `true` | When false, samples are left at their decoded level |

The loudness defaults follow stage 1 of Emilia-Pipe ([arXiv 2407.05361](https://arxiv.org/abs/2407.05361)). The clamp matters: an unbounded normalizer amplifies a quiet recording's noise floor along with its speech. Level is plain RMS, not LUFS.

## Formats

Whatever symphonia's `all` feature enables: WAV, MP3, FLAC, Ogg Vorbis, MP4/M4A (AAC, ALAC), MKV/WebM, and CAF. All decoders are pure Rust.

## Duration is preserved exactly

Resampled output is `round(input_len × target_rate / source_rate)` samples long. That takes two deliberate steps, both covered by tests:

- The trailing partial chunk is flushed through `process_partial_into_buffer`; otherwise the end of the file is dropped.
- The filter's `output_delay()` frames of ramp-up are discarded from the front and the same amount is flushed from the back; otherwise the audio is offset and its tail is lost.

## Errors

Malformed packets are skipped, as symphonia recommends. A file that cannot be opened, is not audio, or has no usable track returns a `DecodeError`.
