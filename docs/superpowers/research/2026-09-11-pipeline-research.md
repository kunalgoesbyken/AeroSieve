# AeroSieve v2 — Research Notes (2026-09-11)

Gathered via WebSearch/WebFetch. Purpose: ground a redesign of the ingestion
pipeline in real architectures instead of invented heuristics.

## 1. Reference architecture: Emilia-Pipe (arXiv 2407.05361 / 2501.15907)

The canonical open pipeline for turning in-the-wild audio into speech-generation
training data. 101k+ hours, 6 languages. Six ordered stages:

| # | Stage | Model / method |
|---|-------|----------------|
| 1 | Standardization | WAV, mono, 16-bit, 24 kHz; normalize to −20 dBFS, gain clipped ±3 dB |
| 2 | Source separation | UVR-MDX-NET-Inst_HQ_3 (ONNX) — strip music/background |
| 3 | Speaker diarization | pyannote speaker-diarization-3.1 |
| 4 | Fine-grained segmentation | Silero VAD → 3–30 s single-speaker clips |
| 5 | ASR | WhisperX (faster-whisper + CTranslate2) |
| 6 | Filtering | DNSMOS P.835 OVRL > 3.0; language ID conf > 80%; IQR outlier rejection on char/phone duration |

Reported results:
- Throughput **2.5 raw hours/minute on 8× RTX 4090**
- **Retention ~29.4%** of raw audio after filtering
- DNSMOS improved **2.50 → 3.26** through the pipeline

**Key lesson:** the ordering is load-bearing (cheap gates before expensive ones),
and *rejection is the product* — ~70% of input is supposed to be discarded.

## 2. VAD: deterministic RMS/SNR is not competitive

Benchmarks on noisy audio (UrbanSound8K):
- WebRTC VAD **58%**
- pyannote VAD **62%**
- **Silero VAD 87%**

WebRTC is better at detecting *silence* than *speech* and has many false positives.
Silero is geared at speech vs. noise/music.

Silero VAD specifics (github.com/snakers4/silero-vad, MIT license):
- **Required chunk: exactly 512 samples @ 16 kHz** (256 @ 8 kHz) — 32 ms
- **< 1 ms per chunk on a single CPU thread**
- JIT model ~2 MB; ONNX available, "up to 4–5x faster" under some conditions
- Trained on 6000+ languages
- Defaults (`get_speech_timestamps`): `threshold=0.5`,
  `min_speech_duration_ms=250`, `min_silence_duration_ms=100`,
  `speech_pad_ms=30`, `max_speech_duration_s=inf`,
  `neg_threshold = max(threshold-0.15, 0.01)`

**Implication for AeroSieve:** our 320-sample frame is the wrong unit. Silero
needs 512. And a *hysteresis state machine* (enter/exit thresholds + min
durations) is the real algorithm — not a per-frame boolean.

## 3. Quality estimation: MOS predictors, not raw RMS

- **DNSMOS P.835** (arXiv 2110.01763): CNN on log-power spectrograms, outputs
  SIG / BAK / OVRL. PCC vs human ratings: 0.94 SIG, 0.98 BAK, 0.98 OVRL.
  This is what Emilia filters on (OVRL > 3.0).
- **NISQA**: CNN + self-attention, multi-dimensional.
- **UTMOS**: naturalness-oriented.
- Caveat from ICASSP 2023 Speech Signal Improvement Challenge: correlation of
  DNSMOS/NISQA with subjective P.804 ranges PCC 0.478–0.700 — useful as a
  *filter*, not as ground truth.

SNR estimation (non-intrusive):
- **WADA-SNR** (Interspeech 2008): assumes clean speech ~ Gamma(shape 0.4),
  noise Gaussian; estimates SNR from amplitude distribution. Significantly less
  bias and less variance across noise types than **NIST STNR**. Tends to
  *underestimate*; NIST tends to call noise clean.

