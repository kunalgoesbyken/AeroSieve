//! Python bindings: `aerosieve.inspect` and `aerosieve.curate`.

use std::path::{Path, PathBuf};

use aerosieve::decode::decode_file;
use aerosieve::{
    DecodeConfig, DecodedAudio, FileOutcome, FilterPolicy, Inspector, Pipeline, PipelineConfig,
    PipelineError, PipelineStats, QualityMetrics, SegmentConfig,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pythonize::{depythonize, pythonize};
use serde::Serialize;

const ACCEPTED: &str = "accept";

#[derive(Serialize)]
struct Clip {
    start_s: f32,
    duration_s: f32,
    #[serde(flatten)]
    metrics: QualityMetrics,
    verdict: &'static str,
}

/// Peak amplitude per bucket of `step_s` seconds, for drawing a waveform.
#[derive(Serialize)]
struct Envelope {
    step_s: f32,
    peaks: Vec<f32>,
}

impl Envelope {
    fn of(audio: &DecodedAudio, points: usize) -> Self {
        let bucket = audio.samples.len().div_ceil(points.max(1)).max(1);
        let peaks = audio
            .samples
            .chunks(bucket)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
            })
            .collect();
        Self {
            step_s: bucket as f32 / audio.sample_rate as f32,
            peaks,
        }
    }
}

#[derive(Serialize)]
struct Inspection {
    path: PathBuf,
    duration_s: f32,
    source_sample_rate: u32,
    applied_gain_db: f32,
    clips: Vec<Clip>,
    envelope: Option<Envelope>,
}

impl From<FileOutcome> for Inspection {
    fn from(outcome: FileOutcome) -> Self {
        let clips = outcome
            .segments
            .iter()
            .map(|clip| Clip {
                start_s: clip.segment.start_sample as f32 / outcome.sample_rate as f32,
                duration_s: clip.segment.duration_s(outcome.sample_rate),
                metrics: clip.metrics,
                verdict: clip.rejected.map_or(ACCEPTED, |reason| reason.as_str()),
            })
            .collect();
        Self {
            path: outcome.path,
            duration_s: outcome.duration_s,
            source_sample_rate: outcome.source_sample_rate,
            applied_gain_db: outcome.applied_gain_db,
            clips,
            envelope: None,
        }
    }
}

fn runtime_error(error: PipelineError) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

/// Read filter thresholds from a dict; missing keys keep their defaults.
fn policy_from(overrides: Option<&Bound<'_, PyAny>>) -> PyResult<FilterPolicy> {
    let Some(overrides) = overrides else {
        return Ok(FilterPolicy::default());
    };
    depythonize(overrides).map_err(|error| PyValueError::new_err(error.to_string()))
}

fn inspect_file(
    path: &Path,
    policy: FilterPolicy,
    max_duration_s: Option<f32>,
    envelope_points: Option<usize>,
) -> Result<Inspection, PipelineError> {
    let decode = DecodeConfig {
        max_duration_s,
        ..DecodeConfig::default()
    };
    let audio = decode_file(path, &decode)?;
    let mut inspector = Inspector::new(decode, SegmentConfig::default(), policy)?;

    let mut inspection = Inspection::from(inspector.inspect_audio(path, &audio)?);
    inspection.envelope = envelope_points.map(|points| Envelope::of(&audio, points));
    Ok(inspection)
}

fn curate_files(
    files: &[(PathBuf, Option<String>)],
    config: PipelineConfig,
) -> Result<PipelineStats, PipelineError> {
    let mut pipeline = Pipeline::new(config)?;
    for (path, transcript) in files {
        // A file that fails to decode is counted in `files_failed`.
        let _ = pipeline.process_file(path, transcript.as_deref());
    }
    pipeline.finish()
}

/// Segment and score one recording without writing anything.
///
/// Returns a dict with the file's duration and a `clips` list; each clip has
/// `start_s`, `duration_s`, the four quality metrics, and a `verdict` that is
/// either "accept" or the rejection reason.
///
/// `max_duration_s` refuses longer recordings while decoding, which matters
/// when the file comes from an untrusted upload. `envelope_points` adds an
/// `envelope` of about that many peak values for drawing a waveform.
#[pyfunction]
#[pyo3(signature = (path, policy=None, *, max_duration_s=None, envelope_points=None))]
fn inspect<'py>(
    py: Python<'py>,
    path: PathBuf,
    policy: Option<&Bound<'py, PyAny>>,
    max_duration_s: Option<f32>,
    envelope_points: Option<usize>,
) -> PyResult<Bound<'py, PyAny>> {
    let policy = policy_from(policy)?;
    let inspection = py
        .detach(|| inspect_file(&path, policy, max_duration_s, envelope_points))
        .map_err(runtime_error)?;
    Ok(pythonize(py, &inspection)?)
}

/// Curate recordings into WebDataset shards under `out_dir`.
///
/// `transcripts`, if given, must hold one entry (a string or None) per file.
/// Returns the run's totals as a dict.
#[pyfunction]
#[pyo3(signature = (files, out_dir, *, transcripts=None, rules=None, policy=None, staging_dir=None))]
fn curate<'py>(
    py: Python<'py>,
    files: Vec<PathBuf>,
    out_dir: PathBuf,
    transcripts: Option<Vec<Option<String>>>,
    rules: Option<PathBuf>,
    policy: Option<&Bound<'py, PyAny>>,
    staging_dir: Option<PathBuf>,
) -> PyResult<Bound<'py, PyAny>> {
    let transcripts = transcripts.unwrap_or_else(|| vec![None; files.len()]);
    if transcripts.len() != files.len() {
        return Err(PyValueError::new_err(
            "transcripts must have one entry per file",
        ));
    }
    let staging_dir = staging_dir.unwrap_or_else(|| out_dir.join(".staging"));
    let mut config = PipelineConfig::new(staging_dir, out_dir).with_policy(policy_from(policy)?);
    if let Some(rules) = rules {
        config = config.with_rules(rules);
    }
    let files: Vec<_> = files.into_iter().zip(transcripts).collect();

    let stats = py
        .detach(|| curate_files(&files, config))
        .map_err(runtime_error)?;
    Ok(pythonize(py, &stats)?)
}

#[pymodule(name = "aerosieve")]
fn python_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(inspect, module)?)?;
    module.add_function(wrap_pyfunction!(curate, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))
}
