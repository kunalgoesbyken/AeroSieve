# Do AeroSieve's rejections predict ASR errors on real Hindi phone speech?

SieveBench shows AeroSieve catches defects that were injected on purpose. This experiment asks a question about real audio instead: are the utterances AeroSieve rejects actually harder for speech recognition? No model is trained; two pretrained models transcribe every utterance and their errors are compared across AeroSieve's verdicts.

## Setup

- **Audio:** Gram Vaani Hindi Dev + Eval ([OpenSLR 118](https://openslr.org/118)), 2,917 utterances, 7.8 h of spontaneous phone-collected speech from Bihar, Jharkhand, Madhya Pradesh and Uttar Pradesh. 60% was recorded at 8 kHz. These two splits have transcripts verified twice by humans, so a high error rate reflects the audio, not a bad label. The corpus is free for academic use; the audio is not redistributed here.
- **AeroSieve:** each utterance is scored whole, without re-segmenting, with `aerosieve.inspect(path, whole=True)` and the default policy.
- **Recognizers:** `openai/whisper-large-v3` and `facebook/mms-1b-all` (Hindi adapter), greedy decoding, no fine-tuning, on Kaggle's free T4 GPUs (about 45 minutes).
- **Scoring:** corpus-level WER and CER after NFC, the indic-nlp Hindi normalizer, tag and punctuation removal (vowel signs are kept). 95% confidence intervals come from 10,000 bootstrap resamples of utterances. The two utterances over 30 s are excluded.

## Results

| | Whisper-large-v3 | MMS-1B |
|---|---|---|
| WER on utterances AeroSieve accepts | 55.7% | 52.0% |
| WER on utterances it rejects | 64.6% | 60.6% |
| Gap (95% CI) | +8.9 points (+2.2 to +16.9) | +8.6 points (+5.5 to +11.6) |
| Gap within 8 kHz audio only | +7.6 (+1.1 to +15.3) | +6.6 (+3.5 to +9.8) |

- **Rejected utterances are harder for both models,** and the gap holds within narrowband audio alone, so it is not just "phone audio is harder".
- **The signal comes from spectral flatness.** The 142 utterances still rejected as `tonal` are the hardest group (69.3% and 66.7% WER).
- **The SNR check does not predict difficulty here.** All 58 `low_snr` rejections are wideband, half with a music bed, and they are easier than average (52.6% and 44.4% WER). Within each channel, WADA-SNR barely correlates with per-utterance WER (Spearman between −0.12 and +0.01).
- **AeroSieve barely tracks human labels.** It rejects 8.3% of utterances annotators marked `inaudible` and 6.1% of unmarked ones.

## What it found in AeroSieve

Before the fix, AeroSieve rejected 38% of the 8 kHz utterances as `tonal` and kept only 75% of the audio. Low-bitrate codecs cut narrowband speech off near 3 kHz, about 35 dB down, and those empty bins at the top of the 300 to 3400 Hz flatness band dragged the geometric mean down until speech read as a tone. Flatness now leaves out band edges whose long-term level sits more than 30 dB below the band's median. Retention went from 75% to 93%; tones and hum still score 0, and every SieveBench score is unchanged.

The 523 utterances the fix stopped rejecting are band-limited rather than tonal, and somewhat harder than other narrowband audio for Whisper (+6.0 points, CI +2.4 to +9.7) but not clearly for MMS (+1.6, CI −0.1 to +3.3). Whether dropping such audio helps a trained model is the question [`PLAN.md`](PLAN.md) sets up; that training run has not been done.

Full tables, including per-reason, per-channel, SNR-quintile and human-label breakdowns for the code before and after the fix, are in [`results/report.md`](results/report.md).

## Reproduce

```sh
uv venv .venv --python 3.12
uv pip install --python .venv ../../bindings/python soundfile numpy jiwer indic-nlp-library
.venv/Scripts/python prepare.py --gv-root <gram-vaani-dir> --splits GV_Dev_5h GV_Eval_3h --out <prepared-dir>
.venv/Scripts/python sieve.py --data <prepared-dir>
```

Upload `<prepared-dir>` (audio and `manifest.jsonl`) as a private Kaggle dataset, point `kaggle/kernel-metadata.json` at it, and run `kaggle kernels push -p kaggle`. Then:

```sh
.venv/Scripts/python report.py --data <prepared-dir> --hyps hyps_whisper.jsonl hyps_mms.jsonl --out results
```

`prepare.py` needs `ffmpeg` and `ffprobe` on the path. Building the Python binding on Windows needs the MSVC Rust toolchain.
