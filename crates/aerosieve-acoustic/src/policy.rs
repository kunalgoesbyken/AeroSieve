//! The acceptance policy applied to [`QualityMetrics`].

use crate::QualityMetrics;

/// Why a buffer was rejected.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// No measurable level: digital silence or an empty buffer.
    Silence,
    /// Audible, but below [`FilterPolicy::min_dbfs`].
    TooQuiet,
    /// Above [`FilterPolicy::max_dbfs`].
    TooLoud,
    /// More than [`FilterPolicy::max_clip_ratio`] of samples at full scale.
    Clipping,
    /// Spectral flatness below [`FilterPolicy::min_spectral_flatness`]: a tone
    /// or music rather than speech.
    Tonal,
    /// WADA-SNR below [`FilterPolicy::min_snr_db`], or too short to estimate.
    LowSnr,
}

impl RejectReason {
    /// Stable identifier, identical to the serialized form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Silence => "silence",
            Self::TooQuiet => "too_quiet",
            Self::TooLoud => "too_loud",
            Self::Clipping => "clipping",
            Self::Tonal => "tonal",
            Self::LowSnr => "low_snr",
        }
    }
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// Thresholds a buffer must meet to be kept.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FilterPolicy {
    /// WADA-SNR floor, in dB. WADA is biased low, so a strict floor discards
    /// usable speech.
    pub min_snr_db: f32,
    /// Largest tolerated fraction of clipped samples.
    pub max_clip_ratio: f32,
    /// RMS level floor, in dBFS.
    pub min_dbfs: f32,
    /// RMS level ceiling, in dBFS.
    pub max_dbfs: f32,
    /// Spectral flatness floor. Pure tones read below 1e-4; the speech-like
    /// fixtures this default was calibrated on read above 0.1.
    pub min_spectral_flatness: f32,
}

impl Default for FilterPolicy {
    fn default() -> Self {
        Self {
            min_snr_db: 10.0,
            max_clip_ratio: 0.01,
            min_dbfs: -40.0,
            max_dbfs: -3.0,
            min_spectral_flatness: 0.02,
        }
    }
}

impl FilterPolicy {
    /// `None` if the buffer is accepted, otherwise the first failed check.
    ///
    /// Checks run from the most specific cause to the least: a silent buffer
    /// reports `Silence` rather than `LowSnr`, and a pure tone reports `Tonal`
    /// even though WADA also scores it as noise.
    pub fn evaluate(&self, metrics: &QualityMetrics) -> Option<RejectReason> {
        if !metrics.rms_dbfs.is_finite() {
            return Some(RejectReason::Silence);
        }
        if metrics.rms_dbfs < self.min_dbfs {
            return Some(RejectReason::TooQuiet);
        }
        if metrics.rms_dbfs > self.max_dbfs {
            return Some(RejectReason::TooLoud);
        }
        if metrics.clip_ratio > self.max_clip_ratio {
            return Some(RejectReason::Clipping);
        }
        if metrics.spectral_flatness < self.min_spectral_flatness {
            return Some(RejectReason::Tonal);
        }
        if metrics.wada_snr_db.is_nan() || metrics.wada_snr_db < self.min_snr_db {
            return Some(RejectReason::LowSnr);
        }
        None
    }

    pub fn accepts(&self, metrics: &QualityMetrics) -> bool {
        self.evaluate(metrics).is_none()
    }
}
