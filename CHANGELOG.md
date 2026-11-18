# Changelog

## Unreleased

### Added

- `Inspector::score_whole` and Python's `inspect(..., whole=True)` score a recording as one clip without segmenting it, for corpora already cut and transcribed per utterance. Re-segmenting such audio separates it from its transcript and silently drops anything under 3 s; whole scoring reports digital silence as `silence` instead of returning no clip.
- `experiments/asr-hi`: a check on real Hindi phone speech with human-verified transcripts. Utterances AeroSieve rejects have 8.6 to 8.9 points higher WER for Whisper-large-v3 and MMS-1B, and the gap holds within 8 kHz audio alone.
- `aerosieve-serve`: a streaming quality gate. Audio goes in over a WebSocket and each utterance's verdict comes back as soon as its closing pause is heard, with session, utterance-length, frame-size and idle limits, Prometheus metrics, a load generator, and a Python client.
- `StreamSegmenter` and `LiveInspector` segment and score audio that arrives in pieces. Fed any chunking, they return exactly what batch segmentation and inspection return for the whole buffer, and hold only the audio of the utterance in progress.

### Changed

- Scoring is 4.7 times faster with identical results: spectral flatness uses a planned real FFT (`realfft`, already in the tree through the resampler) instead of a hand-written one, and WADA-SNR takes one logarithm per 512 samples rather than per sample. Every SieveBench score is unchanged; 48 kHz FLAC curation went from 660 to 940 times real time.
- `Segmenter::segment` runs the streaming segmenter over the whole buffer, so batch and live results cannot drift apart.

### Fixed

- Low-bitrate narrowband speech is no longer rejected as `tonal`. Codecs cut it off near 3 kHz, and the empty top of the 300 to 3400 Hz flatness band made speech read as a tone: on Gram Vaani Hindi, 38% of 8 kHz utterances were rejected this way. Flatness now leaves out band edges whose long-term level sits more than `CARRIED_BAND_FLOOR_DB` (30 dB) below the band's median; retention there went from 75% to 93%. Tones and hum still score 0, every SieveBench score is unchanged, and curation speed is unchanged.

## 0.4.0

### Fixed

- Clipping is measured against the source's full scale. Loudness leveling could turn a clipped file down by 3 dB before scoring, which hid the clipping and let damaged speech through.
- Clips are no longer cut at every word gap. A pause must last 500 ms, not 100 ms, to end a clip, so natural speech is no longer broken into fragments under the 3 s minimum and dropped. On 43 minutes of FLEURS Hindi, kept speech went from 16.4 to 25.6 minutes.

### Added

- SieveBench (`aerosieve-sievebench`): injects known defects into clean speech and reports how well each filter catches them, in six languages.
- `Pipeline::process_files` curates on every core with output identical to a sequential run. `aerosieve curate` and Python's `curate` use it.
- WebAssembly bindings and a landing page with a live in-browser demo at https://aerosieve.vercel.app.

### Changed

- `aerosieve_acoustic::score` and `clip_ratio` take the magnitude the source's full scale sits at; pass `FULL_SCALE` for untouched audio.
- `aerosieve` has a default `parallel` feature; disable default features to build without rayon.
