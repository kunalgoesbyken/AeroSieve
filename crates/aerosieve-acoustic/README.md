# aerosieve-acoustic

Deterministic acoustic quality scoring for mono `f32` audio. No models and no randomness: the same buffer always scores the same.

`score` measures; `FilterPolicy` judges. They are separate so thresholds can be tuned without touching the measurements.

```rust
use aerosieve_acoustic::{FilterPolicy, score};

let metrics = score(&samples, 16_000);
match FilterPolicy::default().evaluate(&metrics) {
    None => println!("accepted"),
    Some(reason) => println!("rejected: {reason}"),
}
```

## Metrics

| Metric | Meaning |
|---|---|
| `wada_snr_db` | SNR estimated from the amplitude distribution alone (WADA; Kim & Stern, Interspeech 2008), in `[-20, 100]` dB |
| `rms_dbfs` | RMS level relative to full scale |
| `clip_ratio` | Fraction of samples at or beyond ±0.999 |
| `spectral_flatness` | Wiener entropy over the 300–3400 Hz speech band: near 1 is noise-like, near 0 is tonal |

WADA models clean speech as Gamma(0.4) and noise as Gaussian. A signal that is not speech-like, such as a pure tone, reads as low SNR however clean it is. `src/wada_table.rs` documents how the lookup table was generated.

Flatness is restricted to the speech band because resampled audio has empty bins near Nyquist, which would otherwise make speech read as a tone.

## Policy

`FilterPolicy::evaluate` returns the first failed check, ordered from the most specific cause to the least:

| Check | Default | Reason |
|---|---|---|
| level is measurable | — | `silence` |
| `min_dbfs` | −40 | `too_quiet` |
| `max_dbfs` | −3 | `too_loud` |
| `max_clip_ratio` | 0.01 | `clipping` |
| `min_spectral_flatness` | 0.02 | `tonal` |
| `min_snr_db` | 10 | `low_snr` |

The defaults are calibrated on the synthetic signals in `aerosieve-fixtures`, not on real recordings.
