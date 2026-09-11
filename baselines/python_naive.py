"""
Naive Python baseline for the "David vs. Goliath" benchmark.

This script implements the same ingestion task as AeroSieve using a
straightforward Python stack: list comprehensions for audio filtering and the
standard `re` module for text normalization.  It is intentionally *not*
optimized — the goal is to show what an engineer reaches for by default and
how much headroom a dedicated Rust engine leaves on the table.

Run with:
    python baselines/python_naive.py
"""

import math
import re
import sys
import time

# Windows consoles default to cp1252, which cannot encode the box-drawing
# characters used in the output tables. Force UTF-8 so `python
# baselines/python_naive.py` works as documented on every platform.
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except (AttributeError, OSError):  # pragma: no cover - exotic stdout
        pass


# ─────────────────────────────────────────────────────────────────────────────
# Audio "sieve": RMS, silence gate, and clip gate
# ─────────────────────────────────────────────────────────────────────────────
def rms_db(samples: list[float]) -> float:
    if not samples:
        return float("-inf")
    squared = sum(x * x for x in samples) / len(samples)
    return 20.0 * math.log10(math.sqrt(squared))


def analyze_audio(samples: list[float]) -> dict:
    db = rms_db(samples)
    silence_threshold = -50.0
    clip_count = sum(1 for x in samples if abs(x) >= 0.999)
    clip_ratio = clip_count / len(samples) if samples else 0.0
    clip_threshold = 0.001

    reject = db < silence_threshold or clip_ratio > clip_threshold
    return {
        "pass": not reject,
        "rms_db": db,
        "clip_ratio": clip_ratio,
    }


# ─────────────────────────────────────────────────────────────────────────────
# Text normalization: a small Hinglish rule set using Python regex
# ─────────────────────────────────────────────────────────────────────────────
RULES = [
    (re.compile(r"\u20B9\s*(\d+)"), r"\1 rupaye"),
    (re.compile(r"\b(\d+)\s*(?:lakh|laakh)\b"), r"\1 lakh"),
    (re.compile(r"\b(\d+)\s*(?:crore|karod)\b"), r"\1 crore"),
    (re.compile(r"\bdr\."), "doctor"),
    (re.compile(r"\bprof\."), "professor"),
    (re.compile(r"\s+"), " "),  # collapse whitespace
]


def normalize_text(text: str) -> str:
    for pattern, repl in RULES:
        text = pattern.sub(repl, text)
    return text.strip()


# ─────────────────────────────────────────────────────────────────────────────
# Workload generation
# ─────────────────────────────────────────────────────────────────────────────
def make_speech(frame_samples: int = 320) -> list[float]:
    return [math.sin(i * 0.1) * 0.5 for i in range(frame_samples)]


def make_silence(frame_samples: int = 320) -> list[float]:
    return [0.0] * frame_samples


# ─────────────────────────────────────────────────────────────────────────────
# Benchmarks
# ─────────────────────────────────────────────────────────────────────────────
def latency_distribution(frames: int = 10_000):
    speech = make_speech()
    text = "yeh \u20B9500 hai aur 5 laakh rupaye"

    latencies = []
    for _ in range(frames):
        t0 = time.perf_counter()
        result = analyze_audio(speech)
        if result["pass"]:
            normalize_text(text)
        latencies.append((time.perf_counter() - t0) * 1e9)

    latencies.sort()
    def pct(p: float) -> float:
        idx = int(len(latencies) * p / 100.0)
        return latencies[min(idx, len(latencies) - 1)]

    print("\n  Python Naive Baseline — Tail Latency")
    print("  ──────────────────────────────────────────────────────────────")
    print(f"  P50: {pct(50):>10.0f} ns")
    print(f"  P90: {pct(90):>10.0f} ns")
    print(f"  P95: {pct(95):>10.0f} ns")
    print(f"  P99: {pct(99):>10.0f} ns")
    print(f"  P99.9: {pct(99.9):>8.0f} ns")
    print("  ──────────────────────────────────────────────────────────────")


def throughput(frames: int = 100_000):
    speech = make_speech()
    text = "yeh \u20B9500 hai aur 5 laakh rupaye"

    processed = 0
    t0 = time.perf_counter()
    for _ in range(frames):
        result = analyze_audio(speech)
        if result["pass"]:
            normalize_text(text)
        processed += 1
    elapsed = time.perf_counter() - t0

    print("\n  Python Naive Baseline — Throughput")
    print("  ──────────────────────────────────────────────────────────────")
    print(f"  {processed} frames in {elapsed:.2f} s")
    print(f"  {processed / elapsed:>10.0f} fps")
    print("  ──────────────────────────────────────────────────────────────")


def main():
    latency_distribution(10_000)
    throughput(100_000)


if __name__ == "__main__":
    main()
