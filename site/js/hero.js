import { element } from "./dom.js";
import { fetchJson } from "./http.js";
import { describe, isKept } from "./verdicts.js";
import { drawWaveform } from "./waveform.js";

const pillFor = (clip, order) => {
  const { label, tone, detail } = describe(clip);
  const pill = element("li", { className: `pill tone-${tone}` }, [
    element("span", { className: "pill-time", text: `${clip.start_s.toFixed(1)}s` }),
    element("span", { className: "pill-label", text: label }),
    element("span", { className: "pill-detail", text: detail }),
  ]);
  pill.style.setProperty("--order", order);
  return pill;
};

export async function mountHero(root) {
  const hero = await fetchJson("data/hero.json");
  const canvas = root.querySelector("[data-hero-waveform]");
  const draw = () => drawWaveform(canvas, { envelope: hero.envelope, durationS: hero.duration_s, clips: hero.clips });
  const pills = hero.clips.map(pillFor);

  root.querySelector("[data-hero-file]").textContent = hero.sample;
  root.querySelector("[data-hero-length]").textContent = `${hero.duration_s.toFixed(1)} s · ${hero.sample_rate / 1000} kHz`;
  root.querySelector("[data-hero-kept]").replaceChildren(...pills.filter((_, i) => isKept(hero.clips[i])));
  root.querySelector("[data-hero-dropped]").replaceChildren(...pills.filter((_, i) => !isKept(hero.clips[i])));
  root.querySelector("[data-hero-summary]").textContent =
    `${hero.clips.filter(isKept).length} of ${hero.clips.length} clips kept · decoded and scored by the WebAssembly build`;
  new ResizeObserver(draw).observe(canvas);
}
