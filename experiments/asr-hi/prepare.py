import argparse
import csv
import json
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import soundfile

TARGET_RATE = 16_000


def read_pairs(path: Path) -> dict[str, str]:
    pairs = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split(maxsplit=1)
        if parts:
            pairs[parts[0]] = parts[1].strip() if len(parts) > 1 else ""
    return pairs


def read_labels(path: Path) -> dict[str, dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        rows = csv.DictReader(handle, delimiter="\t")
        return {row["Uttids"]: row for row in rows}


def source_rate(mp3: Path) -> int:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries",
         "stream=sample_rate", "-of", "csv=p=0", str(mp3)],
        capture_output=True, text=True, check=True,
    )
    return int(out.stdout.strip())


def to_flac(mp3: Path, flac: Path) -> None:
    if flac.exists():
        return
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-i", str(mp3), "-ac", "1",
         "-ar", str(TARGET_RATE), "-sample_fmt", "s16", str(flac)],
        check=True,
    )


def tags(value: str | None) -> list[str]:
    if not value or value == "NA":
        return []
    return [tag for tag in value.split(",") if tag and tag != "NA"]


def prepare_utterance(split_dir: Path, audio_dir: Path, labels: dict, utt: str, relative: str, text: str) -> dict:
    mp3 = (split_dir / relative).resolve()
    flac = audio_dir / f"{utt}.flac"
    rate = source_rate(mp3)
    to_flac(mp3, flac)
    label = labels.get(utt, {})
    return {
        "id": utt,
        "split": split_dir.name,
        "audio": flac.relative_to(audio_dir.parent).as_posix(),
        "text": text,
        "source_rate": rate,
        "duration_s": soundfile.info(flac).duration,
        "background": tags(label.get("Background")),
        "other": tags(label.get("Other")),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Convert Gram Vaani splits to 16 kHz FLAC with a manifest.")
    parser.add_argument("--gv-root", type=Path, required=True)
    parser.add_argument("--splits", nargs="+", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()

    audio_dir = args.out / "audio"
    audio_dir.mkdir(parents=True, exist_ok=True)
    jobs = []
    for split in args.splits:
        split_dir = args.gv_root / split
        texts = read_pairs(split_dir / "text")
        paths = read_pairs(split_dir / "mp3.scp")
        labels = read_labels(args.gv_root / "Metadata" / f"utt2labels_{split}.txt")
        missing = set(texts) ^ set(paths)
        if missing:
            raise SystemExit(f"{split}: {len(missing)} utterances lack audio or text, e.g. {sorted(missing)[:3]}")
        jobs += [(split_dir, audio_dir, labels, utt, paths[utt], texts[utt]) for utt in sorted(texts)]

    with ThreadPoolExecutor(args.workers) as pool:
        rows = list(pool.map(lambda job: prepare_utterance(*job), jobs))

    manifest = args.out / "manifest.jsonl"
    with manifest.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    hours = sum(row["duration_s"] for row in rows) / 3600
    print(f"{len(rows)} utterances, {hours:.2f} h -> {manifest}")


if __name__ == "__main__":
    main()
