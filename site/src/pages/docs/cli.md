---
layout: ../../layouts/Docs.astro
---

## Install

```bash
cargo install aerosieve-cli
```

This installs one binary, `aerosieve`, with three commands.

| Command | What it does |
| :--- | :--- |
| `aerosieve inspect <PATHS>...` | Print each clip's metrics and verdict, then totals and percentiles. Writes nothing. |
| `aerosieve curate <PATHS>... --out <DIR>` | Write accepted clips to WebDataset shards, with a manifest and totals. |
| `aerosieve nemo-manifest <RUN_DIR>` | Add a NeMo-style manifest to a finished `curate` run. |

On failure the tool prints `error: ...` to stderr and exits with a non-zero status.

## Input paths

Both `inspect` and `curate` take one or more paths.

- A file is used as given, whatever its extension.
- A directory is searched recursively for files ending in `aac`, `caf`, `flac`, `m4a`, `mkv`, `mp3`, `mp4`, `oga`, `ogg`, `opus`, `wav` or `webm`, in any letter case.
- The list is sorted and de-duplicated, so a run is repeatable.
- A file that cannot be decoded is reported on stderr as `skipped` and counted in the totals. The run continues.

## inspect

```bash
aerosieve inspect [--policy <FILE>] [--summary-only] <PATHS>...
```

