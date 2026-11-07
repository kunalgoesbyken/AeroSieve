# aerosieve-sievebench

SieveBench measures how well AeroSieve's filters catch defects of known kinds and levels.

It takes a directory of clean speech, damages every recording under 28 conditions, and runs each damaged copy through the same decode, segment and score code as `aerosieve curate`:

| Family | Conditions | What a good filter does |
| :--- | :--- | :--- |
| Control | untouched, phone line (8 kHz, G.711 μ-law) | keep |
| Noise | white, pink and babble at 30, 20, 15, 10, 5 and 0 dB SNR | keep at 20 dB and up, reject at 5 dB and down |
| Clipping | overdriven ×2, ×4, ×8, ×16 | reject when over 2% of samples clip, keep under 0.5% |
| Level | turned down to −55 dBFS | reject |
| Tone | dial tone, 1 kHz test tone, 50 Hz hum | reject |

The expected outcome comes from the damage itself, never from AeroSieve's thresholds, so changing `FilterPolicy` cannot make the benchmark agree with itself. Levels between the keep and reject bounds are reported but not scored.

The JSON report has recall, precision and false rejects overall and per family, how often a rejection named the injected defect, a per-condition breakdown, and how far WADA-SNR reads from the injected SNR for each noise type.

## Run it

```bash
cargo run --release -p aerosieve-sievebench -- corpus/ --name Hindi --source "FLEURS dev split" --out report.json
```

`corpus/` is any directory of clean recordings (wav, flac, mp3, ogg, m4a). Babble is made from the corpus's other talkers, so it needs at least two.

To reproduce the reports on the site, which downloads about 1.4 GB of public CC BY 4.0 speech:

```bash
crates/aerosieve-sievebench/scripts/run_all.sh
```

It needs Python with `pyarrow` to unpack the Hugging Face parquet files.
