# Changelog

## 0.4.0 (2026-11-18)

### Added

- `aerosieve-serve`: a WebSocket quality gate that returns each utterance's verdict as soon as its closing pause is heard. Session, utterance, frame and idle limits, Prometheus metrics, a load generator and a Python client.
- `StreamSegmenter` and `LiveInspector` judge audio that arrives in pieces, with results identical to batch inspection for any chunking.
- `Inspector::score_whole` and Python's `inspect(..., whole=True)` score a pre-cut utterance as one clip, so it stays matched to its transcript. Segmenting it instead drops anything under 3 s.
- `Pipeline::process_files` curates on every core with output identical to a sequential run. `aerosieve curate` and Python's `curate` use it.
- SieveBench (`aerosieve-sievebench`) injects known defects into clean speech in six languages and reports how many each filter catches.
- `experiments/asr-hi`: on 2,917 Hindi phone utterances with human-verified transcripts, rejected audio has 8.6 to 8.9 points higher WER for Whisper-large-v3 and MMS-1B.
- WebAssembly bindings and a live in-browser demo at https://aerosieve.vercel.app.

### Changed

- `aerosieve_acoustic::score` and `clip_ratio` take the level the source's full scale sits at; pass `FULL_SCALE` for untouched audio.
- Scoring is 4.7 times faster with identical results: a planned real FFT for flatness, and one logarithm per 512 samples for WADA-SNR.
- `Segmenter::segment` runs the streaming segmenter over the whole buffer, so batch and live results cannot drift apart.
- `aerosieve` has a default `parallel` feature; disable default features to build without rayon.

### Fixed

- Low-bitrate phone audio is no longer rejected as `tonal`. Codecs cut it off near 3 kHz, and the empty top of the flatness band read as a tone, rejecting 38% of 8 kHz Gram Vaani utterances. Band edges more than `CARRIED_BAND_FLOOR_DB` below the band's median are now left out; retention there rose from 75% to 93%.
- Clipping is measured against the source's full scale. Loudness leveling could turn a clipped file down 3 dB before scoring and hide the clipping.
- A pause must last 500 ms, not 100 ms, to end a clip. Natural speech was being cut into fragments under 3 s and dropped; on 43 minutes of FLEURS Hindi, kept speech rose from 16.4 to 25.6 minutes.
