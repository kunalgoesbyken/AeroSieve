const SVG = "http://www.w3.org/2000/svg";
const SIZE = { width: 560, height: 340 };
const PAD = { top: 20, right: 76, bottom: 44, left: 48 };
const RANGE_DB = [-5, 30];
const TICKS_DB = [0, 10, 20, 30];
const LABEL_GAP = 14;
const SERIES = [
  { noise: "white", label: "White", color: "var(--series-1)" },
  { noise: "pink", label: "Pink", color: "var(--series-2)" },
  { noise: "babble", label: "Babble", color: "var(--series-3)" },
];

const svg = (tag, attributes = {}, text) => {
  const node = document.createElementNS(SVG, tag);
  Object.entries(attributes).forEach(([name, value]) => node.setAttribute(name, value));
  if (text !== undefined) node.textContent = text;
  return node;
};

const scale = (value, [low, high], [from, to]) => from + ((value - low) / (high - low)) * (to - from);
const x = (db) => scale(db, RANGE_DB, [PAD.left, SIZE.width - PAD.right]);
const y = (db) => scale(Math.max(RANGE_DB[0], Math.min(RANGE_DB[1], db)), RANGE_DB, [SIZE.height - PAD.bottom, PAD.top]);

const axes = () => {
  const group = svg("g", { class: "axes" });
  TICKS_DB.forEach((db) => {
    group.append(
      svg("line", { x1: PAD.left, x2: SIZE.width - PAD.right, y1: y(db), y2: y(db), class: "grid" }),
      svg("text", { x: PAD.left - 10, y: y(db) + 4, "text-anchor": "end" }, `${db}`),
      svg("text", { x: x(db), y: SIZE.height - PAD.bottom + 20, "text-anchor": "middle" }, `${db}`),
    );
  });
  group.append(
    svg("text", { x: (PAD.left + SIZE.width - PAD.right) / 2, y: SIZE.height - 6, "text-anchor": "middle", class: "axis-title" }, "Injected SNR (dB)"),
    svg("text", { x: 14, y: PAD.top - 6, class: "axis-title" }, "Estimated (dB)"),
    svg("line", { x1: x(RANGE_DB[0]), y1: y(RANGE_DB[0]), x2: x(RANGE_DB[1]), y2: y(RANGE_DB[1]), class: "identity" }),
    svg("text", { x: x(26), y: y(28.5), class: "identity-label", "text-anchor": "end" }, "perfect estimate"),
  );
  return group;
};

const lastPoint = (points) => points.reduce((a, b) => (b.injected_snr_db > a.injected_snr_db ? b : a));

const spreadLabels = (anchors) => {
  const sorted = [...anchors].sort((a, b) => a.y - b.y);
  sorted.forEach((label, index) => {
    if (index > 0) label.y = Math.max(label.y, sorted[index - 1].y + LABEL_GAP);
  });
  return anchors;
};

const endLabels = (lines) =>
  spreadLabels(
    lines.map(({ series, points }) => {
      const last = lastPoint(points);
      return { series, x: x(last.injected_snr_db) + 10, y: y(last.median_estimate_db) + 4 };
    }),
  ).map(({ series, x: lx, y: ly }) => svg("text", { x: lx, y: ly, class: "series-label" }, series.label));

const seriesGroup = (series, points, showTip) => {
  const group = svg("g", { style: `--color:${series.color}` });
  const path = points.map((p, i) => `${i ? "L" : "M"}${x(p.injected_snr_db)},${y(p.median_estimate_db)}`).join("");
  group.append(svg("path", { d: path, class: "series-line" }));
  points.forEach((point) => {
    const cx = x(point.injected_snr_db);
    const cy = y(point.median_estimate_db);
    const hit = svg("circle", { cx, cy, r: 12, class: "hit", tabindex: "0" });
    const show = () => showTip(series, point, cx, cy);
    hit.addEventListener("pointerenter", show);
    hit.addEventListener("focus", show);
    group.append(svg("circle", { cx, cy, r: 4.5, class: "series-dot" }), hit);
  });
  return group;
};

const tooltipText = (series, point) =>
  `${series.label} noise at ${point.injected_snr_db} dB → reads ${point.median_estimate_db.toFixed(1)} dB ` +
  `(middle half ${point.p25_estimate_db.toFixed(1)}–${point.p75_estimate_db.toFixed(1)}) · ${Math.round(point.kept_rate * 100)}% kept`;

export function renderSnrChart(container, report) {
  const tooltip = container.querySelector("[data-chart-tip]");
  const chart = svg("svg", { viewBox: `0 0 ${SIZE.width} ${SIZE.height}`, role: "img", "aria-label": "WADA-SNR estimate against injected SNR for white, pink and babble noise" });
  const showTip = (series, point, cx, cy) => {
    tooltip.textContent = tooltipText(series, point);
    tooltip.style.left = `${(cx / SIZE.width) * 100}%`;
    tooltip.style.top = `${(cy / SIZE.height) * 100}%`;
    tooltip.hidden = false;
  };
  const lines = SERIES.map((series) => ({
    series,
    points: report.snr_estimates
      .filter((estimate) => estimate.noise === series.noise)
      .sort((a, b) => a.injected_snr_db - b.injected_snr_db),
  }));
  chart.append(axes(), ...lines.map(({ series, points }) => seriesGroup(series, points, showTip)), ...endLabels(lines));
  chart.addEventListener("pointerleave", () => (tooltip.hidden = true));
  container.querySelector("[data-chart]").replaceChildren(chart);
}
