//! WADA-SNR: Kim & Stern, "Robust Signal-to-Noise Ratio Estimation Based on
//! Waveform Amplitude Distribution Analysis", Interspeech 2008.

use crate::wada_table::{G_VALS, SNR_MAX_DB, SNR_MIN_DB, SNR_STEP_DB};

/// Fewest non-zero samples for which an estimate is reported. Below this the
/// statistic is dominated by sampling variance.
pub const WADA_MIN_SAMPLES: usize = 256;

/// Estimate SNR in dB from the amplitude distribution alone, with no clean
/// reference.
///
/// Clean speech is modeled as Gamma(0.4) and noise as Gaussian. The statistic
/// `G = ln(mean(|x|)) - mean(ln(|x|))` over the non-zero samples depends only
/// on SNR, not on level, and is mapped to dB through a lookup table.
///
/// The result lies in `[-20, 100]` dB. A signal that does not resemble speech
/// (a pure tone, a DC offset) reads near the bottom of that range. Returns
/// `f32::NEG_INFINITY` when fewer than [`WADA_MIN_SAMPLES`] non-zero samples
/// are available, so that any SNR floor rejects the buffer.
pub fn wada_snr_db(samples: &[f32]) -> f32 {
    let (count, sum, sum_ln) = samples
        .iter()
        .map(|s| f64::from(s.abs()))
        .filter(|&magnitude| magnitude > 0.0)
        .fold((0usize, 0.0, 0.0), |(count, sum, sum_ln), magnitude| {
            (count + 1, sum + magnitude, sum_ln + magnitude.ln())
        });

    if count < WADA_MIN_SAMPLES {
        return f32::NEG_INFINITY;
    }
    let n = count as f64;
    let g = (sum / n).ln() - sum_ln / n;
    if !g.is_finite() {
        return f32::NEG_INFINITY;
    }
    snr_from_g(g as f32)
}

/// Linearly interpolate the lookup table, clamping outside its range.
pub(crate) fn snr_from_g(g: f32) -> f32 {
    let last = G_VALS.len() - 1;
    if g <= G_VALS[0] {
        return SNR_MIN_DB;
    }
    if g >= G_VALS[last] {
        return SNR_MAX_DB;
    }
    let upper = G_VALS.partition_point(|&value| value <= g);
    let lower = upper - 1;
    let fraction = (g - G_VALS[lower]) / (G_VALS[upper] - G_VALS[lower]);
    SNR_MIN_DB + (lower as f32 + fraction) * SNR_STEP_DB
}
