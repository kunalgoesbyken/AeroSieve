import { addCodeCopyButtons, mountCopyButtons } from "./copy.js";
import { mountDemo } from "./demo.js";
import { mountHero } from "./hero.js";
import { mountResults } from "./results.js";
import { mountTabs } from "./tabs.js";

addCodeCopyButtons(document);
mountCopyButtons(document);
mountTabs(document);

const mount = (selector, run) => {
  const root = document.querySelector(selector);
  if (!root) return;
  const near = new IntersectionObserver(
    (entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      near.disconnect();
      run(root).catch((error) => {
        root.dataset.failed = "";
        console.error(error);
      });
    },
    { rootMargin: "400px 0px" },
  );
  near.observe(root);
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
