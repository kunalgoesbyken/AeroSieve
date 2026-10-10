//! `aerosieve`: curate audio into training shards from the command line.

mod inputs;
mod nemo;
mod summary;

use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aerosieve::{
    DecodeConfig, FileOutcome, FilterPolicy, Inspector, Pipeline, PipelineConfig, SegmentConfig,
};
use clap::{Args, Parser, Subcommand};

use crate::summary::Summary;

const TRANSCRIPT_EXTENSION: &str = "txt";
const DEFAULT_STAGING_SUBDIR: &str = ".staging";
const MAX_TRANSCRIPT_BYTES: u64 = 1024 * 1024;

type CliResult<T> = Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(name = "aerosieve", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Segment and filter recordings, writing accepted clips as WebDataset shards
    Curate(CurateArgs),
    /// Show how the filter judges recordings, without writing anything
    Inspect(InspectArgs),
    /// Write a NeMo-style tarred-dataset manifest for a finished run
    NemoManifest {
        /// Output directory of a finished `curate` run
        run_dir: PathBuf,
    },
}

#[derive(Args)]
struct CurateArgs {
    #[command(flatten)]
    inputs: InputArgs,
    /// Directory to write shards, manifest.jsonl, and stats.json into
    #[arg(long)]
    out: PathBuf,
    /// Scratch directory on the same volume as --out; defaults to OUT/.staging
    #[arg(long)]
    staging: Option<PathBuf>,
    /// YAML ruleset used to normalize transcripts
    #[arg(long)]
    rules: Option<PathBuf>,
}

#[derive(Args)]
struct InspectArgs {
    #[command(flatten)]
    inputs: InputArgs,
    /// Print only the totals, not one line per clip
    #[arg(long)]
    summary_only: bool,
}

#[derive(Args)]
struct InputArgs {
    /// Audio files, or directories to search recursively
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// JSON file overriding filter thresholds, e.g. {"min_snr_db": 15}
    #[arg(long)]
    policy: Option<PathBuf>,
}

impl InputArgs {
    fn policy(&self) -> CliResult<FilterPolicy> {
        let Some(path) = &self.policy else {
            return Ok(FilterPolicy::default());
        };
        let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(serde_json::from_reader(BufReader::new(file))?)
    }
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Curate(args) => curate(&args),
        Command::Inspect(args) => inspect(&args),
        Command::NemoManifest { run_dir } => nemo::write_manifest(&run_dir),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn curate(args: &CurateArgs) -> CliResult<()> {
    let files = inputs::audio_files(&args.inputs.paths)?;
    let staging = args
        .staging
        .clone()
        .unwrap_or_else(|| args.out.join(DEFAULT_STAGING_SUBDIR));

    let mut config = PipelineConfig::new(staging, &args.out).with_policy(args.inputs.policy()?);
    if let Some(rules) = &args.rules {
        config = config.with_rules(rules);
    }
    let mut pipeline = Pipeline::new(config)?;

    for file in &files {
        let curated = sidecar_transcript(file)
            .and_then(|transcript| Ok(pipeline.process_file(file, transcript.as_deref())?));
        if let Err(error) = curated {
            eprintln!("{}: skipped ({error})", file.display());
        }
    }

    let stats = pipeline.finish()?;
    println!(
        "{} files curated, {} skipped",
        stats.files_processed, stats.files_failed
    );
    println!(
        "{} of {} clips accepted ({:.1}% of detected speech)",
        stats.segments_accepted,
        stats.segments_found,
        stats.retention_rate() * 100.0
    );
    for (reason, count) in &stats.rejections {
        println!("  rejected as {reason:<10} {count}");
    }
    println!(
        "{:.1} minutes of audio in {} shard(s) at {}",
        stats.shard.total_duration_s / 60.0,
        stats.shard.shards_written,
        args.out.display()
    );
    Ok(())
}

/// The transcript stored next to `audio` as `<name>.txt`, if there is one.
///
/// Only a regular file of modest size is read. A corpus may come from an
/// untrusted source, and following a symlink here would copy whatever it
/// points at into the published manifest.
fn sidecar_transcript(audio: &Path) -> CliResult<Option<String>> {
    let path = audio.with_extension(TRANSCRIPT_EXTENSION);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Err(format!("{}: not a regular file", path.display()).into());
    }
    if metadata.len() > MAX_TRANSCRIPT_BYTES {
        return Err(format!(
            "{}: transcript is larger than {MAX_TRANSCRIPT_BYTES} bytes",
            path.display()
        )
        .into());
    }
    let transcript =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Some(transcript))
}

fn inspect(args: &InspectArgs) -> CliResult<()> {
    let files = inputs::audio_files(&args.inputs.paths)?;
    let mut inspector = Inspector::new(
        DecodeConfig::default(),
        SegmentConfig::default(),
        args.inputs.policy()?,
    )?;

    let mut summary = Summary::default();
    for file in &files {
        match inspector.inspect_file(file) {
            Ok(outcome) => {
                if !args.summary_only {
                    print_clips(&outcome);
                }
                summary.add(&outcome);
            }
            Err(error) => {
                eprintln!("{}: skipped ({error})", file.display());
                summary.add_failure();
            }
        }
    }
    print!("{summary}");
    Ok(())
}

fn print_clips(outcome: &FileOutcome) {
    println!(
        "{} ({:.1} s, {} Hz source)",
        outcome.path.display(),
        outcome.duration_s,
        outcome.source_sample_rate
    );
    println!(
        "  {:>7} {:>7} {:>8} {:>8} {:>6} {:>9}  VERDICT",
        "START", "LENGTH", "SNR dB", "dBFS", "CLIP", "FLATNESS"
    );
    for clip in &outcome.segments {
        println!(
            "  {:>6.1}s {:>6.1}s {:>8.1} {:>8.1} {:>6.3} {:>9.3}  {}",
            clip.segment.start_sample as f32 / outcome.sample_rate as f32,
            clip.segment.duration_s(outcome.sample_rate),
            clip.metrics.wada_snr_db,
            clip.metrics.rms_dbfs,
            clip.metrics.clip_ratio,
            clip.metrics.spectral_flatness,
            clip.rejected.map_or("accept", |reason| reason.as_str()),
        );
    }
    println!();
}
