use std::path::PathBuf;

/// Errors produced by this crate.
///
/// [`EnergyDetector`](crate::EnergyDetector) never fails; every variant
/// concerns the optional Silero detector.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SegmentError {
    /// The Silero model file does not exist.
    #[error("silero model not found at {}", .0.display())]
    ModelNotFound(PathBuf),

    /// ONNX Runtime failed to load the model or run inference.
    #[error("onnx runtime error: {0}")]
    Onnx(String),

    /// The Silero model accepts only 8 kHz and 16 kHz audio.
    #[error("unsupported sample rate {0} Hz for silero (expected 8000 or 16000)")]
    UnsupportedSampleRate(u32),
}
