const PACKAGE = "/pkg/aerosieve_wasm.js";

let ready;

const load = () => {
  ready ??= import(/* @vite-ignore */ PACKAGE).then(async (wasm) => {
    await wasm.default();
    return wasm;
  });
  return ready;
};

export const extensionOf = (name) => name.split(".").pop().toLowerCase();

export async function inspect(bytes, extension) {
  const { Inspection } = await load();
  const started = performance.now();
  const inspection = new Inspection(new Uint8Array(bytes), extension);
  return { inspection, elapsedMs: performance.now() - started };
}

export async function defaults() {
  const { defaultPolicy } = await load();
  return defaultPolicy();
}
