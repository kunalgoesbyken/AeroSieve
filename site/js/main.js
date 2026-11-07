import { mountCopyButtons } from "./copy.js";
import { mountDemo } from "./demo.js";
import { mountHero } from "./hero.js";
import { mountResults } from "./results.js";
import { mountTabs } from "./tabs.js";

mountCopyButtons(document);
mountTabs(document);
mountHero(document.querySelector("[data-hero]"));
mountDemo(document.querySelector("[data-demo]"));
mountResults(document.querySelector("[data-results]"));

const reveal = new IntersectionObserver(
  (entries) =>
    entries.filter((entry) => entry.isIntersecting).forEach((entry) => {
      entry.target.classList.add("is-visible");
      reveal.unobserve(entry.target);
    }),
  { rootMargin: "0px 0px -10% 0px" },
);
document.querySelectorAll("[data-reveal]").forEach((node) => reveal.observe(node));
