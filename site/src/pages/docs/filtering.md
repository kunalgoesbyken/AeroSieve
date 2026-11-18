---
layout: ../../layouts/Docs.astro
---

## What reaches the filter

Before a clip is judged, the audio is prepared in three steps. Knowing them explains most surprising verdicts.

1. **Decode.** Any supported file is converted to mono at 16 kHz. Loudness is then leveled toward -20 dBFS RMS, with the gain limited to 3 dB in either direction. A quiet recording is lifted by at most 3 dB, and its noise floor with it.
2. **Segment.** A speech detector cuts clips of 3 to 30 seconds. By default it works from signal level, with its midpoint at -45 dBFS, so it measures energy rather than speech: loud music or machinery counts as speech. A pause has to last 500 ms to end a clip.
3. **Score.** Each clip gets four measurements, and a policy compares them with thresholds.

The batch tools measure after leveling. The [streaming server](/docs/streaming) and `LiveInspector` do not level, so a microphone set very low is rejected as `too_quiet` there rather than turned up.

## The four measurements

| Measurement | What it is |
| :--- | :--- |
| `wada_snr_db` | Signal-to-noise ratio estimated from the amplitude distribution of the clip alone, with no clean reference (WADA-SNR, Kim and Stern, Interspeech 2008). It lies in -20 to 100 dB. |
| `rms_dbfs` | RMS level relative to full scale. Digital silence is negative infinity. |
| `clip_ratio` | Fraction of samples at or beyond 0.999 of the source's own full scale. |
| `spectral_flatness` | Wiener entropy over the 300 to 3400 Hz speech band: near 1 is noise-like, near 0 is tonal. |

**WADA-SNR** models clean speech as a Gamma(0.4) distribution and noise as Gaussian, so a signal that is not speech-like, such as a pure tone, reads as low SNR however clean it is. It needs at least 256 non-zero samples; below that it reports negative infinity, so any SNR floor rejects the clip. It reads a few dB low on real speech.

**Clipping** is measured against the level the source's full scale sits at after leveling, not against 1.0. Loudness leveling can turn a clipped file down, and measuring against 1.0 would hide the clipping.

**Flatness** is limited to the speech band because resampled audio has empty bins near the Nyquist frequency, which would otherwise make speech read as a tone. It is computed on non-overlapping 1,024-sample frames and averaged; a trailing partial frame is ignored, and a clip shorter than one frame reads 0.0.

### The band edge rule

Telephone audio often stops short of the band. Low-bitrate codecs cut speech off near 3 kHz, and the empty top of the band would read as a tone. Before the rule existed, this rejected 38% of the 8 kHz Gram Vaani Hindi utterances as `tonal`; retention on them rose from 75% to 93% with it.

So the edges of the band are trimmed first:

- Each bin's power is summed over the clip's frames, giving its long-term level.
- Bins at either end of the band whose level is more than 30 dB below the band's median level are left out. A bin exactly 30 dB below is kept.
- Only the ends are trimmed. A dip in the middle of the band stays in.
- The 30 dB figure is relative to the median, so at most half the band can be trimmed. A pure tone has the noise floor as its median bin, so it keeps the whole band and still reads as tonal.

The constant is `CARRIED_BAND_FLOOR_DB` in `aerosieve-acoustic`.

## The policy

`FilterPolicy` holds five thresholds. Every field has a default, and a policy file or dict only needs the ones you change.

| Field | Default | Rejects a clip when | Reason |
| :--- | ---: | :--- | :--- |
| no field | n/a | the level cannot be measured: digital silence or an empty buffer | `silence` |
| `min_dbfs` | -40 | `rms_dbfs` is below it | `too_quiet` |
| `max_dbfs` | -3 | `rms_dbfs` is above it | `too_loud` |
| `max_clip_ratio` | 0.01 | `clip_ratio` is above it | `clipping` |
| `min_spectral_flatness` | 0.005 | `spectral_flatness` is below it | `tonal` |
| `min_snr_db` | 10 | `wada_snr_db` is below it, or is not a number | `low_snr` |

Comparisons are strict, so a clip exactly at a threshold passes.

### Order of checks

The checks run in the order of the table, and the first failure is the reported reason. The order goes from the most specific cause to the least:

- A silent clip reports `silence` rather than `low_snr`.
- A pure tone reports `tonal` even though WADA also scores it as noise.
- A clip that is both too loud and clipped reports `too_loud`, because level is checked before clipping.

A clip with several problems therefore shows only the first. To see every measurement, run `inspect`, which prints all four for each clip.

## Choosing thresholds

The defaults come from a small set of real recordings. Clean speech read 13 dB or more on WADA-SNR and 0.02 to 0.18 on flatness, while 13 of 14 music clips read 8 dB or less on WADA-SNR, with flatness overlapping speech. So the SNR floor is what rejects most music, and the flatness floor catches tones and hum. The SNR default of 10 dB keeps speech with roughly 15 dB or more of true SNR, because WADA reads low.

Treat them as a starting point, not a verdict on your data.

- **Check what the floor removes.** On real Hindi phone speech, the SNR floor rejected music-bed recordings that speech recognizers handled well. If your corpus has background music or sustained noise that you can live with, a fixed SNR floor may throw away usable audio.
- **Do not assume SNR predicts recognition errors.** In `experiments/asr-hi`, 2,917 Hindi phone utterances with human-verified transcripts were transcribed by Whisper-large-v3 and MMS-1B. The utterances AeroSieve rejected had 8.6 to 8.9 points higher word error rate, and that gap came from the tonality check. The SNR check did not predict errors there. No model has yet been trained on the kept audio to show it does better.
- **Expect some gaps.** A single sustained instrument can pass as speech, and babble noise reads cleaner than it is.

### A workflow

1. Run `aerosieve inspect --summary-only` on a representative sample. The percentile rows show where your data sits against each threshold.
2. Look at the rejections by reason. A large `low_snr` count on audio that sounds fine means the floor is too high for it.
3. Listen to clips near the threshold, not only the extremes.
4. Write the thresholds to a JSON file and run `inspect --policy` again, comparing the totals.
5. Use the same file with `curate --policy`.

```bash
aerosieve inspect recordings/ --summary-only
echo '{"min_snr_db": 6}' > policy.json
aerosieve inspect recordings/ --summary-only --policy policy.json
aerosieve curate recordings/ --out data/run-1 --policy policy.json
```

The [browser demo](/#demo) has a slider for each threshold and re-judges the clips without decoding the file again, which is a quick way to build intuition on a few recordings.

## Silero VAD

The default detector measures energy, so it cannot tell speech from other loud sounds. That affects both which regions become clips and whether long pauses in a noisy recording split a clip.

For a real speech detector, use the Silero VAD v5 option: build the Rust library with the `silero` feature and pass a `SileroDetector` to `Pipeline::with_detector`. It supports 8 kHz and 16 kHz audio. Setup and code are on the [Rust page](/docs/rust#silero-vad). The command-line tool and the Python bindings do not expose it yet.

It changes where clips are cut. It does not change how a clip is judged: the same four measurements and the same policy apply.
