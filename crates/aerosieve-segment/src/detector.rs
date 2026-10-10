use aerosieve_acoustic::rms_dbfs;

use crate::error::SegmentError;

/// Duration of one detector window. Silero VAD requires exactly this much
/// audio per call (512 samples at 16 kHz, 256 at 8 kHz), and
/// [`EnergyDetector`] uses the same so both produce comparable boundaries.
pub const WINDOW_MS: u32 = 32;

/// Width, in dB, of the logistic transition from "silence" to "speech".
const SOFTNESS_DB: f32 = 4.0;

/// RMS level at which [`EnergyDetector::new`] reports a probability of 0.5.
pub const DEFAULT_ENERGY_THRESHOLD_DBFS: f32 = -45.0;

pub(crate) fn window_samples(sample_rate: u32) -> usize {
    (u64::from(sample_rate) * u64::from(WINDOW_MS) / 1000) as usize
}

/// Scores consecutive windows of audio with a speech probability.
///
/// Windows are fed in order, each exactly
/// [`window_size`](SpeechDetector::window_size) samples long. Detectors may
/// carry state between calls; [`reset`](SpeechDetector::reset) clears it.
pub trait SpeechDetector {
    /// Samples per window. Constant for the lifetime of the detector.
    fn window_size(&self) -> usize;

    /// Speech probability for `window`, in `0.0..=1.0`.
    fn probability(&mut self, window: &[f32]) -> Result<f32, SegmentError>;

    /// Return to the state before any audio was seen.
    fn reset(&mut self);
}

/// Level-based detector with no model and no dependencies.
///
/// The window's RMS level is mapped through a logistic curve centred on the
/// threshold: 12 dB above it reads 0.95, 12 dB below reads 0.05, and digital
/// silence reads exactly 0.
///
/// This measures energy, not speech: loud music or machinery scores as
/// speech. Use the `silero` feature when that distinction matters.
#[derive(Debug, Clone)]
pub struct EnergyDetector {
    window: usize,
    threshold_dbfs: f32,
}

impl EnergyDetector {
    /// Detector centred on [`DEFAULT_ENERGY_THRESHOLD_DBFS`].
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self::with_threshold_dbfs(sample_rate, DEFAULT_ENERGY_THRESHOLD_DBFS)
    }

    /// Detector whose probability crosses 0.5 at `threshold_dbfs`.
    #[must_use]
    pub fn with_threshold_dbfs(sample_rate: u32, threshold_dbfs: f32) -> Self {
        Self {
            window: window_samples(sample_rate),
            threshold_dbfs,
        }
    }
}

impl SpeechDetector for EnergyDetector {
    fn window_size(&self) -> usize {
        self.window
    }

    fn probability(&mut self, window: &[f32]) -> Result<f32, SegmentError> {
        let level_dbfs = rms_dbfs(window);
        if level_dbfs.is_nan() {
            return Ok(0.0);
        }
        let distance = (level_dbfs - self.threshold_dbfs) / SOFTNESS_DB;
        Ok(1.0 / (1.0 + (-distance).exp()))
    }

    fn reset(&mut self) {}
}
