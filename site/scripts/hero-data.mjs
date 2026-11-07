import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const site = fileURLToPath(new URL("..", import.meta.url));
const sample = "mixed.ogg";
const envelopePoints = 140;

const engine = await import(new URL("../pkg/aerosieve_wasm.js", import.meta.url));
await engine.default({ module_or_path: readFileSync(`${site}/pkg/aerosieve_wasm_bg.wasm`) });

const inspection = new engine.Inspection(readFileSync(`${site}/samples/${sample}`), "ogg");
const round = (value, digits) => Number(value.toFixed(digits));

const hero = {
  sample,
  duration_s: round(inspection.duration_s, 2),
  sample_rate: inspection.sample_rate,
  envelope: Array.from(inspection.envelope(envelopePoints), (peak) => round(peak, 3)),
  clips: inspection.clips(),
};

writeFileSync(`${site}/data/hero.json`, JSON.stringify(hero));
console.log(`wrote data/hero.json: ${hero.clips.length} clips`);
