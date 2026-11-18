import { drawWaveform } from "./waveform.js";

export async function mountHero(root) {
  const canvas = root.querySelector("[data-hero-waveform]");
  const waveform = JSON.parse(root.dataset.hero);
  new ResizeObserver(() => drawWaveform(canvas, waveform)).observe(canvas);
}
