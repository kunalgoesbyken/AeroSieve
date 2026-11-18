# Does AeroSieve-filtered data train a better Hindi ASR model?

Status: draft. Frozen and pushed as a public tag before any AeroSieve scoring of the training pool (step 4). Arm manifest hashes are pushed in a second tag before the first Kaggle run.

## Question

AeroSieve catches injected defects (SieveBench). It has never been shown to improve a downstream model. This experiment fine-tunes Whisper on real, messy Hindi speech with and without AeroSieve selection and measures WER/CER, using only free compute.

## Prerequisite: score whole utterances

`Inspector::inspect_audio` always segments (3–30 s, splits at 500 ms pauses, drops runs under 3 s) and `Pipeline::write` copies one transcript to every clip. Pre-cut ASR utterances therefore lose alignment, and utterances under 3 s are silently unscored.

Change:
- `crates/aerosieve/src/lib.rs`: `Inspector::score_whole(&self, source, audio) -> FileOutcome` judging the whole buffer as one segment, reusing `judge()`, `full_scale()` and `ensure_matching_rates`.
- `bindings/python/src/lib.rs`: `inspect(..., whole: bool = False)`, routing to `score_whole`.
- `crates/aerosieve/src/tests.rs`: 2 s clean clip gives one accepted clip; silence gives one `silence` clip (segmented mode gives zero); a hot clipped file stays `clipping` after normalization.
- README (Python bindings) and CHANGELOG lines. CLI unchanged.

AeroSieve is used only as a selector. Every arm trains on the same audio with the same transcripts; selection is the only difference. AeroSieve scores exactly the 16 kHz FLAC that Whisper trains on. Any AeroSieve failure on a RAW file is a bug to fix, never a rejection.

## Data

