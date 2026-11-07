# Changelog

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
