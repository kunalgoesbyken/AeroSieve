import init, { Inspection, defaultPolicy } from "../pkg/aerosieve_wasm.js";

let ready;

const load = () => {
  ready ??= init();
  return ready;
};

export const extensionOf = (name) => name.split(".").pop().toLowerCase();

export async function inspect(bytes, extension) {
  await load();
  const started = performance.now();
  const inspection = new Inspection(new Uint8Array(bytes), extension);
  return { inspection, elapsedMs: performance.now() - started };
}

export async function defaults() {
  await load();
  return defaultPolicy();
}
