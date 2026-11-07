//! SieveBench: inject known defects into clean speech and measure how well
//! each AeroSieve filter catches them.

mod condition;
mod corpus;
mod evaluate;
mod noise;
mod report;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use aerosieve::FilterPolicy;
use clap::Parser;

use crate::condition::Condition;
use crate::corpus::SAMPLE_RATE;
use crate::report::RunInfo;

/// Measure how well AeroSieve's filters catch injected defects.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Directory of clean speech recordings.
    corpus: PathBuf,
    /// Name to record in the report, such as "FLEURS Hindi dev".
    #[arg(long)]
    name: String,
    /// Where to write the JSON report.
    #[arg(long)]
    out: PathBuf,
    /// Use only the first N recordings.
    #[arg(long)]
    limit: Option<usize>,
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus::load(&args.corpus, args.limit)?;
    if corpus.len() < 2 {
        return Err(format!(
            "{} holds fewer than two usable recordings",
            args.corpus.display()
        )
        .into());
    }
    let samples: usize = corpus.iter().map(|u| u.samples.len()).sum();
    let conditions = Condition::all();
    let policy = FilterPolicy::default();

    let started = Instant::now();
    let observations = evaluate::run(&corpus, &conditions, &policy)?;
    let info = RunInfo {
        corpus: args.name,
        utterances: corpus.len(),
        hours: samples as f64 / f64::from(SAMPLE_RATE) / 3600.0,
        elapsed_s: started.elapsed().as_secs_f64(),
        policy,
    };
    let report = report::build(info, &conditions, &observations);

    std::fs::write(&args.out, serde_json::to_string_pretty(&report)?)?;
    let overall = &report.overall;
    println!(
        "{} recordings x {} conditions in {:.1}s: recall {:.1}%, precision {:.1}%, false rejects {:.1}%",
        report.utterances,
        conditions.len(),
        report.elapsed_s,
        overall.recall * 100.0,
        overall.precision * 100.0,
        overall.false_reject_rate * 100.0,
    );
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
