import { element } from "./dom.js";
import { renderSnrChart } from "./snr-chart.js";

const CORPORA = ["en", "hi", "ta", "bn", "te", "mr"];

const percent = (value) => `${Math.round(value * 100)}%`;

const keptShare = (report, id) => {
  const condition = report.conditions.find((entry) => entry.id === id);
  return condition && condition.clips ? condition.kept / condition.clips : 0;
};

const rightReasonShare = (report) => {
  const defects = report.families.filter((family) => family.family !== "control");
  const caught = defects.reduce((total, family) => total + family.right_reason, 0);
  const expected = defects.reduce((total, family) => total + family.should_reject, 0);
  return expected ? caught / expected : 0;
};

const barCell = (value) =>
  element("td", {}, [element("span", { className: "bar", style: `--v:${value.toFixed(3)}` }), percent(value)]);

const rowFor = (report) =>
  element("tr", {}, [
    element("td", {}, [element("strong", { text: report.corpus }), element("span", { className: "source", text: report.source })]),
    element("td", { text: `${Math.round(report.hours * 60)} min` }),
    barCell(keptShare(report, "clean")),
    barCell(keptShare(report, "phone_line")),
    element("td", { className: "strong", text: percent(report.overall.recall) }),
    element("td", { text: percent(rightReasonShare(report)) }),
  ]);

const totals = (reports) => {
  const recordings = reports.reduce((total, report) => total + report.utterances, 0);
  const conditions = reports[0].conditions.length;
  const caught = reports.reduce((total, report) => total + report.overall.caught, 0);
  const expected = reports.reduce((total, report) => total + report.overall.should_reject, 0);
  return { recordings, clips: recordings * conditions, caught, expected };
};

const mountChartSwitch = (root, reports) => {
  const chart = root.querySelector("[data-snr]");
  const buttons = reports.map((report) =>
    element("button", { type: "button", className: "chip", "aria-pressed": "false", text: report.corpus }),
  );
  const select = (index) => {
    buttons.forEach((button, i) => button.setAttribute("aria-pressed", String(i === index)));
    renderSnrChart(chart, reports[index]);
  };
  buttons.forEach((button, index) => button.addEventListener("click", () => select(index)));
  root.querySelector("[data-snr-switch]").replaceChildren(...buttons);
  select(0);
};

export async function mountResults(root) {
  const reports = await Promise.all(
    CORPORA.map(async (id) => (await fetch(`data/sievebench-${id}.json`)).json()),
  );
  const summary = totals(reports);
  root.querySelector("[data-languages]").replaceChildren(...reports.map(rowFor));
  Object.entries(summary).forEach(([name, value]) =>
    document.querySelectorAll(`[data-bench-${name}]`).forEach((node) => (node.textContent = value.toLocaleString("en"))),
  );
  mountChartSwitch(root, reports);
}
