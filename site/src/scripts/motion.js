const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

export const prefersMotion = () => !reducedMotion.matches;

export function whileVisible(target, onChange) {
  new IntersectionObserver(([entry]) => onChange(entry.isIntersecting)).observe(target);
}

const ENGAGEMENT_EVENTS = ["pointermove", "pointerdown", "scroll", "keydown"];

export function onFirstEngagement(run) {
  const start = () => {
    ENGAGEMENT_EVENTS.forEach((type) => removeEventListener(type, start));
    run();
  };
  ENGAGEMENT_EVENTS.forEach((type) => addEventListener(type, start, { passive: true }));
}
