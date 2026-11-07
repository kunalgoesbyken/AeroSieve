#!/usr/bin/env sh
# Reproduce every SieveBench report on the site: downloads public speech
# corpora (CC BY 4.0), extracts them, and writes site/data/sievebench-*.json.
# Usage: crates/aerosieve-sievebench/scripts/run_all.sh [work-dir]
set -eu

root="$(cd "$(dirname "$0")/../../.." && pwd)"
work="${1:-$root/target/sievebench}"
python="${PYTHON:-python3}"
mkdir -p "$work"

cargo build --release -p aerosieve-sievebench --manifest-path "$root/Cargo.toml"
bench="$root/target/release/aerosieve-sievebench"

fetch() {
  [ -f "$work/$1.parquet" ] || curl -fsSL -A aerosieve-sievebench -o "$work/$1.parquet" "$2"
  [ -d "$work/$1" ] || "$python" "$root/crates/aerosieve-sievebench/scripts/extract_parquet.py" "$work/$1.parquet" "$work/$1"
}

run() {
  "$bench" "$work/$1" --name "$2" --source "$3" --out "$root/site/data/sievebench-$1.json"
}

fetch en "https://huggingface.co/datasets/hf-internal-testing/librispeech_asr_dummy/resolve/refs%2Fconvert%2Fparquet/clean/validation/0000.parquet"
run en "English" "LibriSpeech dev-clean sample"

for pair in hi:Hindi ta:Tamil bn:Bengali te:Telugu mr:Marathi; do
  code="${pair%%:*}"
  fetch "$code" "https://huggingface.co/api/datasets/google/fleurs/parquet/${code}_in/validation/0.parquet"
  run "$code" "${pair#*:}" "FLEURS dev split"
done
