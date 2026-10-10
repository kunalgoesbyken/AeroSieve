# aerosieve (Python)

Python bindings for [AeroSieve](https://github.com/kunalgoesbyken/AeroSieve): segment recordings into speech clips, filter them on audio quality, and write the survivors as WebDataset shards.

## Install from source

Requires a Rust toolchain.

```bash
pip install maturin
cd bindings/python
maturin develop --release
```

## Inspect a recording

Nothing is written. Use this to see how the filter judges your audio.

```python
import aerosieve

result = aerosieve.inspect("recording.wav")
for clip in result["clips"]:
    print(clip["start_s"], clip["duration_s"], clip["wada_snr_db"], clip["verdict"])
```

`verdict` is `"accept"` or the rejection reason: `silence`, `too_quiet`, `too_loud`, `clipping`, `tonal`, or `low_snr`.

Thresholds can be overridden; keys you leave out keep their defaults:

```python
aerosieve.inspect("recording.wav", policy={"min_snr_db": 15.0})
```

## Curate a set of recordings

```python
stats = aerosieve.curate(
    ["a.wav", "b.mp3"],
    "out/run-1",
    transcripts=["hello there", None],
    rules="crates/aerosieve-lexical/rules/hinglish.yaml",
)
print(stats["segments_accepted"], "of", stats["segments_found"], "clips kept")
```

`out/run-1` receives `shard-*.tar`, `manifest.jsonl`, and `stats.json`. Each run needs its own output directory. Files that fail to decode are counted in `stats["files_failed"]`.
