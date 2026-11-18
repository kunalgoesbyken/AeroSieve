import argparse
import json
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import aerosieve


def score(root: Path, row: dict) -> dict:
    inspection = aerosieve.inspect(root / row["audio"], whole=True)
    clip = inspection["clips"][0]
    return {
        "id": row["id"],
        "verdict": clip["verdict"],
        "wada_snr_db": clip["wada_snr_db"],
        "rms_dbfs": clip["rms_dbfs"],
        "clip_ratio": clip["clip_ratio"],
        "spectral_flatness": clip["spectral_flatness"],
        "applied_gain_db": inspection["applied_gain_db"],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Score every manifest utterance as one clip with AeroSieve.")
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()

    rows = [json.loads(line) for line in (args.data / "manifest.jsonl").read_text(encoding="utf-8").splitlines()]
    with ThreadPoolExecutor(args.workers) as pool:
        scores = list(pool.map(lambda row: score(args.data, row), rows))

    with (args.data / "scores.jsonl").open("w", encoding="utf-8") as handle:
        for entry in scores:
            handle.write(json.dumps(entry) + "\n")

    verdicts = Counter(entry["verdict"] for entry in scores)
    hours = {row["id"]: row["duration_s"] / 3600 for row in rows}
    kept = sum(hours[entry["id"]] for entry in scores if entry["verdict"] == "accept")
    print(dict(verdicts.most_common()))
    print(f"kept {kept:.2f} of {sum(hours.values()):.2f} h ({kept / sum(hours.values()):.1%})")


if __name__ == "__main__":
    main()
