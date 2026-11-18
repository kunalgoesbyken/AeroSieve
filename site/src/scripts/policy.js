import { element } from "./dom.js";

const FIELDS = [
  { key: "min_snr_db", label: "SNR floor", min: -5, max: 30, step: 0.5, format: (v) => `${v.toFixed(1)} dB` },
  { key: "max_clip_ratio", label: "Clipping ceiling", min: 0, max: 0.2, step: 0.005, format: (v) => `${(v * 100).toFixed(1)}%` },
  { key: "min_dbfs", label: "Level floor", min: -70, max: -10, step: 1, format: (v) => `${v.toFixed(0)} dBFS` },
  { key: "max_dbfs", label: "Level ceiling", min: -20, max: 0, step: 1, format: (v) => `${v.toFixed(0)} dBFS` },
  { key: "min_spectral_flatness", label: "Flatness floor", min: 0, max: 0.05, step: 0.001, format: (v) => v.toFixed(3) },
];

const sliderFor = (field, value, onInput) => {
  const input = element("input", { type: "range", min: field.min, max: field.max, step: field.step });
  input.value = value;
  const output = element("output");
  const row = element("label", { className: "policy-row" }, [
    element("span", { className: "policy-label", text: field.label }, [element("code", { text: field.key })]),
    output,
    input,
  ]);
  const show = () => (output.textContent = field.format(Number(input.value)));
  input.addEventListener("input", () => {
    show();
    onInput(field.key, Number(input.value));
  });
  show();
  return { row, set: (next) => ((input.value = next), show()) };
};

export function mountPolicyControls(container, defaults, onChange) {
  let policy = { ...defaults };
  const sliders = FIELDS.map((field) =>
    sliderFor(field, defaults[field.key], (key, value) => {
      policy = { ...policy, [key]: value };
      onChange(policy);
    }),
  );
  container.replaceChildren(...sliders.map((slider) => slider.row));

  return () => {
    policy = { ...defaults };
    sliders.forEach((slider, index) => slider.set(defaults[FIELDS[index].key]));
    onChange(policy);
  };
}
