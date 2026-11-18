import { element } from "./dom.js";
import { extensionOf, inspect } from "./engine.js";
import { fetchBytes } from "./http.js";
import { ClipPlayer } from "./player.js";
import { mountPolicyControls } from "./policy.js";
import { describe, isKept } from "./verdicts.js";
import { drawWaveform, ENVELOPE_POINTS } from "./waveform.js";

const MAX_UPLOAD_BYTES = 50 * 1024 * 1024;

const seconds = (value) => `${value.toFixed(1)}s`;

export async function mountDemo(root) {
  const find = (selector) => root.querySelector(selector);
  const ui = {
    canvas: find("[data-waveform]"),
    overlay: find("[data-overlay]"),
    rows: find("[data-rows]"),
    status: find("[data-status]"),
    stats: find("[data-stats]"),
    fileInput: find("[data-file]"),
    samples: [...root.querySelectorAll("[data-sample]")],
  };
  const player = new ClipPlayer();
  const snapshot = JSON.parse(root.dataset.demo);
  let current;
  let policy = snapshot.policy;

  const setStatus = (message, isError = false) => {
    ui.status.textContent = message;
    ui.status.classList.toggle("is-error", isError);
  };

  const play = (clip, element) => {
    if (!current) return;
    root.querySelectorAll(".is-playing").forEach((node) => node.classList.remove("is-playing"));
    element.classList.add("is-playing");
    player.play(clip.start_s, clip.duration_s, () => element.classList.remove("is-playing"));
  };

  const renderOverlay = (clips, durationS) => {
    ui.overlay.replaceChildren(
      ...clips.map((clip) => {
        const { label, tone, detail } = describe(clip);
        const button = element(
          "button",
          { type: "button", className: `segment tone-${tone}`, "aria-label": `Play ${seconds(clip.start_s)}: ${label}, ${detail}` },
          [element("span", { className: "tag", text: label })],
        );
        button.style.left = `${(clip.start_s / durationS) * 100}%`;
        button.style.width = `${(clip.duration_s / durationS) * 100}%`;
        button.addEventListener("click", () => play(clip, button));
        return button;
      }),
    );
  };

  const cell = (text, className) => element("td", { text, className });

  const renderRows = (clips) => {
    ui.rows.replaceChildren(
      ...clips.map((clip) => {
        const { label, tone, detail } = describe(clip);
        const row = element("tr", { tabindex: "0" }, [
          cell(seconds(clip.start_s)),
          cell(seconds(clip.duration_s)),
          cell(clip.wada_snr_db.toFixed(1)),
          cell(clip.rms_dbfs.toFixed(1)),
          cell(clip.clip_ratio.toFixed(3)),
          cell(clip.spectral_flatness.toFixed(3)),
          cell(label, `verdict tone-${tone}`),
          cell(detail, "detail"),
        ]);
        const activate = () => play(clip, row);
        row.addEventListener("click", activate);
        row.addEventListener("keydown", (event) => event.key === "Enter" && activate());
        return row;
      }),
    );
  };

  const renderStats = ({ name, durationS, sourceRate, clips, elapsedMs }) => {
    const kept = clips.filter(isKept);
    const keptSeconds = kept.reduce((total, clip) => total + clip.duration_s, 0);
    const values = [
      ["file", name],
      ["length", seconds(durationS)],
      ["source rate", `${(sourceRate / 1000).toFixed(1)} kHz`],
      ["clips kept", `${kept.length} of ${clips.length}`],
      ["speech kept", seconds(keptSeconds)],
      ["decoded + scored", elapsedMs === undefined ? "…" : `${elapsedMs.toFixed(0)} ms`],
    ];
    ui.stats.replaceChildren(
      ...values.map(([term, value]) =>
        element("div", {}, [element("dt", { text: term }), element("dd", { text: value })]),
      ),
    );
  };

  const viewOf = ({ inspection, name, elapsedMs, envelope }) => ({
    name,
    elapsedMs,
    envelope,
    durationS: inspection.duration_s,
    sourceRate: inspection.source_sample_rate,
    clips: inspection.clips(policy),
  });

  let view = {
    name: snapshot.sample,
    envelope: snapshot.envelope,
    durationS: snapshot.duration_s,
    sourceRate: snapshot.source_sample_rate,
    clips: snapshot.clips,
  };

  const render = () => {
    if (current) view = viewOf(current);
    drawWaveform(ui.canvas, view);
    renderOverlay(view.clips, view.durationS);
    renderRows(view.clips);
    renderStats(view);
  };

  const ready = () => setStatus(view.clips.length ? "Click a clip to hear it." : "No speech long enough to make a clip.");

  let latest = 0;

  const run = async (name, readBytes) => {
    const request = ++latest;
    setStatus(`Decoding ${name}…`);
    try {
      const { inspection, elapsedMs } = await inspect(await readBytes(), extensionOf(name));
      if (request !== latest) {
        inspection.free();
        return;
      }
      current?.inspection.free();
      current = { inspection, name, elapsedMs, envelope: inspection.envelope(ENVELOPE_POINTS) };
      player.load(inspection.samples(), inspection.sample_rate);
      render();
      ready();
    } catch (error) {
      if (request === latest) setStatus(`Could not read ${name}: ${error.message ?? error}`, true);
    }
  };

  const loadSample = (button) => {
    ui.samples.forEach((other) => other.setAttribute("aria-pressed", String(other === button)));
    const name = button.dataset.sample;
    return run(name, () => fetchBytes(`/samples/${name}`));
  };

  const loadFile = async (file) => {
    if (!file) return;
    if (file.size > MAX_UPLOAD_BYTES) {
      setStatus(`${file.name} is over 50 MB. Try a shorter recording.`, true);
      return;
    }
    ui.samples.forEach((button) => button.setAttribute("aria-pressed", "false"));
    await run(file.name, () => file.arrayBuffer());
  };

  ui.samples.forEach((button) => button.addEventListener("click", () => loadSample(button)));
  ui.fileInput.addEventListener("change", () => loadFile(ui.fileInput.files[0]));
  root.addEventListener("dragover", (event) => {
    event.preventDefault();
    root.classList.add("is-dragging");
  });
  root.addEventListener("dragleave", () => root.classList.remove("is-dragging"));
  root.addEventListener("drop", (event) => {
    event.preventDefault();
    root.classList.remove("is-dragging");
    loadFile(event.dataTransfer.files[0]);
  });

  const reset = mountPolicyControls(find("[data-policy]"), policy, (next) => {
    policy = next;
    render();
  });
  find("[data-reset]").addEventListener("click", reset);
  new ResizeObserver(render).observe(ui.canvas);
  await loadSample(ui.samples.find((button) => button.dataset.sample === snapshot.sample));
}