Training pool: Gram Vaani Hindi, OpenSLR SLR118 (https://openslr.org/118). Spontaneous phone-collected speech from Bihar, Jharkhand, MP and UP; source rates 8 kHz 60.9%, 44.1 kHz 34.5%, 48 kHz 3.6%, 16 kHz 0.8%. Crowd-sourced training transcripts of varying accuracy. Academic-use license: Kaggle datasets stay private, audio is never redistributed.
- `GV_Train_100h.tar.gz` (37,152 utterances, 100.9 h), `GV_Dev_5h.tar.gz`, `GV_Eval_3h.tar.gz`, `Metadata.tar.gz`.
- Metadata fields are unverified. Inspect in step 3; the metadata agreement table and speaker/district counts are dropped if the fields do not exist.

Channel stratum: narrowband (source rate 8 kHz) vs wideband (everything else), read with ffprobe in `prepare.py`.

Held-out train slice: 300 utterances removed from TRAIN before anything else (speaker-disjoint if metadata allows). Used for the pilot, the N choice and training curves. Never part of any arm or test.

Test sets (never filtered, never used for any decision):
1. Primary: GV Dev + Eval combined (2,917 utterances, about 7.8 h). GV Eval also reported on its own next to published anchors (TDNN 29.7%, Conformer 32.9%, IndicWhisper 26.8%), stated as not directly comparable (TDNN uses an LM; IndicWhisper trained on Vistaar data that includes Gram Vaani).
2. Kathbath Hindi test (Vistaar `kathbath.zip`; split and folder layout confirmed in step 3 and named in the report).
3. Kathbath-Hard Hindi (Vistaar `kathbath_noisy.zip`), paired with 2 to measure noise robustness.
4. Lahaja (`ai4bharat/Lahaja`), fixed 2,000-utterance subset, seed 0.

## Shared preprocessing (all arms, before any split)

Drop utterances over 30 s, labels over 444 tokens (448 minus 4 prefix tokens), empty or unparseable transcripts, decode failures. Strip noise/bracket tags identically everywhere (GV conventions checked first). Convert to 16 kHz mono FLAC once with the ffmpeg CLI. Remainder minus the held-out slice is RAW.

## Selection rule (fixed before scoring)

1. Score RAW with `inspect(..., whole=True)`, default FilterPolicy.
2. If the non-SNR checks alone (`too_quiet`, `too_loud`, `clipping`, `tonal`, `silence`) remove more than 40% of hours, stop: the defaults do not fit whole-utterance phone audio and the corpus or design is rethought before any GPU time.
3. Otherwise B is everything the default policy accepts, if that is between 40% and 90% of RAW hours.
4. If below 40%: B is the highest-WADA utterances among those passing every non-SNR check, up to 60% of RAW hours.
5. If above 90%: same ranking, up to 75% of RAW hours, and AeroSieve is described as a ranking signal.

The manual audit is descriptive only and cannot change the rule or the policy.

## Arms

Model `openai/whisper-base`, full fine-tune, identical hyperparameters and decoding for every arm, N steps each.

| Arm | Data | Seeds |
|---|---|---|
| A raw | all of RAW | 5 |
| B sieve | selected by the rule above | 5 |
| C random | random subset of RAW with B's hours, stratified by channel × 2 s duration bin to match B, redrawn per seed | 5 |
| W worst | lowest-WADA utterances of RAW with B's hours, same stratification | 3 |

W is a dose-response check: if quality selection matters, W should be worse than C. Same compute means same optimizer steps × same batch (Whisper pads every input to 30 s). Each run has its own full warmup + linear decay to 0. The last checkpoint is evaluated.

Hyperparameters: lr 5e-5, warmup 200, linear decay, AdamW, grad clip 1.0, fp16 AMP with fp32 master weights, no SpecAugment, no gradient checkpointing. Effective batch 32; on the T4 micro-batch 16–32 with no accumulation. Log-mel features computed on the GPU (or cached) so CPU is not the bottleneck.

N: from the pilot curve of arm A on the held-out slice, the step count near A's plateau, capped so one run takes at most 1.5 h on a T4 with both GPUs busy. Fixed before the second tag.

## Metrics and statistics

Normalizer (one function shared by every scorer, unit-tested on matras, nukta, danda, digits): NFC, indic-nlp-library Hindi normalizer, Devanagari digits to ASCII, remove tags, replace Unicode P* plus `।` `॥` with space (never touch category M), lowercase Latin, collapse whitespace. Whisper's `BasicTextNormalizer` is not used because it strips matras.

WER, CER and insertion rate with jiwer. Decoding: greedy, `language="hi"`, `task="transcribe"`, `max_new_tokens` = 1.5 × the longest test reference in tokens.

Pre-registered contrasts on the primary set:
- H1 (main): B vs C. Win requires (a) the 95% bootstrap CI of ΔWER entirely below 0 and at least 0.5 absolute, and (b) B_s − C_s negative for all 5 seeds s.
- H2: B vs A at equal compute, same two criteria.
- H3: W vs C, expected positive.

Bootstrap: 10,000 resamples over test utterances; in each, an arm's WER is total errors summed over its seeds divided by total reference words. Per-seed WER mean ± sd also reported.

Descriptive (pre-registered, no win claims):
- Each arm's WER on test utterances AeroSieve accepts vs rejects (test sets scored for analysis only).
- Each arm's WER per channel stratum.
- Kathbath-Hard minus Kathbath per arm.
- Zero-shot whisper-base on every set.
- Per arm: hours, utterance count, reference tokens per hour, channel mix, speaker/district counts if metadata has them.
- Overlap of AeroSieve rejections with a characters-per-second outlier heuristic (CPU only), so the "one-line heuristic" objection is answered.
- Compute: steps, samples seen, audio-hours, measured GPU-hours, share of hours removed.

## Outcomes and write-up

- Win: H1 met, plus H2 or a clear H3.
- Null: H1 not met. The stratified analyses (accepted vs rejected test utterances, per channel) say where filtering helped or hurt.
- Loss: B worse than C or A beyond 0.5 WER, or Kathbath-Hard degrades more for B. Published with what was removed by reason and channel.

Always stated: AeroSieve is acoustic-only and cannot catch mislabelled transcripts; pretrained Whisper is already noise-robust; one corpus, one language, one model size. Published with results: plan, arm manifests, per-utterance hypotheses and AeroSieve scores (no audio).

## Manual audit (descriptive)

50 rejected (stratified by reason and channel, at least 5 per reason) plus 50 kept training utterances, shuffled and blind (static HTML page with audio, transcript, radio buttons, CSV export). Rubric: usable for training; main defect (noise, clipping, silence/truncation including leading/trailing silence, music/tone, multiple speakers, none); transcript matches audio (logged separately). Rejection precision and kept-unusable rate with Wilson 95% CIs, per channel, and a reason-vs-human confusion table.

## Compute

Local GTX 1660 Ti (6 GB, TU116, no tensor cores, no bf16): preflight, smoke test, pilot. Known TU116 cuDNN fp16 NaN issue (pytorch#58123): preflight compares fp32 vs autocast encoder outputs and runs 20 finite-loss steps. Fallback: disable cuDNN, then fp32. The pilot is never part of the results.

Kaggle "GPU T4 ×2" for every reported run (P100 images are broken on sm_60). Two single-GPU queues per session (`CUDA_VISIBLE_DEVICES` per process, assert `n_gpu == 1`), arms interleaved across GPUs, train then evaluate in-session. Only complete runs are scheduled into a session; an unfinished run is rerun from scratch. Same platform and lockfile for every reported run.

Budget: 18 runs at up to 1.5 h on two GPUs in parallel, plus evaluation, about 15–18 quota hours over 2–3 sessions, inside the ~30 h weekly quota.

Data upload: RAW FLAC (about 6–7 GB) plus test sets as a few tar shards in one private Kaggle dataset; file counts asserted on Kaggle.

## Software (native Windows, no WSL)

Local:
```
uv venv .venv --python 3.12
uv pip install --python .venv torch==2.14.1 --index-url https://download.pytorch.org/whl/cu126
uv pip install --python .venv transformers accelerate soundfile jiwer numpy indic-nlp-library huggingface_hub pyarrow maturin
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"; uv pip install --python .venv D:\code\AEROSIEVE\bindings\python
```
Exact versions pinned in a lockfile after the first working install. Kaggle uses a fresh `uv venv` from the same lockfile minus `maturin` and `aerosieve` (no scoring happens there). torch 2.14.1 is the last release with CUDA 12.x wheels and driver 572.60 supports up to 12.8. Not installed: `datasets`, `torchaudio`, `torchcodec`, `webdataset`. Config via stdlib `tomllib`, profiles `local` and `kaggle`, no paths in code. Transformers v5: `processing_class=`, `generation_config.language="hi"`, `task="transcribe"`, `forced_decoder_ids=None`, `report_to="none"`, no torch.compile. Windows DataLoader: top-level collator, `__main__` guards.

## Layout

`experiments/asr-hi/`: `config.toml`, `common.py`, `prepare.py`, `sieve.py`, `preflight.py`, `train.py`, `evaluate.py`, `report.py`, `audit/`, `kaggle/run.py`, `kaggle/kernel-metadata.json`.

## Steps

1. AeroSieve `score_whole` + Python `whole=`, tests, build the wheel.
2. Environment and preflight on the 1660 Ti.
3. Download GV, Vistaar Kathbath (extract Hindi, delete zips), Lahaja. Inspect Metadata fields, Kathbath layout and split, GV tag conventions.
4. `prepare.py` (ffprobe rates, ffmpeg FLAC, filters, held-out slice, test manifests) and the normalizer with unit tests. Freeze this plan and push tag `asr-hi-plan`.
5. `sieve.py`: score RAW and test sets, apply the selection rule, retention report by reason and channel, build arm manifests for A, B, C (5 draws), W.
6. Manual audit.
7. `train.py`, `evaluate.py`, smoke test, local pilot of A on the held-out slice to see the curve shape.
8. Kaggle runner, dataset upload, throughput test with both GPUs busy, fix N from the plateau and time cap. Push tag `asr-hi-arms` with N and the SHA-256 of every arm manifest.
9. Kaggle grid, 2–3 sessions.
10. `report.py`, bootstrap, stratified tables, write-up following the outcome rules.
