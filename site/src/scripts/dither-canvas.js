import { prefersMotion, whileVisible } from "./motion.js";

const CELL_PX = 4;

const inkOf = (canvas) => {
  const probe = new OffscreenCanvas(1, 1).getContext("2d");
  probe.fillStyle = getComputedStyle(canvas).color;
  probe.fillRect(0, 0, 1, 1);
  return new Uint32Array(probe.getImageData(0, 0, 1, 1).data.buffer)[0];
};

const sizeOf = (canvas) => ({
  width: Math.ceil(canvas.clientWidth / CELL_PX),
  height: Math.ceil(canvas.clientHeight / CELL_PX),
});

export function mountDither(canvas) {
  if (!prefersMotion() || !("transferControlToOffscreen" in canvas)) return;
  const worker = new Worker(new URL("./dither-worker.js", import.meta.url), { type: "module" });
  const offscreen = canvas.transferControlToOffscreen();
  worker.postMessage({ canvas: offscreen, seed: Number(canvas.dataset.seed), ink: inkOf(canvas) }, [offscreen]);
  worker.addEventListener("message", () => canvas.classList.add("is-live"), { once: true });
  new ResizeObserver(() => worker.postMessage({ size: sizeOf(canvas) })).observe(canvas);
  let inView = false;
  const play = () => worker.postMessage({ visible: inView && !document.hidden });
  whileVisible(canvas, (visible) => {
    inView = visible;
    play();
  });
  document.addEventListener("visibilitychange", play);
}
