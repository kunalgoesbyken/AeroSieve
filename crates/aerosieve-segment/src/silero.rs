use std::path::Path;

use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::{Tensor, TensorRef};

use crate::detector::{SpeechDetector, window_samples};
use crate::error::SegmentError;

const SUPPORTED_SAMPLE_RATES: [u32; 2] = [8_000, 16_000];
const STATE_SHAPE: [usize; 3] = [2, 1, 128];
const STATE_LEN: usize = STATE_SHAPE[0] * STATE_SHAPE[1] * STATE_SHAPE[2];

impl<R> From<ort::Error<R>> for SegmentError {
    fn from(error: ort::Error<R>) -> Self {
        Self::Onnx(error.to_string())
    }
}

/// Silero VAD v5, run through ONNX Runtime.
///
/// The model is recurrent: it takes its previous state as an input and
/// returns the next one, so windows must be fed in order.
pub struct SileroDetector {
    session: Session,
    sample_rate: i64,
    window: usize,
    state: Vec<f32>,
}

impl std::fmt::Debug for SileroDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SileroDetector")
            .field("sample_rate", &self.sample_rate)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl SileroDetector {
    /// Load `silero_vad.onnx` from `path` for audio at `sample_rate`.
    pub fn from_model_path(path: &Path, sample_rate: u32) -> Result<Self, SegmentError> {
        if !SUPPORTED_SAMPLE_RATES.contains(&sample_rate) {
            return Err(SegmentError::UnsupportedSampleRate(sample_rate));
        }
        if !path.exists() {
            return Err(SegmentError::ModelNotFound(path.to_path_buf()));
        }

        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?
            .commit_from_file(path)?;

        Ok(Self {
            session,
            sample_rate: i64::from(sample_rate),
            window: window_samples(sample_rate),
            state: vec![0.0; STATE_LEN],
        })
    }
}

impl SpeechDetector for SileroDetector {
    fn window_size(&self) -> usize {
        self.window
    }

    fn probability(&mut self, window: &[f32]) -> Result<f32, SegmentError> {
        let input = TensorRef::from_array_view(([1, window.len()], window))?;
        let sample_rate = Tensor::from_array(([1], vec![self.sample_rate]))?;
        let state = TensorRef::from_array_view((STATE_SHAPE, self.state.as_slice()))?;

        let outputs = self.session.run(ort::inputs![
            "input" => input,
            "sr" => sample_rate,
            "state" => state,
        ])?;
        let (_, probability) = outputs["output"].try_extract_tensor::<f32>()?;
        let (_, next_state) = outputs["stateN"].try_extract_tensor::<f32>()?;

        let probability = probability
            .first()
            .copied()
            .ok_or_else(|| SegmentError::Onnx("model returned no probability".into()))?;
        self.state.copy_from_slice(next_state);
        Ok(probability.clamp(0.0, 1.0))
    }

    fn reset(&mut self) {
        self.state.fill(0.0);
    }
}
