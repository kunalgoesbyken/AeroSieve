# aerosieve-wasm

WebAssembly bindings for AeroSieve. A file's bytes go through the same decode, segment and score code as `aerosieve inspect`, so a verdict in the browser is the verdict the CLI gives.

```js
import init, { Inspection, defaultPolicy } from "./pkg/aerosieve_wasm.js";

await init();
const inspection = new Inspection(new Uint8Array(await file.arrayBuffer()), "mp3");
inspection.clips();                     // every clip with its metrics and verdict
inspection.clips({ min_snr_db: 15 });   // re-judge the stored metrics, no re-decoding
inspection.envelope(200);               // peaks for drawing a waveform
inspection.samples();                   // 16 kHz mono, for playback
```

Recordings over 15 minutes are refused while decoding. Build with `build.sh`, which writes `site/pkg`.
