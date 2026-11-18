import argparse
import json
import re
import unicodedata
from collections import defaultdict
from pathlib import Path

import jiwer
import numpy as np
from indicnlp.normalize.indic_normalize import IndicNormalizerFactory

TAG = re.compile(r"<[^>]*>")
MAX_WHISPER_S = 30.0
RESAMPLES = 10_000


def normalized_char(char: str) -> str:
    if unicodedata.category(char).startswith("P"):
        return " "
    digit = unicodedata.decimal(char, None)
    return char if digit is None else str(digit)


def normalize(text: str, script) -> str:
    text = script.normalize(unicodedata.normalize("NFC", TAG.sub(" ", text)))
    return " ".join("".join(map(normalized_char, text)).lower().split())


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def errors(reference: str, hypothesis: str) -> tuple[int, int, int, int]:
    words = jiwer.process_words(reference, hypothesis or "<empty>")
    chars = jiwer.process_characters(reference, hypothesis or " ")
    word_errors = words.substitutions + words.deletions + words.insertions
    char_errors = chars.substitutions + chars.deletions + chars.insertions
    return word_errors, len(reference.split()), char_errors, len(reference)


def rate(rows: list[dict], kind: str) -> float:
    return sum(row[f"{kind}_err"] for row in rows) / max(1, sum(row[f"{kind}_ref"] for row in rows))


def bootstrap_gap(worse: list[dict], better: list[dict], rng: np.random.Generator) -> tuple[float, float, float]:
    def arrays(rows):
        return np.array([row["word_err"] for row in rows]), np.array([row["word_ref"] for row in rows])

    (worse_err, worse_ref), (better_err, better_ref) = arrays(worse), arrays(better)
    worse_pick = rng.integers(0, len(worse), (RESAMPLES, len(worse)))
    better_pick = rng.integers(0, len(better), (RESAMPLES, len(better)))
    gaps = worse_err[worse_pick].sum(1) / worse_ref[worse_pick].sum(1) - better_err[better_pick].sum(1) / better_ref[better_pick].sum(1)
    point = worse_err.sum() / worse_ref.sum() - better_err.sum() / better_ref.sum()
    low, high = np.percentile(gaps, [2.5, 97.5])
    return float(point), float(low), float(high)


def ranks(values: list[float]) -> np.ndarray:
    return np.argsort(np.argsort(np.array(values))).astype(float)


def spearman(x: list[float], y: list[float]) -> float:
    return float(np.corrcoef(ranks(x), ranks(y))[0, 1])


def channel(row: dict) -> str:
    return "narrowband" if row["source_rate"] <= 8_000 else "wideband"


def human_flag(row: dict) -> str:
    flags = set(row["other"]) | {tag.lower() for tag in row["background"]}
    return ",".join(sorted(flags)) if flags else "none"


def table(header: list[str], body: list[list]) -> str:
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in body]
    return "\n".join(lines)


def pct(value: float) -> str:
    return f"{100 * value:.1f}%"


def group_rows(rows: list[dict], key) -> dict[str, list[dict]]:
    groups = defaultdict(list)
    for row in rows:
        groups[key(row)].append(row)
    return dict(sorted(groups.items()))