| Flag | Meaning |
| :--- | :--- |
| `--policy <FILE>` | JSON file overriding filter thresholds. See [Policy file](#policy-file). |
| `--summary-only` | Print only the totals, not one line per clip. |

Real output for a 70 s recording of clean speech, then the same speaker under noise, clipped, and over a dial tone:

```text
mixed.ogg (70.6 s, 16000 Hz source)
    START  LENGTH   SNR dB     dBFS   CLIP  FLATNESS  VERDICT
     0.5s    4.9s     15.6    -22.1  0.000     0.093  accept
     7.3s    9.1s      1.9    -20.8  0.000     0.344  low_snr
    17.8s    9.3s     37.1     -7.8  0.134     0.164  clipping
    28.5s    5.1s      4.1    -29.1  0.000     0.000  tonal
    35.2s   18.2s     21.3    -22.9  0.000     0.169  accept
    55.1s   12.4s     29.1    -23.3  0.000     0.125  accept

1 files inspected, 0 skipped
3 of 6 clips accepted
  rejected as clipping   1
  rejected as tonal      1
  rejected as low_snr    1
  PERCENTILE       p5      p50      p95
  SNR dB        1.947   15.647   37.083
  dBFS        -29.132  -22.912   -7.752
  flatness      0.000    0.125    0.344
```

The percentile rows are nearest-rank values over every clip, which is the quickest way to see where your thresholds sit relative to your data.

## curate

```bash
aerosieve curate --out <DIR> [--policy <FILE>] [--rules <FILE>] [--staging <DIR>] <PATHS>...
```

| Flag | Meaning |
| :--- | :--- |
| `--out <DIR>` | Required. Directory to write shards, `manifest.jsonl` and `stats.json` into. |
| `--policy <FILE>` | JSON file overriding filter thresholds. |
| `--rules <FILE>` | YAML ruleset used to normalize transcripts. |
| `--staging <DIR>` | Scratch directory on the same volume as `--out`. Defaults to `<out>/.staging`. |

Each run needs its own output directory. A directory that already holds shards, a manifest or stats is refused, so a second run cannot overwrite the first.

The command prints how many files were curated, how many clips were accepted (and what share of the detected speech that is, by duration), the rejections by reason, and the audio written. See [Get started](/docs) for sample output.

Files are decoded, segmented and scored on every core, then written in input order. The shards are identical to a single-threaded run. To limit the threads used, set `RAYON_NUM_THREADS`:

```bash
RAYON_NUM_THREADS=4 aerosieve curate recordings/ --out data/run-1
```

### Output layout

```text
data/run-1/
  shard-000000.tar
  manifest.jsonl
  stats.json
```

Inside a shard, each clip is two adjacent members that share a key:

```text
000000000.wav
000000000.json
000000001.wav
000000001.json
```

- Keys are sequential and zero-padded, so they sort in write order across shards.
- Audio is mono 16-bit PCM at 16 kHz.
- A shard is closed once it reaches about 1 GB, and a clip never straddles two shards.
- Shards are built in the staging directory and renamed into place, so a reader of the output directory only ever sees complete tars.

Each `{key}.json` holds the same record as one line of `manifest.jsonl`:

```json
{"key":"000000000","shard":"shard-000000.tar","duration_s":4.924,"sample_rate":16000,"transcript":"","source_file":"recordings/clean.ogg","clip_ratio":0.0,"rms_dbfs":-18.88,"spectral_flatness":0.0931,"wada_snr_db":15.65}
```

`stats.json` holds `samples_written`, `shards_written`, `total_bytes` and `total_duration_s` for the run.

An empty `.staging` directory is left inside the output directory.

### Transcripts

A transcript is read from a `.txt` file with the same name as the recording, in the same directory: `call-0412.txt` for `call-0412.wav`.

- The whole text is stored, unchanged or normalized by `--rules`, with **every** clip cut from that recording. There is no alignment and no speech recognition.
- A recording without a `.txt` file gets an empty transcript.
- Files over 1 MiB, or that are not regular files, are reported on stderr and the recording is skipped. Symbolic links are not followed.

For audio that is already cut per utterance, use [Python's `whole=True`](/docs/python#inspect) instead, so each file stays matched to its transcript.

### Transcript rules

`--rules` takes a YAML file of regex rules. Every rule is matched against the original text, not the output of an earlier rule, and the surviving matches are spliced in one pass. When matches overlap, the leftmost wins, then the longest, then the higher `priority`, then the rule listed first. Runs of whitespace are then collapsed.

```yaml
version: 2
rules:
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    action: Replace
    category: currency
    priority: 0
```

Only `pattern` is required. `action` is `Replace` (default), `Remove`, `Prefix` or `Suffix`. Unknown fields and unsupported versions are rejected when the file loads, and a ruleset that cannot be loaded stops the run rather than leaving transcripts half-normalized.

Patterns use Rust's `regex` syntax: no backreferences and no lookaround. Write `${1}s`, not `$1s`, when a capture is followed by a letter.

The repository ships `crates/aerosieve-lexical/rules/hinglish.yaml`, which normalizes romanized Hindi-English transcripts. It is one language's ruleset, not a default.

## Policy file

`--policy` takes a JSON object with any of these keys. Keys you leave out keep their defaults, and an unknown key is an error.

```json
{
  "min_snr_db": 15,
  "max_clip_ratio": 0.01,
  "min_dbfs": -40,
  "max_dbfs": -3,
  "min_spectral_flatness": 0.005
}
```

What each threshold does, and how to choose one, is in [How clips are judged](/docs/filtering). The same file works for `inspect` and `curate`, so try a policy with `inspect --summary-only` before spending a run on it.

## nemo-manifest

```bash
aerosieve nemo-manifest <RUN_DIR>
```

Reads `manifest.jsonl` in a finished run and writes `nemo_manifest.jsonl` beside it, one JSON object per clip:

```json
{"audio_filepath":"000000000.wav","duration":4.924,"text":"","shard_id":0}
```

`audio_filepath` is the clip's name inside its shard. `shard_id` numbers the shards in the order they first appear in the manifest.

The fields follow NeMo's documented layout for tarred datasets. The file has not been loaded by NeMo itself.

## Not exposed by the CLI

The command line uses the default segmentation settings and the energy-based speech detector. To change clip lengths, the pause that ends a clip, or to use Silero VAD, use the [Rust library](/docs/rust).
