# AeroSieve site

The landing page, proof and docs at [aerosieve.kunalrc.xyz](https://aerosieve.kunalrc.xyz). A static [Astro](https://astro.build) build with no backend: the live demo runs AeroSieve's Rust pipeline in the browser through [`bindings/wasm`](../bindings/wasm).

| Path | Contents |
| :--- | :--- |
| `src/pages/` | `index.astro`, `proof.astro`, and the docs as Markdown under `docs/` |
| `src/components/` | Shared layout pieces, plus one folder per page: `home/`, `proof/` |
| `src/scripts/` | One module per job: `engine` loads WebAssembly, `demo` drives the live demo, `results` switches the SieveBench chart, `waveform` and `snr-chart` draw, `dither` generates the background pattern (drawn by `dither-worker` off the main thread) |
| `src/data/` | `site.js` for links and navigation, `sievebench.js` for the totals computed from the reports at build time |
| `public/pkg/` | WebAssembly build of `bindings/wasm`, committed so the site builds without Rust |
| `public/data/` | `hero.json` for the hero card and the demo's first render, and the SieveBench reports |
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

`node site/scripts/dither.mjs` regenerates the static background tiles in `public/assets/` from the same pattern the animated canvas draws.

SieveBench reports in `public/data/` come from `crates/aerosieve-sievebench/scripts/run_all.sh`.
