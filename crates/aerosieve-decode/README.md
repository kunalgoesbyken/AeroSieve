# aerosieve-decode

Stage 1 of the AeroSieve v2 pipeline: **standardization**.

Takes an arbitrary audio file and produces canonical **16 kHz mono `f32`**
samples, loudness-normalized to **-20 dBFS**. This is the entry point of the
batch curation pipeline; every later stage (`aerosieve-segment`,
`aerosieve-acoustic`, `aerosieve-shard`) assumes its input is already in this
canonical form.

## What it does

1. **Demux and decode** with [symphonia](https://github.com/pdeljanov/Symphonia).
   The container is identified by probing the bytes; the file extension is
   passed as a hint only, so a mislabeled extension still decodes.
2. **Downmix to mono** by averaging all channels of each frame.
3. **Resample to 16 kHz** with [rubato](https://github.com/HEnquist/rubato)'s
   `SincFixedIn` (256-tap windowed sinc, cubic interpolation,
   Blackman-Harris² window). Skipped entirely when the source is already at the
   target rate, in which case samples pass through bit-exact.
4. **Normalize loudness** to -20 dBFS RMS with the applied gain clamped to
   ±3 dB.

## Supported formats

Whatever symphonia's `all` feature enables. In practice:

| Container | Codecs |
|---|---|
| WAV / RIFF | PCM (integer and float), ADPCM |
| MP3 | MPEG-1/2 Layer I/II/III |
| FLAC | FLAC |
| Ogg | Vorbis, FLAC |
| MP4 / M4A / ISO-MP4 | AAC, ALAC |
| MKV / WebM | contained codecs above |
| CAF | contained codecs above |

MP3, AAC, and ALAC decoding are symphonia's own pure-Rust implementations —
there is no FFmpeg or other C dependency anywhere in this crate.

## Loudness normalization

Emilia-Pipe ([arXiv 2407.05361](https://arxiv.org/abs/2407.05361)) — the
101k-hour, 6-language speech pipeline this architecture follows — normalizes
every source in its standardization stage to **-20 dBFS** with the gain
**clamped to ±3 dB**. Both numbers are reproduced here.

The clamp is the important part. An unbounded normalizer takes a quiet
recording and amplifies its noise floor along with the speech, and takes an
already-hot master and either drives it into clipping or crushes its dynamics.
Clamping to ±3 dB moves badly-leveled sources toward the target without doing
either. A source more than 3 dB from target simply ends up closer to it rather
than exactly on it, and the gain actually applied is reported in
`DecodedAudio::applied_gain_db` so downstream filtering can see what happened.

Level is measured as plain RMS, not ITU-R BS.1770 / LUFS. Digital silence is a
no-op returning 0 dB rather than an infinite boost.

## Usage

```rust
use std::path::Path;
use aerosieve_decode::{decode_file, DecodeConfig};

let audio = decode_file(Path::new("interview.mp3"), &DecodeConfig::default())?;

assert_eq!(audio.sample_rate, 16_000);
println!(
    "{:.2}s from {} Hz / {} ch, gain {:+.2} dB",
    audio.duration_s,
    audio.source_sample_rate,
    audio.source_channels,
    audio.applied_gain_db,
);
# Ok::<(), aerosieve_decode::DecodeError>(())
```

From an in-memory buffer, or any other `Read + Seek` source:

```rust
use aerosieve_decode::{decode_reader, DecodeConfig};

let bytes: Vec<u8> = std::fs::read("clip.flac")?;
let audio = decode_reader(
    std::io::Cursor::new(bytes),
    Some("flac"),           // optional hint
    &DecodeConfig::default(),
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Configuration

| Field | Default | Meaning |
|---|---|---|
| `target_sample_rate` | `16_000` | Output rate; everything is resampled to it |
| `target_dbfs` | `-20.0` | RMS loudness target |
| `max_gain_db` | `3.0` | Absolute clamp on the applied gain |
| `normalize` | `true` | When false, gain is 0 dB and samples are untouched |

## Duration preservation

Resampling is **sample-exact**: the output length is
`round(input_len × target_rate / source_rate)`, verified by test at 8 kHz,
22.05 kHz, 44.1 kHz, and 48 kHz sources.

This is not automatic. `SincFixedIn` consumes a fixed number of input frames
per call and its centered sinc filter introduces `output_delay()` frames of
latency, so a naive loop goes wrong in two independent ways:

- **Dropping the trailing partial chunk.** A file is rarely an exact multiple
  of the chunk size, so the remainder must be flushed through
  `process_partial_into_buffer`, which zero-pads it internally.
- **Leaving the filter delay in.** The first `output_delay()` output frames are
  the filter ramping up on zeros. They must be discarded, and an equal amount
  of extra output pulled out at the end, or the audio is offset and the real
  tail stays stuck inside the filter.

Measured against this crate's own implementation, skipping both steps yields
output **0.52% long at 44.1 kHz** and **5.03% long at 8 kHz with a completely
silent tail** — plausible-looking audio with the end missing. The tests assert
sample-exact length and check that both the first and last frames carry
signal, so either mistake fails the suite.

## Errors

`DecodeError` distinguishes `Io`, `UnsupportedFormat`, `NoAudioTrack`,
`MissingSampleRate`, `NoChannels`, `Decode`, `Resample`, and `InvalidConfig`.
Per symphonia's documented recovery pattern, malformed individual packets are
skipped and `ResetRequired` resets the decoder; only unrecoverable conditions
surface as an error. Missing files, non-audio files, zero-byte files, and
zero-length audio are all handled without panicking.

## Tests

```
cargo test -p aerosieve-decode
```

Fixtures are generated at runtime with `hound` into a self-cleaning temp
directory, so no binary audio is committed to the repository.
