//! Export a run's manifest in the layout NVIDIA NeMo uses for tarred audio
//! datasets: one JSON object per line with `audio_filepath`, `duration`,
//! `text`, and `shard_id`.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use aerosieve::shard::SampleRecord;
use serde::Serialize;

use crate::CliResult;

const RUN_MANIFEST: &str = "manifest.jsonl";
const NEMO_MANIFEST: &str = "nemo_manifest.jsonl";

#[derive(Serialize)]
struct NemoEntry<'a> {
    audio_filepath: String,
    duration: f32,
    text: &'a str,
    shard_id: usize,
}

pub fn write_manifest(run_dir: &Path) -> CliResult<()> {
    let source = run_dir.join(RUN_MANIFEST);
    let reader = BufReader::new(
        File::open(&source).map_err(|error| format!("{}: {error}", source.display()))?,
    );
    let destination = run_dir.join(NEMO_MANIFEST);
    let mut writer = BufWriter::new(File::create(&destination)?);

    let mut shards: Vec<String> = Vec::new();
    let mut entries = 0usize;
    for line in reader.lines() {
        let record: SampleRecord = serde_json::from_str(&line?)?;
        let shard_id = match shards.iter().position(|shard| *shard == record.shard) {
            Some(shard_id) => shard_id,
            None => {
                shards.push(record.shard.clone());
                shards.len() - 1
            }
        };
        let entry = NemoEntry {
            audio_filepath: format!("{}.wav", record.key),
            duration: record.meta.duration_s,
            text: &record.meta.transcript,
            shard_id,
        };
        serde_json::to_writer(&mut writer, &entry)?;
        writer.write_all(b"\n")?;
        entries += 1;
    }
    writer.flush()?;

    println!("{entries} entries written to {}", destination.display());
    Ok(())
}
