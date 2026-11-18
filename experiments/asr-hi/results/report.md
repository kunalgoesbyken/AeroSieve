# AeroSieve verdicts vs ASR errors on Gram Vaani Hindi

Gram Vaani Dev + Eval (OpenSLR 118), transcripts verified twice by humans. Each utterance is scored whole by AeroSieve (`inspect(..., whole=True)`, default policy) and transcribed by pretrained models with no fine-tuning. Utterances over 30 s or with an empty normalized reference are excluded. WER is corpus-level (total errors over total reference words); CIs come from 10,000 bootstrap resamples of utterances.

## openai/whisper-large-v3

2915 utterances, overall WER 56.3%, CER 33.5%.

### Accepted vs rejected (current)

| group | utterances | hours | WER | CER |
|---|---|---|---|---|
| accepted | 2714 | 7.27 | 55.7% | 33.1% |
| rejected: clipping | 1 | 0.00 | 8.7% | 6.9% |
| rejected: low_snr | 58 | 0.15 | 52.6% | 30.0% |
| rejected: tonal | 142 | 0.37 | 69.3% | 43.4% |
| rejected: all | 201 | 0.52 | 64.6% | 39.8% |

Rejected minus accepted WER: +8.9 points (95% bootstrap CI +2.2 to +16.9).

Within each channel:

| channel | accepted | rejected | accepted WER | rejected WER | gap, 95% CI |
|---|---|---|---|---|---|
| narrowband | 1612 | 142 | 61.7% | 69.3% | +7.6 (+1.1 to +15.3) |
| wideband | 1102 | 59 | 47.4% | 51.9% | +4.5 (-9.3 to +26.4) |

### Accepted vs rejected (v0.4.0)

| group | utterances | hours | WER | CER |
|---|---|---|---|---|
| accepted | 2191 | 5.86 | 53.2% | 31.0% |
| rejected: clipping | 1 | 0.00 | 8.7% | 6.9% |
| rejected: low_snr | 58 | 0.15 | 52.6% | 30.0% |
| rejected: tonal | 665 | 1.79 | 66.4% | 41.6% |
| rejected: all | 724 | 1.93 | 65.3% | 40.7% |

Rejected minus accepted WER: +12.1 points (95% bootstrap CI +8.8 to +15.6).

Within each channel:

| channel | accepted | rejected | accepted WER | rejected WER | gap, 95% CI |
|---|---|---|---|---|---|
| narrowband | 1089 | 665 | 59.6% | 66.4% | +6.8 (+3.4 to +10.3) |
| wideband | 1102 | 59 | 47.4% | 51.9% | +4.5 (-9.3 to +26.2) |

### WER by WADA-SNR quintile

| quintile | WADA-SNR dB | utterances | WER |
|---|---|---|---|
| Q1 | -3.4 to 16.4 | 583 | 48.0% |
| Q2 | 16.5 to 24.5 | 583 | 53.4% |
| Q3 | 24.5 to 34.7 | 583 | 61.6% |
| Q4 | 34.7 to 59.1 | 583 | 61.2% |
| Q5 | 59.4 to 100.0 | 583 | 57.7% |

Spearman correlation of WADA-SNR with per-utterance WER: 0.240.

### Human recording-problem labels

| human label | utterances | WER | AeroSieve rejects |
|---|---|---|---|
| audio_jump | 200 | 56.4% | 5.0% |
| audio_jump,inaudible | 82 | 61.9% | 9.8% |
| audio_jump,music | 39 | 52.5% | 12.8% |
| inaudible | 518 | 64.4% | 8.3% |
| inaudible,music | 21 | 39.8% | 4.8% |
| music | 202 | 46.2% | 11.4% |
| none | 1803 | 54.6% | 6.1% |
| not_clear | 16 | 77.7% | 6.2% |

## facebook/mms-1b-all

2915 utterances, overall WER 52.6%, CER 24.3%.

### Accepted vs rejected (current)

| group | utterances | hours | WER | CER |
|---|---|---|---|---|
| accepted | 2714 | 7.27 | 52.0% | 23.9% |
| rejected: clipping | 1 | 0.00 | 26.1% | 12.6% |
| rejected: low_snr | 58 | 0.15 | 44.4% | 19.1% |
| rejected: tonal | 142 | 0.37 | 66.7% | 32.5% |
| rejected: all | 201 | 0.52 | 60.6% | 28.9% |

Rejected minus accepted WER: +8.6 points (95% bootstrap CI +5.5 to +11.6).

Within each channel:

| channel | accepted | rejected | accepted WER | rejected WER | gap, 95% CI |
|---|---|---|---|---|---|
| narrowband | 1612 | 142 | 60.0% | 66.7% | +6.6 (+3.5 to +9.8) |
| wideband | 1102 | 59 | 40.9% | 44.1% | +3.2 (-2.1 to +8.4) |

### Accepted vs rejected (v0.4.0)

| group | utterances | hours | WER | CER |
|---|---|---|---|---|
| accepted | 2191 | 5.86 | 49.7% | 22.5% |
| rejected: clipping | 1 | 0.00 | 26.1% | 12.6% |
| rejected: low_snr | 58 | 0.15 | 44.4% | 19.1% |
| rejected: tonal | 665 | 1.79 | 62.3% | 30.1% |
| rejected: all | 724 | 1.93 | 61.0% | 29.3% |

Rejected minus accepted WER: +11.2 points (95% bootstrap CI +9.7 to +12.8).

Within each channel:

| channel | accepted | rejected | accepted WER | rejected WER | gap, 95% CI |
|---|---|---|---|---|---|
| narrowband | 1089 | 665 | 59.5% | 62.3% | +2.8 (+1.2 to +4.5) |
| wideband | 1102 | 59 | 40.9% | 44.1% | +3.2 (-2.1 to +8.5) |

### WER by WADA-SNR quintile

| quintile | WADA-SNR dB | utterances | WER |
|---|---|---|---|
| Q1 | -3.4 to 16.4 | 583 | 42.3% |
| Q2 | 16.5 to 24.5 | 583 | 47.8% |
| Q3 | 24.5 to 34.7 | 583 | 57.8% |
| Q4 | 34.7 to 59.1 | 583 | 59.5% |
| Q5 | 59.4 to 100.0 | 583 | 56.7% |

Spearman correlation of WADA-SNR with per-utterance WER: 0.309.

### Human recording-problem labels

| human label | utterances | WER | AeroSieve rejects |
|---|---|---|---|
| audio_jump | 200 | 47.9% | 5.0% |
| audio_jump,inaudible | 82 | 57.9% | 9.8% |
| audio_jump,music | 39 | 45.6% | 12.8% |
| inaudible | 518 | 62.4% | 8.3% |
| inaudible,music | 21 | 43.3% | 4.8% |
| music | 202 | 38.2% | 11.4% |
| none | 1803 | 51.7% | 6.1% |
| not_clear | 16 | 69.8% | 6.2% |