**Implication:** AeroSieve's current "SNR" (frame RMS minus its own leading
window RMS) is not an SNR estimator at all. WADA-SNR is the cheap, deterministic,
pure-math upgrade that stays in Rust with no model.

## 4. Text normalization: WFST, not regex chains

- **NeMo ITN** (arXiv 2104.05055, Interspeech 2021): production systems use
  hand-written **WFST grammars** because the task has "extremely low tolerance
  to unrecoverable errors." Compiled with **Pynini** over OpenFst.
- NeMo-text-processing supports English primarily; modular/extensible.
- No mature open code-switched Hindi-English ITN found.

**Implication:** ordered regex `replace_all` chains (current design) are
order-dependent and can cascade corruptions — rule N rewrites rule N-1's output.
A single-pass longest-match transducer avoids this. Full WFST may be overkill;
a single-pass, non-overlapping, leftmost-longest matcher captures most of the win.

## 5. Streaming / real-time architecture

Sequential pipeline `Audio In → VAD → STT → LLM → TTS → Audio Out` is the
production pattern (LiveKit, Pipecat) — modular, independently testable.
- Good VAD adds **~10–50 ms** latency.
- End-to-end voice agent target **~600–800 ms**.
- Turn detection: VAD-only (simple, higher latency) < STT endpointing (best
  default) < model-based semantic (lowest latency).

**Implication:** a *batch curation* pipeline and a *real-time streaming* pipeline
have different latency budgets and different correct designs. AeroSieve currently
claims the real-time framing but benchmarks a batch workload.

## 6. Rust ecosystem (verified crates)

| Need | Crate | Notes |
|------|-------|-------|
| Decode mp3/flac/wav/ogg/mp4 | **symphonia** | pure Rust, 3.2M+ downloads, within ±15% of FFmpeg perf |
| Resample to 16 kHz | **rubato** | sinc/band-limited, chunked, SIMD auto-vectorization, sync + async |
| ONNX inference (Silero) | **ort** 2.0 (pykeio) | "production-ready (just not API stable)", rc.13 |
| Channels / backpressure | **crossbeam-channel** | production workhorse; bounded = natural backpressure, unbounded risks OOM |

## 7. Storage format

- **WebDataset** (tar shards): sequential I/O, 3–10× higher I/O than random
  access, streams from object stores, native PyTorch DataLoader. Recommended for
  audio >1 MB/file. Downside: updating one sample rewrites the shard.
- **Parquet**: good for small audio (<1 MB) w/ small row groups (~100 rows).
- **Lance**: columnar metadata + blob bytes; attractive at scale for the
  metadata query layer.

**Implication:** per-sample `{uuid}.raw` + `{uuid}.txt` (current design) is the
worst case for training I/O — millions of tiny files, random access. Sharded
output is a real, defensible improvement.

## 8. Dedup

MinHash + LSH over shingles is the standard for near-duplicate removal;
deduplication measurably improves LM training and prevents eval contamination.
Generalizes from text to audio feature representations.

## Sources
- Emilia: https://arxiv.org/pdf/2407.05361 , https://arxiv.org/pdf/2501.15907
- Emilia-Pipe impl: https://github.com/open-mmlab/Amphion/blob/main/preprocessors/Emilia/README.md
- Silero VAD: https://github.com/snakers4/silero-vad
- DNSMOS P.835: https://arxiv.org/pdf/2110.01763
- WADA-SNR: https://www.isca-archive.org/interspeech_2008/kim08e_interspeech.html
- NeMo ITN: https://arxiv.org/pdf/2104.05055
- VAD benchmark: https://github.com/Anwarvic/VAD_Benchmark
- LiveKit turn detection: https://livekit.com/blog/turn-detection-voice-agents-vad-endpointing-model-based-detection
- symphonia: https://github.com/pdeljanov/Symphonia
- rubato: https://github.com/HEnquist/rubato
- ort: https://ort.pyke.io/
- WebDataset: https://github.com/webdataset/webdataset
