import { ditherField } from "./dither.js";

const FRAME_MS = 80;
const DRIFT_PER_MS = 0.0006;

let canvas;
let context;
let render;
let ink;
let image;
let pixels;
let timer;
let elapsedMs = 0;

const resize = ({ width, height }) => {
  if (!width || !height) return;
  canvas.width = width;
  canvas.height = height;
  image = context.createImageData(width, height);
  pixels = new Uint32Array(image.data.buffer);
};

const draw = () => {
  if (!pixels) return;
  const { width, height } = canvas;
  pixels.fill(0);
  render(width, height, elapsedMs * DRIFT_PER_MS, (x, y) => (pixels[y * width + x] = ink));
  context.putImageData(image, 0, 0);
};

const tick = () => {
  elapsedMs += FRAME_MS;
  draw();
  timer = setTimeout(tick, FRAME_MS);
};

const play = (visible) => {
  clearTimeout(timer);
  if (visible) timer = setTimeout(tick, FRAME_MS);
};

self.onmessage = ({ data }) => {
  if (data.canvas) {
    ({ canvas } = data);
    context = canvas.getContext("2d");
    render = ditherField(data.seed);
    ink = data.ink;
  }
  if (data.size) {
    resize(data.size);
    draw();
    self.postMessage("drawn");
  }
  if ("visible" in data) play(data.visible);
};
