import json
import os
import subprocess
import sys
import time
from pathlib import Path

import soundfile
import torch

RATE = 16_000
OUT = Path("/kaggle/working")
MODELS = {
    "whisper": {"name": "openai/whisper-large-v3", "batch": 16, "gpu": "0"},
    "mms": {"name": "facebook/mms-1b-all", "batch": 8, "gpu": "1"},
}


def find_manifest() -> Path:
    return next(Path("/kaggle/input").rglob("manifest.jsonl"))


def load_rows() -> tuple[Path, list[dict]]:
    manifest = find_manifest()
    rows = [json.loads(line) for line in manifest.read_text(encoding="utf-8").splitlines()]
    return manifest.parent, sorted(rows, key=lambda row: row["duration_s"])


def audio_path(root: Path, relative: str) -> Path:
    direct = root / relative
    if direct.exists():
        return direct
    return next(root.rglob(Path(relative).name))


def batches(rows: list[dict], size: int):
    for start in range(0, len(rows), size):
        yield rows[start:start + size]


def whisper_transcriber(name: str):
    from transformers import WhisperForConditionalGeneration, WhisperProcessor

    processor = WhisperProcessor.from_pretrained(name)
    model = WhisperForConditionalGeneration.from_pretrained(name, dtype=torch.float16, attn_implementation="sdpa").cuda().eval()

    def transcribe(audios: list) -> list[str]:
        features = processor.feature_extractor(audios, sampling_rate=RATE, return_tensors="pt").input_features
        tokens = model.generate(features.half().cuda(), language="hi", task="transcribe", max_new_tokens=440, num_beams=1)
        return processor.batch_decode(tokens, skip_special_tokens=True)

    return transcribe


def mms_transcriber(name: str):
    from transformers import AutoProcessor, Wav2Vec2ForCTC

    processor = AutoProcessor.from_pretrained(name, target_lang="hin")
    model = Wav2Vec2ForCTC.from_pretrained(name, target_lang="hin", ignore_mismatched_sizes=True, dtype=torch.float16).cuda().eval()

    def transcribe(audios: list) -> list[str]:
        inputs = processor(audios, sampling_rate=RATE, return_tensors="pt", padding=True)
        mask = inputs.get("attention_mask")
        logits = model(inputs.input_values.half().cuda(), attention_mask=None if mask is None else mask.cuda()).logits
        return processor.batch_decode(logits.argmax(-1))

    return transcribe


def work(key: str) -> None:
    spec = MODELS[key]
    root, rows = load_rows()
    transcribe = {"whisper": whisper_transcriber, "mms": mms_transcriber}[key](spec["name"])
    started = time.time()
    with (OUT / f"hyps_{key}.jsonl").open("w", encoding="utf-8") as handle, torch.inference_mode():
        for done, batch in enumerate(batches(rows, spec["batch"])):
            audios = [soundfile.read(audio_path(root, row["audio"]), dtype="float32")[0] for row in batch]
            for row, text in zip(batch, transcribe(audios)):
                handle.write(json.dumps({"id": row["id"], "model": spec["name"], "hyp": text.strip()}, ensure_ascii=False) + "\n")
            handle.flush()
            if done % 20 == 0:
                print(f"{key}: {done * spec['batch']}/{len(rows)} in {time.time() - started:.0f}s", flush=True)
    print(f"{key}: finished {len(rows)} in {time.time() - started:.0f}s", flush=True)


def main() -> None:
    if len(sys.argv) > 1:
        work(sys.argv[1])
        return
    workers = [
        subprocess.Popen([sys.executable, __file__, key], env={**os.environ, "CUDA_VISIBLE_DEVICES": spec["gpu"]})
        for key, spec in MODELS.items()
    ]
    codes = [worker.wait() for worker in workers]
    if any(codes):
        raise SystemExit(f"worker exit codes {codes}")


if __name__ == "__main__":
    main()
