# AeroSieve site

The landing page, proof and docs at [aerosieve.kunalrc.xyz](https://aerosieve.kunalrc.xyz). A static [Astro](https://astro.build) build with no backend: the live demo runs AeroSieve's Rust pipeline in the browser through [`bindings/wasm`](../bindings/wasm).

| Path | Contents |
| :--- | :--- |
| `src/pages/` | `index.astro`, `proof.astro`, and the docs as Markdown under `docs/` |
| `src/components/` | Shared layout pieces, plus one folder per page: `home/`, `proof/` |
| `src/scripts/` | One module per job: `engine` loads WebAssembly, `demo` drives the live demo, `results` renders SieveBench, `waveform` and `snr-chart` draw |
| `src/data/site.js` | Links and navigation used across pages |
| `public/pkg/` | WebAssembly build of `bindings/wasm`, committed so the site builds without Rust |
| `public/data/` | `hero.json` for the hero card and the SieveBench reports |
| `public/samples/` | Demo recordings: LibriSpeech utterances (CC BY 4.0) with noise, clipping and a dial tone added |

## Build and run

```bash
npm install
npm run dev        # http://localhost:4321
npm run build      # writes dist/
```

To refresh the WebAssembly build and the hero card after changing the Rust code:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
bindings/wasm/build.sh             # writes site/public/pkg
node site/scripts/hero-data.mjs    # writes site/public/data/hero.json
```

SieveBench reports in `public/data/` come from `crates/aerosieve-sievebench/scripts/run_all.sh`.
