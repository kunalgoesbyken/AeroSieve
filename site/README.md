# AeroSieve site

The landing page and live demo at [aerosieve.vercel.app](https://aerosieve.vercel.app). It is a static page with no build step and no backend: the demo runs AeroSieve's Rust pipeline in the browser through [`bindings/wasm`](../bindings/wasm).

| Path | Contents |
| :--- | :--- |
| `index.html`, `styles.css` | The page |
| `js/` | One module per job: `engine` loads WebAssembly, `demo` drives the live demo, `results` renders SieveBench, `waveform` and `snr-chart` draw |
| `pkg/` | WebAssembly build of `bindings/wasm`, committed so the page deploys as plain files |
| `data/` | `hero.json` for the hero card and the SieveBench reports |
| `samples/` | Demo recordings: LibriSpeech utterances (CC BY 4.0) with noise, clipping and a dial tone added |

## Build and run

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
bindings/wasm/build.sh             # writes site/pkg
node site/scripts/hero-data.mjs    # writes site/data/hero.json
python3 -m http.server -d site 8000
```

SieveBench reports in `data/` come from `crates/aerosieve-sievebench/scripts/run_all.sh`.
