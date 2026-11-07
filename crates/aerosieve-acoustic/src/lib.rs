//! Deterministic acoustic quality scoring for mono `f32` audio in `[-1, 1]`.
//!
//! [`score`] measures a buffer; [`FilterPolicy`] decides whether to keep it.
//! The two are separate so thresholds can be tuned without touching the
//! measurements, and the same buffer always produces the same
//! [`QualityMetrics`].
//!
//! ```
//! use aerosieve_acoustic::{FULL_SCALE, FilterPolicy, RejectReason, score};
//!
//! let tone: Vec<f32> = (0..16_000)
//!     .map(|i| 0.2 * (std::f32::consts::TAU * 220.0 * i as f32 / 16_000.0).sin())
//!     .collect();
//!
//! let verdict = FilterPolicy::default().evaluate(&score(&tone, 16_000, FULL_SCALE));
//! assert_eq!(verdict, Some(RejectReason::Tonal));
//! ```

mod policy;
mod spectrum;
mod wada;
mod wada_table;

#[cfg(test)]
mod tests;

pub use policy::{FilterPolicy, RejectReason};
pub use spectrum::{FLATNESS_BAND_HZ, FLATNESS_FFT_SIZE, spectral_flatness};
pub use wada::{WADA_MIN_SAMPLES, wada_snr_db};

/// Full scale of audio that has not been rescaled since it was recorded.
pub const FULL_SCALE: f32 = 1.0;

/// Fraction of full scale at or above which a sample counts as clipped.
pub const CLIP_THRESHOLD: f32 = 0.999;

/// Measurements for one buffer.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QualityMetrics {
    /// See [`wada_snr_db`].
    pub wada_snr_db: f32,
    /// See [`rms_dbfs`].
    pub rms_dbfs: f32,
    /// See [`clip_ratio`].
    pub clip_ratio: f32,
    /// See [`spectral_flatness`].
    pub spectral_flatness: f32,
}

/// Compute every metric for one buffer. Never panics, whatever the input.
///
/// `full_scale` is the magnitude the recording's full scale now sits at:
/// [`FULL_SCALE`] for untouched audio, lower if the buffer was turned down
/// after recording. Clipping is measured against it, so normalization cannot
/// hide a clipped source.
pub fn score(samples: &[f32], sample_rate: u32, full_scale: f32) -> QualityMetrics {
    QualityMetrics {
        wada_snr_db: wada_snr_db(samples),
        rms_dbfs: rms_dbfs(samples),
        clip_ratio: clip_ratio(samples, full_scale),
        spectral_flatness: spectral_flatness(samples, sample_rate),
    }
}

/// RMS level in dBFS: `20 * log10(rms)`.
///
/// `f32::NEG_INFINITY` for an empty or all-zero buffer.
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return f32::NEG_INFINITY;
    }
    let sum_of_squares: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = (sum_of_squares / samples.len() as f64).sqrt();
    (20.0 * rms.log10()) as f32
}

/// Fraction of samples whose magnitude reaches [`CLIP_THRESHOLD`] of
/// `full_scale`, in `[0, 1]`.
pub fn clip_ratio(samples: &[f32], full_scale: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let level = CLIP_THRESHOLD * full_scale;
    let clipped = samples.iter().filter(|s| s.abs() >= level).count();
    clipped as f32 / samples.len() as f32
}