def model_section(name: str, rows: list[dict], verdict_keys: dict[str, str], rng: np.random.Generator) -> tuple[str, dict]:
    out = [f"## {name}", "", f"{len(rows)} utterances, overall WER {pct(rate(rows, 'word'))}, CER {pct(rate(rows, 'char'))}.", ""]
    summary = {"utterances": len(rows), "wer": rate(rows, "word"), "cer": rate(rows, "char")}

    for label, key in verdict_keys.items():
        accepted = [row for row in rows if row[key] == "accept"]
        rejected = [row for row in rows if row[key] != "accept"]
        gap = bootstrap_gap(rejected, accepted, rng)
        out += [f"### Accepted vs rejected ({label})", ""]
        body = [["accepted", len(accepted), f"{sum(r['duration_s'] for r in accepted) / 3600:.2f}", pct(rate(accepted, "word")), pct(rate(accepted, "char"))]]
        for reason, members in group_rows(rejected, lambda row: row[key]).items():
            body.append([f"rejected: {reason}", len(members), f"{sum(r['duration_s'] for r in members) / 3600:.2f}", pct(rate(members, "word")), pct(rate(members, "char"))])
        body.append(["rejected: all", len(rejected), f"{sum(r['duration_s'] for r in rejected) / 3600:.2f}", pct(rate(rejected, "word")), pct(rate(rejected, "char"))])
        out += [table(["group", "utterances", "hours", "WER", "CER"], body), ""]
        out += [f"Rejected minus accepted WER: {100 * gap[0]:+.1f} points (95% bootstrap CI {100 * gap[1]:+.1f} to {100 * gap[2]:+.1f}).", ""]
        summary[f"gap_{label}"] = gap

        within = []
        for stratum, members in group_rows(rows, channel).items():
            kept = [row for row in members if row[key] == "accept"]
            dropped = [row for row in members if row[key] != "accept"]
            if kept and dropped:
                stratum_gap = bootstrap_gap(dropped, kept, rng)
                within.append([stratum, len(kept), len(dropped), pct(rate(kept, "word")), pct(rate(dropped, "word")), f"{100 * stratum_gap[0]:+.1f} ({100 * stratum_gap[1]:+.1f} to {100 * stratum_gap[2]:+.1f})"])
        out += ["Within each channel:", "", table(["channel", "accepted", "rejected", "accepted WER", "rejected WER", "gap, 95% CI"], within), ""]

    quintiles = np.quantile([row["wada_snr_db"] for row in rows], [0.2, 0.4, 0.6, 0.8])
    bins = group_rows(rows, lambda row: int(np.searchsorted(quintiles, row["wada_snr_db"])))
    body = [[f"Q{index + 1}", f"{min(r['wada_snr_db'] for r in members):.1f} to {max(r['wada_snr_db'] for r in members):.1f}", len(members), pct(rate(members, "word"))] for index, members in bins.items()]
    rho = spearman([row["wada_snr_db"] for row in rows], [row["word_err"] / max(1, row["word_ref"]) for row in rows])
    out += ["### WER by WADA-SNR quintile", "", table(["quintile", "WADA-SNR dB", "utterances", "WER"], body), "", f"Spearman correlation of WADA-SNR with per-utterance WER: {rho:.3f}.", ""]
    summary["spearman_wada_wer"] = rho

    body = [[flag, len(members), pct(rate(members, "word")), pct(sum(row["verdict"] != "accept" for row in members) / len(members))] for flag, members in group_rows(rows, human_flag).items() if len(members) >= 15]
    out += ["### Human recording-problem labels", "", table(["human label", "utterances", "WER", "AeroSieve rejects"], body), ""]
    return "\n".join(out), summary


def main() -> None:
    parser = argparse.ArgumentParser(description="Relate AeroSieve verdicts to ASR errors on Gram Vaani.")
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--language", required=True, help="ISO 639-1 code understood by indic-nlp, e.g. hi, ta, bn")
    parser.add_argument("--hyps", type=Path, nargs="+", required=True)
    parser.add_argument("--baseline-scores", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    manifest = {row["id"]: row for row in read_jsonl(args.data / "manifest.jsonl")}
    scores = {row["id"]: row for row in read_jsonl(args.data / "scores.jsonl")}
    baseline = {row["id"]: row["verdict"] for row in read_jsonl(args.baseline_scores)} if args.baseline_scores else {}
    verdict_keys = {"current": "verdict", **({"v0.4.0": "baseline_verdict"} if baseline else {})}

    script = IndicNormalizerFactory().get_normalizer(args.language)
    rng = np.random.default_rng(0)
    sections, summaries = [], {}
    for hyps_path in args.hyps:
        hyps = read_jsonl(hyps_path)
        model = hyps[0]["model"]
        rows = []
        for hyp in hyps:
            row = {**manifest[hyp["id"]], **scores[hyp["id"]], "baseline_verdict": baseline.get(hyp["id"])}
            reference = normalize(row["text"], script)
            if not reference or row["duration_s"] > MAX_WHISPER_S:
                continue
            row["word_err"], row["word_ref"], row["char_err"], row["char_ref"] = errors(reference, normalize(hyp["hyp"], script))
            rows.append(row)
        section, summaries[model] = model_section(model, rows, verdict_keys, rng)
        sections.append(section)

    intro = [
        "# AeroSieve verdicts vs ASR errors on Gram Vaani Hindi",
        "",
        "Gram Vaani Dev + Eval (OpenSLR 118), transcripts verified twice by humans. Each utterance is scored whole by AeroSieve (`inspect(..., whole=True)`, default policy) and transcribed by pretrained models with no fine-tuning. Utterances over 30 s or with an empty normalized reference are excluded. WER is corpus-level (total errors over total reference words); CIs come from 10,000 bootstrap resamples of utterances.",
        "",
    ]
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "report.md").write_text("\n".join(intro + sections), encoding="utf-8")
    (args.out / "summary.json").write_text(json.dumps(summaries, indent=2), encoding="utf-8")
    print(json.dumps(summaries, indent=2))


if __name__ == "__main__":
    main()
