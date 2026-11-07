# aerosieve-serve

A streaming speech-quality gate. Clients stream audio over a WebSocket and get a verdict for each utterance as soon as the pause that ends it is heard: kept, or rejected as clipped, too quiet, tonal, silent, or too noisy.

The verdicts are the ones batch curation reaches on the same audio. The live path is the batch segmenter fed incrementally, and tests check that streaming in any chunk size gives identical clips and identical scores.

## Run it

```sh
cargo run --release -p aerosieve-serve -- --addr 127.0.0.1:8080
python crates/aerosieve-serve/clients/stream_wav.py recording.wav
```

```text
   0.51s   4.92s  accept      snr  15.5 dB  decided at 5.92s
   7.30s   9.12s  low_snr     snr   1.7 dB  decided at 16.90s
  17.83s   9.31s  clipping    snr  36.6 dB  decided at 27.62s
  28.55s   5.12s  tonal       snr   3.1 dB  decided at 34.16s
  35.17s  18.24s  accept      snr  21.3 dB  decided at 53.90s
  55.07s  12.64s  accept      snr  30.0 dB  decided at 68.20s
kept 3 of 6 clips from 70.6s
```

That is `site/samples/mixed.ogg`, converted to 16 kHz WAV: clean speech, then the same speaker under noise, clipped, and over a dial tone.

## Protocol

Connect to `/v1/stream`. Every server message is one JSON text frame.

| Direction | Message | Meaning |
| :--- | :--- | :--- |
| server | `{"type":"ready","sample_rate":16000,"encoding":"pcm_s16le","max_utterance_s":120.0}` | Sent first. Stream audio in this format. |
| client | binary frame | Mono 16-bit little-endian PCM, any length that holds whole samples. 20 ms frames are typical. |
| server | `{"type":"clip","decided_at_s":5.92,"start_s":0.514,"duration_s":4.924,"wada_snr_db":15.47,"rms_dbfs":-19.14,"clip_ratio":0.0,"spectral_flatness":0.093,"verdict":"accept"}` | A clip ended and was judged. `decided_at_s` is how much audio had arrived when it was. |
| client | `{"type":"end"}` | No more audio. |
| server | `{"type":"summary","heard_s":70.6,"clips":6,"accepted":3}` | Pending clips have been judged; the server closes. |
| server | `{"type":"error","message":"..."}` | A limit was broken or a frame was malformed; the server closes. |

`verdict` is `accept` or one of `silence`, `too_quiet`, `clipping`, `tonal`, `low_snr`.

`/metrics` serves Prometheus counters (sessions, refusals, audio seconds, clips by verdict) and `aerosieve_decision_seconds`, a histogram of the time from receiving the frame that completed a clip to sending its verdict. `/healthz` answers `ok`.

## Limits

| Flag | Default | Why |
| :--- | ---: | :--- |
| `--max-sessions` | 256 | Further connections get HTTP 503 before the upgrade, so clients can back off. |
| `--max-utterance-s` | 120 | Audio is held only for the utterance in progress, so memory per session is bounded by how long a speaker goes without pausing. A stream that never pauses, like a test tone, is closed once it passes this. |
| `--idle-timeout-s` | 30 | Sessions that stop sending are closed. |

Frames over 1 MiB are refused. Audio is judged as received, with no loudness leveling, so a microphone set very low is rejected as `too_quiet` rather than turned up.

## Latency

Measured on one laptop (Core i5-9300H, 4 cores, 8 threads) with the load generator on the same machine, each client replaying `site/samples/mixed.ogg` (72 s, six utterances: three kept, one each clipped, noisy and over a dial tone) in 20 ms frames at real-time pace. Latency runs from sending the frame that completed a clip to receiving its verdict.

| Live streams | Started | Verdicts | p50 | p95 | p99 | Failed |
| ---: | :--- | ---: | ---: | ---: | ---: | ---: |
| 1 | staggered over 10 s | 6 | 1.7 ms | 3.1 ms | 3.1 ms | 0 |
| 100 | staggered over 10 s | 600 | 2.2 ms | 3.8 ms | 4.1 ms | 0 |
| 1,000 | staggered over 10 s | 6,000 | 6.5 ms | 14.8 ms | 19.2 ms | 0 |
| 100 | all at once | 600 | 12.5 ms | 34.4 ms | 43.8 ms | 0 |
| 1,000 | all at once | 6,000 | 29.9 ms | 117.2 ms | 182.0 ms | 0 |

"All at once" is a deliberate worst case: identical replays started together end every utterance in the same instant, so a thousand clips are scored at once. Unpaced, 16 clients stream 1.27 hours of audio through the server in 0.60 s, 7,650 times faster than real time. Raw reports are in [`bench`](bench).

A verdict cannot arrive before the segmenter has heard 500 ms of pause after the speech: that hold is the `min_silence_duration_ms` of `SegmentConfig`, and lowering it trades earlier verdicts for clips cut at word gaps. Everything after the hold is measured above.

Reproduce with:

```sh
cargo build --release -p aerosieve-serve --bins
./target/release/aerosieve-serve --max-sessions 2048 &
./target/release/aerosieve-loadgen site/samples/mixed.ogg --streams 1000 --ramp-s 10
```
