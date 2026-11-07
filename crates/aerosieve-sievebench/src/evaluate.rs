use std::io::Cursor;
use std::path::Path;

use aerosieve::decode::decode_reader;
use aerosieve::{
    DecodeConfig, FilterPolicy, Inspector, PipelineError, RejectReason, SegmentConfig,
};
use rayon::prelude::*;

use crate::condition::{Condition, Expect};
use crate::corpus::{SAMPLE_RATE, Utterance};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Kept,
    Rejected(RejectReason),
    /// The segmenter found no speech long enough to make a clip.
    NoClip,
}

#[derive(Debug, Clone, Copy)]
pub struct Observation {
    pub condition: usize,
    pub expect: Expect,
    pub verdict: Verdict,
    pub wada_snr_db: Option<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum EvaluateError {
    #[error("pipeline: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("decode: {0}")]
    Decode(#[from] aerosieve::decode::DecodeError),
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
}

/// Damage every utterance under every condition and record what the
/// pipeline does with each resulting clip.
pub fn run(
    corpus: &[Utterance],
    conditions: &[Condition],
    policy: &FilterPolicy,
) -> Result<Vec<Observation>, EvaluateError> {
    let inspector = || {
        Inspector::new(
            DecodeConfig::default(),
            SegmentConfig::default(),
            policy.clone(),
        )
    };
    inspector()?;

    let jobs: Vec<(usize, usize)> = (0..conditions.len())
        .flat_map(|condition| (0..corpus.len()).map(move |utterance| (condition, utterance)))
        .collect();

    jobs.par_iter()
        .map_init(
            || inspector().expect("configuration validated above"),
            |inspector, &(condition, utterance)| {
                observe(
                    inspector,
                    &conditions[condition],
                    condition,
                    corpus,
                    utterance,
                )
            },
        )
        .collect::<Result<Vec<_>, _>>()
        .map(|batches| batches.into_iter().flatten().collect())
}

fn observe(
    inspector: &mut Inspector,
    condition: &Condition,
    index: usize,
    corpus: &[Utterance],
    utterance: usize,
) -> Result<Vec<Observation>, EvaluateError> {
    let degraded = condition.apply(corpus, utterance)?;
    let audio = decode_reader(
        Cursor::new(wav_bytes(&degraded.samples)?),
        Some("wav"),
        &DecodeConfig::default(),
    )?;
    let outcome = inspector.inspect_audio(Path::new(&corpus[utterance].name), &audio)?;

    let observe = |verdict, wada_snr_db| Observation {
        condition: index,
        expect: degraded.expect,
        verdict,
        wada_snr_db,
    };
    if outcome.segments.is_empty() {
        return Ok(vec![observe(Verdict::NoClip, None)]);
    }
    Ok(outcome
        .segments
        .iter()
        .map(|clip| {
            let verdict = clip.rejected.map_or(Verdict::Kept, Verdict::Rejected);
            observe(verdict, Some(clip.metrics.wada_snr_db))
        })
        .collect())
}

fn wav_bytes(samples: &[f32]) -> Result<Vec<u8>, hound::Error> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut buffer, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(buffer.into_inner())
}
