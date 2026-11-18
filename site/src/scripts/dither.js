export const DITHER_TILE = 128;
export const DITHER_SEEDS = { light: 7, dark: 11 };

const DENSITY = { floor: 0.42, span: 0.4, max: 0.7 };
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
const OCTAVES = [
  { period: 8, weight: 0.55, drift: [0.11, 0.04] },
  { period: 16, weight: 0.3, drift: [-0.07, 0.09] },
  { period: 32, weight: 0.15, drift: [0.05, -0.12] },
];

const random = (seed) => () => {
  seed = (seed * 1664525 + 1013904223) >>> 0;
  return seed / 2 ** 32;
};

const smooth = (t) => t * t * (3 - 2 * t);

const wrap = (value, period) => ((value % period) + period) % period;

const axis = (length, period, shift) => {
  const low = new Int32Array(length);
  const high = new Int32Array(length);
  const blend = new Float64Array(length);
  for (let i = 0; i < length; i += 1) {
    const position = (i / DITHER_TILE) * period + shift;
    const cell = Math.floor(position);
    low[i] = wrap(cell, period);
    high[i] = wrap(cell + 1, period);
    blend[i] = smooth(position - cell);
  }
  return { low, high, blend };
};

export const ditherField = (seed) => {
  const next = random(seed);
  const octaves = OCTAVES.map((octave) => ({ ...octave, values: Array.from({ length: octave.period ** 2 }, next) }));

  return (width, height, time, plot) => {
    const layers = octaves.map(({ period, weight, drift, values }) => ({
      period,
      weight,
      values,
      columns: axis(width, period, drift[0] * time),
      rows: axis(height, period, drift[1] * time),
    }));
    const noise = new Float64Array(width);
    for (let y = 0; y < height; y += 1) {
      noise.fill(0);
      for (const { period, weight, values, columns, rows } of layers) {
        const top = rows.low[y] * period;
        const bottom = rows.high[y] * period;
        const ty = rows.blend[y];
        for (let x = 0; x < width; x += 1) {
          const left = columns.low[x];
          const right = columns.high[x];
          const tx = columns.blend[x];
          const upper = values[top + left] + (values[top + right] - values[top + left]) * tx;
          const lower = values[bottom + left] + (values[bottom + right] - values[bottom + left]) * tx;
          noise[x] += weight * (upper + (lower - upper) * ty);
        }
      }
      for (let x = 0; x < width; x += 1) {
        const density = Math.min(1, Math.max(0, (noise[x] - DENSITY.floor) / DENSITY.span)) * DENSITY.max;
        if (density > (BAYER[(y % 4) * 4 + (x % 4)] + 0.5) / 16) plot(x, y);
      }
    }
  };
};
