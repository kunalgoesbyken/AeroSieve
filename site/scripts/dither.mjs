import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const assets = fileURLToPath(new URL("../public/assets/", import.meta.url));
const size = 128;
const bayer = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

const random = (seed) => () => {
  seed = (seed * 1664525 + 1013904223) >>> 0;
  return seed / 2 ** 32;
};

const lattice = (period, next) => Array.from({ length: period * period }, next);

const sampler = (period, next) => {
  const values = lattice(period, next);
  const at = (x, y) => values[((y % period) * period) + (x % period)];
  const ease = (t) => t * t * (3 - 2 * t);
  return (x, y) => {
    const fx = (x / size) * period;
    const fy = (y / size) * period;
    const x0 = Math.floor(fx);
    const y0 = Math.floor(fy);
    const tx = ease(fx - x0);
    const ty = ease(fy - y0);
    const top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
    const bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
    return top + (bottom - top) * ty;
  };
};

const tile = (seed, fill) => {
  const next = random(seed);
  const octaves = [sampler(8, next), sampler(16, next), sampler(32, next)];
  const weights = [0.55, 0.3, 0.15];
  const cells = [];
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const noise = octaves.reduce((sum, octave, index) => sum + octave(x, y) * weights[index], 0);
      const density = Math.min(1, Math.max(0, (noise - 0.42) / 0.4)) * 0.7;
      if (density > (bayer[(y % 4) * 4 + (x % 4)] + 0.5) / 16) cells.push(`M${x} ${y}h1v1h-1z`);
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" shape-rendering="crispEdges"><path fill="${fill}" d="${cells.join("")}"/></svg>\n`;
};

writeFileSync(`${assets}dither.svg`, tile(7, "#d4d4d8"));
writeFileSync(`${assets}dither-dark.svg`, tile(11, "#27272a"));
