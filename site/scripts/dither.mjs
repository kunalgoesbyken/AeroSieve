import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { DITHER_SEEDS, DITHER_TILE, ditherField } from "../src/scripts/dither.js";

const assets = fileURLToPath(new URL("../public/assets/", import.meta.url));

const tile = (seed, fill) => {
  const cells = [];
  ditherField(seed)(DITHER_TILE, DITHER_TILE, 0, (x, y) => cells.push(`M${x} ${y}h1v1h-1z`));
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${DITHER_TILE} ${DITHER_TILE}" shape-rendering="crispEdges"><path fill="${fill}" d="${cells.join("")}"/></svg>\n`;
};

writeFileSync(`${assets}dither.svg`, tile(DITHER_SEEDS.light, "#d4d4d8"));
writeFileSync(`${assets}dither-dark.svg`, tile(DITHER_SEEDS.dark, "#27272a"));
