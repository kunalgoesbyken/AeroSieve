---
layout: ../../layouts/Docs.astro
---

## Install

The bindings build from source, so you need a Rust toolchain. Work in a virtual environment.

```bash
pip install maturin
cd bindings/python
maturin develop --release
```

The module is `aerosieve`. It has two functions, `inspect` and `curate`, and a `__version__` string. Both release the Python GIL while they work.

## inspect

Segments and scores one recording without writing anything. Use it to see how the filter judges your audio.

```python
import aerosieve

result = aerosieve.inspect("recording.wav")
for clip in result["clips"]:
    print(clip["start_s"], clip["duration_s"], clip["wada_snr_db"], clip["verdict"])
```

Signature:

```python
aerosieve.inspect(path, policy=None, *, max_duration_s=None, envelope_points=None, whole=False)
```

| Argument | Meaning |
| :--- | :--- |
| `path` | Audio file to read. |
| `policy` | Dict of threshold overrides. Keys you leave out keep their defaults. |
| `max_duration_s` | Refuse recordings longer than this, while they are being decoded. Use it for files from someone else. |
| `envelope_points` | Add an `envelope` of about this many peak values, for drawing a waveform. |
| `whole` | Score the file as one clip instead of segmenting it. |

### Return value

A dict:

| Key | Type | Meaning |
| :--- | :--- | :--- |
| `path` | str | The path you passed. |
| `duration_s` | float | Length of the recording. |
| `source_sample_rate` | int | Sample rate of the file before it was converted to 16 kHz. |
| `applied_gain_db` | float | Loudness gain applied while decoding. |
| `clips` | list of dict | One entry per clip. |
| `envelope` | dict or None | `{"step_s": float, "peaks": [float, ...]}` when `envelope_points` is set. `step_s` is the time covered by each peak. |

Each clip is a dict with `start_s`, `duration_s`, the four measurements `wada_snr_db`, `rms_dbfs`, `clip_ratio` and `spectral_flatness`, and `verdict`. The verdict is `"accept"` or the reject reason: `silence`, `too_quiet`, `too_loud`, `clipping`, `tonal` or `low_snr`. See [How clips are judged](/docs/filtering).

### Thresholds

```python
aerosieve.inspect("recording.wav", policy={"min_snr_db": 15.0})
```

The keys are `min_snr_db`, `max_clip_ratio`, `min_dbfs`, `max_dbfs` and `min_spectral_flatness`. An unknown key or a value of the wrong type raises `ValueError`.

### Files from someone else

```python
result = aerosieve.inspect("upload.mp3", max_duration_s=600, envelope_points=2000)
```

### Pre-cut utterances

For a corpus already cut and transcribed per utterance, score each file as one clip so it stays matched to its transcript.

```python
clip = aerosieve.inspect("utt-0001.flac", whole=True)["clips"][0]
keep = clip["verdict"] == "accept"
```

Without `whole=True` the recording is re-segmented, and an utterance shorter than 3 s produces no clip at all. With it there is always exactly one clip: pauses and edge silence count towards its measurements, and digital silence comes back as `silence`.

A file that cannot be decoded, or that exceeds `max_duration_s`, raises `RuntimeError`.

## curate

Writes the accepted clips of many recordings to WebDataset shards.

```python
stats = aerosieve.curate(
    ["a.wav", "b.mp3"],
    "out/run-1",
    transcripts=["hello there", None],
    rules="crates/aerosieve-lexical/rules/hinglish.yaml",
)
print(stats["segments_accepted"], "of", stats["segments_found"], "clips kept")
```

Signature:

```python
aerosieve.curate(files, out_dir, *, transcripts=None, rules=None, policy=None, staging_dir=None)
```

| Argument | Meaning |
| :--- | :--- |
| `files` | List of audio file paths. Directories are not expanded. |
| `out_dir` | Directory that receives the run. It must not already hold one. |
| `transcripts` | One entry per file, a string or `None`. A different length raises `ValueError`. |
| `rules` | YAML ruleset used to normalize transcripts. |
| `policy` | Dict of threshold overrides, as for `inspect`. |
| `staging_dir` | Scratch directory on the same volume as `out_dir`. Defaults to `out_dir/.staging`. |

`out_dir` receives `shard-*.tar`, `manifest.jsonl` and `stats.json`, in the [layout the command line writes](/docs/cli#output-layout). Files are processed on every core with the same result as processing them in order.

Unlike the command line, `curate` does not read `.txt` files beside the recordings. Pass `transcripts` instead.

### Return value

The run's totals as a dict:

| Key | Type | Meaning |
| :--- | :--- | :--- |
| `files_processed` | int | Files that decoded and were curated. |
| `files_failed` | int | Files that could not be decoded. They are counted, not raised. |
| `segments_found` | int | Clips the segmenter found. |
| `segments_accepted` | int | Clips written to shards. |
| `rejections` | dict | Count of rejected clips by reason, for example `{"low_snr": 3}`. Reasons with no rejections are absent. |
| `speech_duration_s` | float | Total duration of every clip found. |
| `accepted_duration_s` | float | Total duration of the accepted clips. |
| `shard` | dict | `samples_written`, `shards_written`, `total_bytes` and `total_duration_s`. |

The share of detected speech that was kept, by duration, is `accepted_duration_s / speech_duration_s`.
