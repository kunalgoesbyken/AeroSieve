import { addCodeCopyButtons, mountCopyButtons } from "./copy.js";
import { mountDemo } from "./demo.js";
import { mountDither } from "./dither-canvas.js";
import { mountHero } from "./hero.js";
import { onFirstEngagement, prefersMotion, whileVisible } from "./motion.js";
import { mountResults } from "./results.js";
import { mountTabs } from "./tabs.js";

addCodeCopyButtons(document);
mountCopyButtons(document);
mountTabs(document);

const mountNear = (selector, run) => {
  const root = document.querySelector(selector);
  if (!root) return;
  const near = new IntersectionObserver(
    (entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      near.disconnect();
      Promise.resolve()
        .then(() => run(root))
        .catch(console.error);
    },
    { rootMargin: "400px 0px" },
  );
  near.observe(root);
};

mountNear("[data-hero]", mountHero);
mountNear("[data-demo]", mountDemo);
mountNear("[data-results]", mountResults);

onFirstEngagement(() => document.querySelectorAll("[data-dither]").forEach(mountDither));

if (prefersMotion()) {
  document.querySelectorAll("[data-art]").forEach((art) => whileVisible(art, (visible) => art.classList.toggle("is-playing", visible)));
}
