"""Write the audio column of a Hugging Face parquet file out as files.

    python extract_parquet.py fleurs-hi-dev.parquet corpus/fleurs-hi
"""

import pathlib
import sys

import pyarrow.parquet as pq


def main(parquet: str, out_dir: str) -> None:
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    table = pq.read_table(parquet, columns=["audio"])
    for index, row in enumerate(table.column("audio").to_pylist()):
        suffix = pathlib.Path(row.get("path") or "clip.wav").suffix or ".wav"
        (out / f"{index:05d}{suffix}").write_bytes(row["bytes"])
    print(f"wrote {table.num_rows} files to {out}")


if __name__ == "__main__":
    main(*sys.argv[1:3])
