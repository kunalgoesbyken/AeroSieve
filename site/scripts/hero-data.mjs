import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { ENVELOPE_POINTS } from "../src/scripts/waveform.js";

const site = fileURLToPath(new URL("..", import.meta.url));
const sample = "mixed.ogg";

const engine = await import(new URL("../public/pkg/aerosieve_wasm.js", import.meta.url));
await engine.default({ module_or_path: readFileSync(`${site}/public/pkg/aerosieve_wasm_bg.wasm`) });

const inspection = new engine.Inspection(readFileSync(`${site}/public/samples/${sample}`), "ogg");
const round = (value, digits) => Number(value.toFixed(digits));

const hero = {
  sample,
  duration_s: round(inspection.duration_s, 2),
  sample_rate: inspection.sample_rate,
  source_sample_rate: inspection.source_sample_rate,
  envelope: Array.from(inspection.envelope(ENVELOPE_POINTS), (peak) => round(peak, 3)),
  policy: engine.defaultPolicy(),
  clips: inspection.clips(),
};

writeFileSync(`${site}/public/data/hero.json`, JSON.stringify(hero));
console.log(`wrote public/data/hero.json: ${hero.clips.length} clips`);
