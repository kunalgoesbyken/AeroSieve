import { whileVisible } from "./motion.js";
import { clipAtTime, drawWaveform } from "./waveform.js";

export async function mountHero(root) {
  const canvas = root.querySelector("[data-hero-waveform]");
  const scan = root.querySelector("[data-hero-scan]");
  const pills = [...root.querySelectorAll("[data-clip]")];
  const waveform = JSON.parse(root.dataset.hero);
  const { clips, durationS } = waveform;
  new ResizeObserver(() => drawWaveform(canvas, waveform)).observe(canvas);

  const [sweep] = scan.getAnimations();
  if (!sweep) return;
  await sweep.ready;
  const { startTime } = sweep;
  const { duration } = sweep.effect.getTiming();
  const toSweep = (seconds) => (seconds / durationS) * duration;
  const boundaries = clips.flatMap((clip) => [toSweep(clip.start_s), toSweep(clip.start_s + clip.duration_s)]).sort((a, b) => a - b);

  let timer;
  const follow = () => {
    const elapsed = (document.timeline.currentTime - startTime) % duration;
    const current = clipAtTime(clips, (elapsed / duration) * durationS);
    pills.forEach((pill) => pill.classList.toggle("is-current", clips[pill.dataset.clip] === current));
    const next = boundaries.find((boundary) => boundary > elapsed) ?? duration;
    timer = setTimeout(follow, next - elapsed);
  };
  whileVisible(canvas, (visible) => {
    clearTimeout(timer);
    if (visible) follow();
  });
}
