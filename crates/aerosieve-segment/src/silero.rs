//! Silero VAD detector, behind the `silero` feature.
//!
//! Not built by default. See the crate README: `ort` ships prebuilt ONNX
//! Runtime binaries only for `x86_64-pc-windows-msvc`, so this feature does not
//! build under the windows-gnu toolchain.

use std::path::Path;

use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

use crate::detector::{SILERO_WINDOW_8K, SILERO_WINDOW_16K, SpeechDetector};
use crate::error::SegmentError;

/// Shape of the Silero recurrent state tensor.
const STATE_SHAPE: [usize; 3] = [2, 1, 128];
/// Flattened length of [`STATE_SHAPE`].
const STATE_LEN: usize = STATE_SHAPE[0] * STATE_SHAPE[1] * STATE_SHAPE[2];

/// Silero VAD v5 via ONNX Runtime.
///
/// The model takes exactly 512 samples at 16 kHz (256 at 8 kHz) per call and
/// carries a recurrent state between calls, so windows must be fed in order.
/// [`reset`](SpeechDetector::reset) zeroes that state.
///
/// Graph signature:
///
/// | name    | shape        | dtype |
/// |---------|--------------|-------|
/// | `input` | `[1, 512]`   | f32   |
/// | `sr`    | `[]`         | i64   |
/// | `state` | `[2, 1, 128]`| f32   |
/// | `output`| `[1, 1]`     | f32   |
/// | `stateN`| `[2, 1, 128]`| f32   |
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
    /// Load `silero_vad.onnx` from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::ModelNotFound`] if `path` does not exist,
    /// [`SegmentError::UnsupportedSampleRate`] for any rate other than 8000 or
    /// 16000, and [`SegmentError::Onnx`] if the session cannot be built.
    pub fn from_model_path(path: &Path, sample_rate: u32) -> Result<Self, SegmentError> {
        let window = match sample_rate {
            16_000 => SILERO_WINDOW_16K,
            8_000 => SILERO_WINDOW_8K,
            other => return Err(SegmentError::UnsupportedSampleRate(other)),
        };

        if !path.exists() {
            return Err(SegmentError::ModelNotFound {
                path: path.display().to_string(),
            });
        }

        let session = Session::builder()
            .map_err(|e| SegmentError::Onnx(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| SegmentError::Onnx(e.to_string()))?
            .with_intra_threads(1)
            .map_err(|e| SegmentError::Onnx(e.to_string()))?
            .commit_from_file(path)
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;

        Ok(Self {
            session,
            sample_rate: i64::from(sample_rate),
            window,
            state: vec![0.0; STATE_LEN],
        })
    }

    /// Run one inference step, returning the probability and the new state.
    fn infer(&mut self, window: &[f32]) -> Result<f32, SegmentError> {
        let input = Tensor::from_array(([1usize, window.len()], window.to_vec()))
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;
        let sr = Tensor::from_array(([1usize], vec![self.sample_rate]))
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;
        let state = Tensor::from_array((STATE_SHAPE, self.state.clone()))
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;

        let outputs = self
            .session
            .run(ort::inputs![
                "input" => input,
                "sr" => sr,
                "state" => state,
            ])
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;

        let (_, new_state) = outputs["stateN"]
            .try_extract_tensor::<f32>()
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;
        self.state.clear();
        self.state.extend_from_slice(new_state);

        let (_, prob) = outputs["output"]
            .try_extract_tensor::<f32>()
            .map_err(|e| SegmentError::Onnx(e.to_string()))?;

        Ok(prob.first().copied().unwrap_or(0.0))
    }
}

impl SpeechDetector for SileroDetector {
    fn window_size(&self) -> usize {
        self.window
    }

    fn probability(&mut self, window: &[f32]) -> f32 {
        // The trait is infallible by design: a failed inference is treated as
        // "no speech here" rather than poisoning the whole segmentation run.
        // Load-time failures surface as `SegmentError` from `from_model_path`.
        match self.infer(window) {
            Ok(p) if p.is_finite() => p.clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    fn reset(&mut self) {
        self.state.clear();
        self.state.resize(STATE_LEN, 0.0);
    }
}
