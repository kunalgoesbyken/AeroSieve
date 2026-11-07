import { mountCopyButtons } from "./copy.js";
import { mountDemo } from "./demo.js";
import { mountHero } from "./hero.js";
import { mountResults } from "./results.js";
import { mountTabs } from "./tabs.js";

mountCopyButtons(document);
mountTabs(document);
const mount = (selector, start) => {
  const root = document.querySelector(selector);
  start(root).catch((error) => {
    root.dataset.failed = "";
    console.error(error);
  });
};

mount("[data-hero]", mountHero);
mount("[data-demo]", mountDemo);
mount("[data-results]", mountResults);

const prefersMotion = !matchMedia("(prefers-reduced-motion: reduce)").matches;

if (prefersMotion && "IntersectionObserver" in window) {
  const reveal = new IntersectionObserver(
    (entries) =>
      entries.filter((entry) => entry.isIntersecting).forEach((entry) => {
        entry.target.classList.add("is-visible");
        reveal.unobserve(entry.target);
      }),
    { rootMargin: "0px 0px -10% 0px" },
  );
  document.querySelectorAll("[data-reveal]").forEach((node) => reveal.observe(node));
  document.documentElement.classList.add("can-reveal");
}
