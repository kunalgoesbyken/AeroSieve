//! Error type for segmentation.

/// Errors produced by this crate.
///
/// The default (pure-DSP) build cannot fail at runtime — segmentation of an
/// in-memory buffer is infallible — so every variant here concerns loading or
/// running the optional ONNX Silero detector.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SegmentError {
    /// The Silero model file could not be found or read.
    #[error("silero model not found at {path}")]
    ModelNotFound {
        /// Path that was attempted.
        path: String,
    },

    /// The ONNX Runtime session failed to build or execute.
    #[error("onnx runtime error: {0}")]
    Onnx(String),

    /// A sample rate other than 8 kHz or 16 kHz was requested for Silero.
    ///
    /// Silero VAD accepts exactly 512 samples @ 16 kHz or 256 samples @ 8 kHz;
    /// no other rate is supported by the shipped model.
    #[error("unsupported sample rate {0} Hz for silero (expected 8000 or 16000)")]
    UnsupportedSampleRate(u32),
}
