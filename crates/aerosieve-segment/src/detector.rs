//! The [`SpeechDetector`] trait and the default pure-DSP [`EnergyDetector`].

use aerosieve_acoustic::rms_dbfs;

/// Window size used by Silero VAD at 16 kHz, and by [`EnergyDetector`] at every
/// rate so the two detectors produce comparable segment boundaries.
pub const SILERO_WINDOW_16K: usize = 512;

/// Window size required by Silero VAD at 8 kHz.
pub const SILERO_WINDOW_8K: usize = 256;

/// Produces a per-window speech probability.
///
/// A detector is fed contiguous, non-overlapping windows of exactly
/// [`window_size`](SpeechDetector::window_size) samples and returns a value in
/// `0.0..=1.0`. Implementations may be stateful (Silero carries an RNN state
/// between calls); [`reset`](SpeechDetector::reset) returns them to the state
/// they had before any audio was seen.
pub trait SpeechDetector {
    /// Number of samples per [`probability`](SpeechDetector::probability) call.
    ///
    /// Must be non-zero and constant for the lifetime of the detector.
    fn window_size(&self) -> usize;

    /// Speech probability for `window`, clamped to `0.0..=1.0`.
    ///
    /// `window` is guaranteed to be exactly `window_size()` samples long.
    fn probability(&mut self, window: &[f32]) -> f32;

    /// Clear any carried state.
    fn reset(&mut self);
}

/// Pure-DSP speech detector: the crate default, with no ONNX dependency.
///
/// # Mapping
///
/// The detector computes the RMS level of the window in dBFS (via
/// [`aerosieve_acoustic::rms_dbfs`]) and maps it through a logistic curve
/// centred on a configurable threshold:
///
/// ```text
/// p = 1 / (1 + exp(-(dbfs - threshold_dbfs) / SOFTNESS))
/// ```
///
/// with `SOFTNESS = 4.0` dB. That gives
///
/// | `dbfs - threshold_dbfs` | `p`   |
/// |-------------------------|-------|
/// | -12 dB                  | 0.047 |
/// |  -6 dB                  | 0.18  |
/// |   0 dB                  | 0.50  |
/// |  +6 dB                  | 0.82  |
/// | +12 dB                  | 0.95  |
///
/// so the transition through the segmenter's 0.35/0.50 hysteresis band spans
/// roughly 2.5 dB — wide enough to be robust to a single quiet window, narrow
/// enough that clearly-voiced audio saturates near 1.0 and true silence
/// collapses to ~0.0 (digital silence returns `-inf` dBFS, hence exactly 0.0).
///
/// The default threshold of `-45 dBFS` sits far below the pipeline's `-20 dBFS`
/// normalisation target, so normalised speech scores ≈1.0 while dithered or
/// noise-floor-only passages score ≈0.0.
///
/// This is an *energy* detector, not a speech/noise classifier: loud
/// non-speech (music, machinery) will read as speech. Enable the `silero`
/// feature when that distinction matters.
#[derive(Debug, Clone)]
pub struct EnergyDetector {
    window: usize,
    threshold_dbfs: f32,
}

/// Logistic width, in dB, of the energy→probability transition.
const SOFTNESS_DB: f32 = 4.0;

/// Default RMS level at which the detector returns p = 0.5.
pub const DEFAULT_ENERGY_THRESHOLD_DBFS: f32 = -45.0;

impl EnergyDetector {
    /// Detector with the default `-45 dBFS` midpoint.
    ///
    /// `sample_rate` selects the window size: 256 samples at 8 kHz, 512
    /// otherwise, matching Silero so the two detectors are interchangeable.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self::with_threshold_dbfs(sample_rate, DEFAULT_ENERGY_THRESHOLD_DBFS)
    }

    /// Detector whose probability crosses 0.5 at `dbfs`.
    #[must_use]
    pub fn with_threshold_dbfs(sample_rate: u32, dbfs: f32) -> Self {
        let window = if sample_rate == 8_000 {
            SILERO_WINDOW_8K
        } else {
            SILERO_WINDOW_16K
        };
        Self {
            window,
            threshold_dbfs: dbfs,
        }
    }

    /// The dBFS level at which this detector returns 0.5.
    #[must_use]
    pub fn threshold_dbfs(&self) -> f32 {
        self.threshold_dbfs
    }
}

impl SpeechDetector for EnergyDetector {
    fn window_size(&self) -> usize {
        self.window
    }

    fn probability(&mut self, window: &[f32]) -> f32 {
        let dbfs = rms_dbfs(window);
        if !dbfs.is_finite() {
            // `rms_dbfs` returns -inf for digital silence (and would return NaN
            // only for NaN input); both mean "no speech here".
            return if dbfs == f32::INFINITY { 1.0 } else { 0.0 };
        }
        let z = (dbfs - self.threshold_dbfs) / SOFTNESS_DB;
        let p: f32 = 1.0 / (1.0 + (-z).exp());
        p.clamp(0.0, 1.0)
    }

    fn reset(&mut self) {
        // Stateless.
    }
}
