---
layout: ../../layouts/Docs.astro
---

## What it does

AeroSieve turns raw audio recordings into training data for speech models. It decodes each file, cuts it into speech clips of 3 to 30 seconds, and scores every clip on noise, loudness, clipping and tonality. Clips that fail a check are dropped with a named reason; the rest are written as WebDataset shards.

It is written in Rust and runs on the CPU with no ML model by default. The same code ships as a command-line tool, a Rust library, Python bindings, a WebSocket server for live audio, and a WebAssembly build that powers the [browser demo](/#demo).

## Install

Pick the interface you need.

| You want | Install |
| :--- | :--- |
| Command line | `cargo install aerosieve-cli` (installs the `aerosieve` binary) |
| Rust library | `cargo add aerosieve` |
| Python | From a checkout of the repository: `pip install maturin`, then `maturin develop --release` inside `bindings/python`, in a virtual environment |
| Streaming server | From a checkout: `cargo run --release -p aerosieve-serve` |
| Browser | Nothing to install: [open the demo](/#demo). Your audio stays in the page. |

## Quickstart

Put some recordings in a directory. Any mix of wav, mp3, flac, ogg or mp4 works.

### 1. Inspect

See how the filter judges your audio, without writing anything.

```bash
aerosieve inspect recordings/
```

Each clip gets a line with its measurements and a verdict, followed by totals:

```text
recordings/call-0412.wav (33.6 s, 8000 Hz source)
    START  LENGTH   SNR dB     dBFS   CLIP  FLATNESS  VERDICT
     0.0s   23.4s     20.4    -23.7  0.000     0.149  accept
    23.4s   10.2s     18.1    -26.0  0.000     0.182  accept
```

If the verdicts look wrong for your recordings, read [How clips are judged](/docs/filtering) before going further.

### 2. Curate

Write the accepted clips to shards in a fresh directory.

```bash
aerosieve curate recordings/ --out data/run-1
```

The summary says how many clips were kept and why the others were not:

```text
5 files curated, 0 skipped
5 of 14 clips accepted (41.4% of detected speech)
  rejected as clipping   3
  rejected as tonal      3
  rejected as low_snr    3
0.8 minutes of audio in 1 shard(s) at data/run-1
```

### 3. Read the output

```text
data/run-1/
  shard-000000.tar    clips, each stored as {key}.wav and {key}.json
  manifest.jsonl      one line per clip: key, shard, transcript, metrics
  stats.json          totals
```

Open a shard with any WebDataset loader. `manifest.jsonl` lists every clip with its measurements, so you can filter further without opening the tars.

## Where to go next

- [Command line](/docs/cli): every command and flag, transcripts, rules and the output layout.
- [Python](/docs/python): `aerosieve.inspect` and `aerosieve.curate`.
- [Rust library](/docs/rust): `Pipeline`, `Inspector` and `LiveInspector`.
- [Streaming server](/docs/streaming): verdicts for live audio over a WebSocket.
- [How clips are judged](/docs/filtering): the measurements, every threshold, and how to choose them.
