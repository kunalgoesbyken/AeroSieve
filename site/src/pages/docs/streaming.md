---
layout: ../../layouts/Docs.astro
---

## What it is

`aerosieve-serve` is a streaming speech-quality gate. A client streams audio over a WebSocket and gets a verdict for each utterance as soon as the pause that ends it is heard: kept, or rejected with a named reason.

The verdicts are the ones batch curation reaches on the same audio. The live path is the batch segmenter fed incrementally, and tests stream audio in random chunk sizes and check that the clips and scores are identical.

## Run it

From a checkout of the repository:

```bash
cargo run --release -p aerosieve-serve -- --addr 127.0.0.1:8080
python crates/aerosieve-serve/clients/stream_wav.py recording.wav
```

The client streams the file at real-time pace and prints each verdict. This is the repository's `mixed.ogg` demo recording, converted to 16 kHz WAV: clean speech, then the same speaker under noise, clipped, and over a dial tone.

```text
   0.51s   4.92s  accept      snr  15.5 dB  decided at 5.92s
   7.30s   9.12s  low_snr     snr   1.7 dB  decided at 16.90s
  17.83s   9.31s  clipping    snr  36.6 dB  decided at 27.62s
  28.55s   5.12s  tonal       snr   3.1 dB  decided at 34.16s
  35.17s  18.24s  accept      snr  21.3 dB  decided at 53.90s
  55.07s  12.64s  accept      snr  30.0 dB  decided at 68.20s
kept 3 of 6 clips from 70.6s
```

The server judges with the default thresholds. The binary has no flag for them.

## Protocol

Connect to `/v1/stream`. Every server message is one JSON text frame.

| Direction | Message | Meaning |
| :--- | :--- | :--- |
| server | `{"type":"ready","sample_rate":16000,"encoding":"pcm_s16le","max_utterance_s":120.0}` | Sent first. Stream audio in this format. |
| client | binary frame | Mono 16-bit little-endian PCM, any length that holds whole samples. 20 ms frames are typical. |
| server | `{"type":"clip","decided_at_s":5.92,"start_s":0.514,"duration_s":4.924,"wada_snr_db":15.47,"rms_dbfs":-19.14,"clip_ratio":0.0,"spectral_flatness":0.093,"verdict":"accept"}` | A clip ended and was judged. `decided_at_s` is how much audio had arrived when it was. |
| client | `{"type":"end"}` | No more audio. |
| server | `{"type":"summary","heard_s":70.6,"clips":6,"accepted":3}` | Pending clips have been judged. The server closes. |
| server | `{"type":"error","message":"..."}` | A limit was broken or a frame was malformed. The server closes. |

`verdict` is `accept` or one of `silence`, `too_quiet`, `too_loud`, `clipping`, `tonal` and `low_snr`. The measurements are the four described in [How clips are judged](/docs/filtering).

Things a client must know:

- The audio format is whatever `ready` announces. A binary frame that does not hold whole samples is an error.
- A text frame must be a control message. Only `end` exists.
- Audio is judged as received, with no loudness leveling. A microphone set very low is rejected as `too_quiet` rather than turned up.
- A verdict cannot arrive before the segmenter has heard 500 ms of pause after the speech. That hold is `min_silence_duration_ms` in the segmentation settings.

## Limits

| Flag | Default | Why |
| :--- | ---: | :--- |
| `--addr` | `127.0.0.1:8080` | Address to listen on. |
| `--max-sessions` | 256 | Further connections get HTTP 503 before the upgrade, so clients can back off. |
| `--max-utterance-s` | 120 | Audio is held only for the utterance in progress, so memory per session is bounded by how long a speaker goes without pausing. A stream that never pauses, like a test tone, is closed once it passes this. |
| `--idle-timeout-s` | 30 | Sessions that stop sending are closed. |

WebSocket messages over 1 MiB are refused.

## Metrics and health

`/metrics` serves Prometheus counters and one histogram:

| Metric | Type | Meaning |
| :--- | :--- | :--- |
| `aerosieve_sessions_active` | gauge | Sessions open now. |
| `aerosieve_sessions_total` | counter | Sessions started. |
| `aerosieve_sessions_refused_total` | counter | Connections refused at the session limit. |
| `aerosieve_audio_seconds_total` | counter | Audio received. |
| `aerosieve_clips_total{verdict="..."}` | counter | Clips judged, by verdict. |
| `aerosieve_decision_seconds` | histogram | Time from receiving the frame that completed a clip to sending its verdict. |

`/healthz` answers `ok`.

## Python client

The repository's client is a short script. It needs `websockets`.

```bash
pip install websockets
python crates/aerosieve-serve/clients/stream_wav.py recording.wav --url ws://127.0.0.1:8080/v1/stream
```

Pass `--fast` to send as quickly as possible instead of at real-time pace. The file must be mono 16-bit PCM at the rate the server announces.

The core of any client is the same, in any language:

```python
import asyncio
import json

import websockets


async def judge(pcm: bytes, url: str = "ws://127.0.0.1:8080/v1/stream") -> None:
    async with websockets.connect(url) as socket:
        ready = json.loads(await socket.recv())
        frame_bytes = ready["sample_rate"] * 20 // 1000 * 2

        async def send() -> None:
            for offset in range(0, len(pcm), frame_bytes):
                await socket.send(pcm[offset : offset + frame_bytes])
                await asyncio.sleep(0.02)
            await socket.send(json.dumps({"type": "end"}))

        sender = asyncio.create_task(send())
        async for message in socket:
            event = json.loads(message)
            if event["type"] == "clip":
                print(event["start_s"], event["duration_s"], event["verdict"])
            elif event["type"] == "summary":
                break
            else:
                raise RuntimeError(event.get("message", event))
        await sender
```

`pcm` is 16 kHz mono 16-bit little-endian audio, for example the frames of a WAV file.

## Latency

Measured on one laptop (Core i5-9300H, 4 cores, 8 threads) with the load generator on the same machine. Each client replays the 72 s demo recording, six utterances, in 20 ms frames at real-time pace. Latency runs from sending the frame that completed a clip to receiving its verdict, after the 500 ms hold.

| Live streams | Started | Verdicts | p50 (ms) | p95 (ms) | p99 (ms) | Failed |
| ---: | :--- | ---: | ---: | ---: | ---: | ---: |
| 1 | staggered over 10 s | 6 | 1.7 | 3.1 | 3.1 | 0 |
| 100 | staggered over 10 s | 600 | 2.2 | 3.8 | 4.1 | 0 |
| 1,000 | staggered over 10 s | 6,000 | 6.5 | 14.8 | 19.2 | 0 |
| 100 | all at once | 600 | 12.5 | 34.4 | 43.8 | 0 |
| 1,000 | all at once | 6,000 | 29.9 | 117.2 | 182.0 | 0 |

"All at once" is a deliberate worst case: identical replays started together end every utterance in the same instant, so a thousand clips are scored at once. These are numbers from one machine with the load generator competing for the same cores, not a capacity guarantee.

To reproduce:

```bash
cargo build --release -p aerosieve-serve --bins
./target/release/aerosieve-serve --max-sessions 2048 &
./target/release/aerosieve-loadgen site/public/samples/mixed.ogg --streams 1000 --ramp-s 10
```

In the Rust library the same path is [`LiveInspector`](/docs/rust#liveinspector).

