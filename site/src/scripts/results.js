import { renderSnrChart } from "./snr-chart.js";

export function mountResults(root) {
  const reports = JSON.parse(root.dataset.results);
  const chart = root.querySelector("[data-snr]");
  const buttons = [...root.querySelectorAll("[data-snr-corpus]")];
  const select = (chosen) => {
    buttons.forEach((button) => button.setAttribute("aria-pressed", String(button === chosen)));
    renderSnrChart(chart, reports[chosen.dataset.snrCorpus]);
  };
  buttons.forEach((button) => button.addEventListener("click", () => select(button)));
  select(buttons[0]);
}
