# aerosieve-cli

The `aerosieve` command-line tool.

```bash
cargo install aerosieve-cli
```

| Command | What it does |
|---|---|
| `aerosieve inspect <paths>...` | Print each clip's metrics and verdict, then totals and percentiles. Writes nothing. `--summary-only` skips the per-clip lines. |
| `aerosieve curate <paths>... --out <dir>` | Write accepted clips to WebDataset shards in `<dir>`, with `manifest.jsonl` and `stats.json`. |
| `aerosieve nemo-manifest <dir>` | Add `nemo_manifest.jsonl` to a finished run: `audio_filepath`, `duration`, `text`, and `shard_id` per clip. |

Paths may be files or directories. Directories are searched recursively for common audio extensions; a file named explicitly is always tried.

Options shared by `inspect` and `curate`:

- `--policy <file>`: JSON overriding filter thresholds, for example `{"min_snr_db": 15}`. Unknown keys are an error.

Options for `curate`:

- `--rules <file>`: YAML ruleset for normalizing transcripts.
- `--staging <dir>`: scratch directory on the same volume as `--out`. Defaults to `<out>/.staging`.

A transcript is read from a `.txt` file beside the recording with the same name. Files that cannot be decoded are reported on stderr and skipped.

The NeMo manifest follows NeMo's documented tarred-dataset fields but has not been loaded by NeMo itself.
