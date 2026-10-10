"""Upload a recording and see which clips AeroSieve would keep, and why."""

import aerosieve
import gradio as gr
import numpy as np
from matplotlib.figure import Figure
import soundfile as sf

ACCEPTED = "accept"
ACCEPT_COLOR = "#2e7d32"
REJECT_COLOR = "#c62828"
MAX_PLOT_POINTS = 4000
COLUMNS = ["start (s)", "length (s)", "SNR (dB)", "level (dBFS)", "clipping", "flatness", "verdict"]


def waveform_envelope(path):
    """Peak envelope of the recording, reduced to a plottable number of points."""
    samples, sample_rate = sf.read(path, dtype="float32", always_2d=True)
    mono = np.abs(samples.mean(axis=1))
    bucket = max(1, len(mono) // MAX_PLOT_POINTS)
    usable = len(mono) - len(mono) % bucket
    envelope = mono[:usable].reshape(-1, bucket).max(axis=1)
    times = np.arange(len(envelope)) * bucket / sample_rate
    return times, envelope


def plot_clips(path, clips):
    times, envelope = waveform_envelope(path)
    figure = Figure(figsize=(11, 2.8))
    axis = figure.subplots()
    axis.fill_between(times, -envelope, envelope, color="#607d8b", linewidth=0)
    for clip in clips:
        color = ACCEPT_COLOR if clip["verdict"] == ACCEPTED else REJECT_COLOR
        axis.axvspan(clip["start_s"], clip["start_s"] + clip["duration_s"], color=color, alpha=0.25)
    axis.set_xlabel("seconds")
    axis.set_yticks([])
    axis.margins(x=0)
    figure.tight_layout()
    return figure


def clip_rows(clips):
    return [
        [
            round(clip["start_s"], 2),
            round(clip["duration_s"], 2),
            round(clip["wada_snr_db"], 1),
            round(clip["rms_dbfs"], 1),
            round(clip["clip_ratio"], 3),
            round(clip["spectral_flatness"], 3),
            clip["verdict"],
        ]
        for clip in clips
    ]


def inspect(path, min_snr_db, min_spectral_flatness):
    if path is None:
        raise gr.Error("Upload or record some audio first.")
    policy = {"min_snr_db": min_snr_db, "min_spectral_flatness": min_spectral_flatness}
    try:
        clips = aerosieve.inspect(path, policy=policy)["clips"]
    except RuntimeError as error:
        raise gr.Error(str(error)) from error

    accepted = sum(clip["verdict"] == ACCEPTED for clip in clips)
    summary = f"{accepted} of {len(clips)} clips accepted. Green spans are kept, red are rejected."
    return plot_clips(path, clips), clip_rows(clips), summary


with gr.Blocks(title="AeroSieve") as demo:
    gr.Markdown(
        "# AeroSieve\n"
        "Cuts a recording into speech clips of 3 to 30 seconds and decides which are clean "
        "enough to train a speech model on. "
        "[Source](https://github.com/kunalgoesbyken/AeroSieve)"
    )
    with gr.Row():
        audio = gr.Audio(type="filepath", label="Recording")
        with gr.Column():
            min_snr_db = gr.Slider(0, 30, value=10, step=1, label="Minimum SNR (dB)")
            min_flatness = gr.Slider(0, 0.05, value=0.005, step=0.001, label="Minimum spectral flatness")
            run = gr.Button("Inspect", variant="primary")
    summary = gr.Markdown()
    plot = gr.Plot(label="Clips")
    table = gr.Dataframe(headers=COLUMNS, label="Clips", interactive=False)

    run.click(inspect, [audio, min_snr_db, min_flatness], [plot, table, summary])

if __name__ == "__main__":
    demo.launch()
