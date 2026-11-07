import { verdictOf } from "./verdicts.js";

const BAR_GAP = 0.35;

const toneColor = (style, tone) => style.getPropertyValue(`--tone-${tone}`).trim();

const fitToDisplay = (canvas) => {
  const ratio = window.devicePixelRatio || 1;
  const { width, height } = canvas.getBoundingClientRect();
  canvas.width = Math.round(width * ratio);
  canvas.height = Math.round(height * ratio);
  const context = canvas.getContext("2d");
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  return { context, width, height };
};

const clipAtTime = (clips, timeS) =>
  clips.find((clip) => timeS >= clip.start_s && timeS < clip.start_s + clip.duration_s);

export function drawWaveform(canvas, { envelope, durationS, clips }) {
  const { context, width, height } = fitToDisplay(canvas);
  const style = getComputedStyle(canvas);
  const idle = toneColor(style, "idle");
  const step = width / envelope.length;
  const barWidth = Math.max(1, step * (1 - BAR_GAP));
  const peak = Math.max(...envelope, 1e-6);
  const middle = height / 2;

  context.clearRect(0, 0, width, height);
  envelope.forEach((value, index) => {
    const clip = clipAtTime(clips, ((index + 0.5) / envelope.length) * durationS);
    const barHeight = Math.max(2, (value / peak) * (height - 4));
    context.fillStyle = clip ? toneColor(style, verdictOf(clip).tone) : idle;
    context.beginPath();
    context.roundRect(index * step, middle - barHeight / 2, barWidth, barHeight, barWidth / 2);
    context.fill();
  });
}
