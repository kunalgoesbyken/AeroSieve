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
    let moments = samples
        .chunks(BLOCK)
        .map(LogMoments::of)
        .fold(LogMoments::default(), LogMoments::merge);

    if moments.count < WADA_MIN_SAMPLES {
        return f32::NEG_INFINITY;
    }
    let n = moments.count as f64;
    let g = (moments.sum / n).ln() - moments.sum_ln / n;
    if !g.is_finite() {
        return f32::NEG_INFINITY;
    }
    snr_from_g(g as f32)
}

/// Samples whose mantissas are multiplied before one logarithm is taken.
/// Each mantissa is below 2, so the product stays under `2^BLOCK`, far
/// inside `f64` range.
const BLOCK: usize = 512;

const F32_MANTISSA_BITS: u32 = 23;
const F32_MANTISSA_MASK: u32 = (1 << F32_MANTISSA_BITS) - 1;
const F32_EXPONENT_BIAS: i32 = 127;
const ONE_BITS: u32 = 0x3f80_0000;

/// Count, sum, and sum of natural logs of the nonzero magnitudes in a
/// buffer.
#[derive(Default)]
struct LogMoments {
    count: usize,
    sum: f64,
    sum_ln: f64,
}

impl LogMoments {
    /// A normal `f32` is `mantissa * 2^exponent` with `mantissa` in `[1, 2)`,
    /// so `ln` of a product of them is `ln(product of mantissas)` plus
    /// `ln 2` times the sum of exponents: one logarithm per block rather
    /// than one per sample. Subnormal and infinite magnitudes take the
    /// direct route.
    fn of(block: &[f32]) -> Self {
        let mut moments = Self::default();
        let mut mantissas = 1.0f64;
        let mut exponents = 0i64;
        let nonzero = block
            .iter()
            .map(|sample| sample.abs())
            .filter(|&magnitude| magnitude > 0.0);
        for magnitude in nonzero {
            moments.count += 1;
            moments.sum += f64::from(magnitude);
            if magnitude.is_normal() {
                let bits = magnitude.to_bits();
                let exponent = (bits >> F32_MANTISSA_BITS) as i32 - F32_EXPONENT_BIAS;
                exponents += i64::from(exponent);
                mantissas *= f64::from(f32::from_bits(bits & F32_MANTISSA_MASK | ONE_BITS));
            } else {
                moments.sum_ln += f64::from(magnitude).ln();
            }
        }
        moments.sum_ln += mantissas.ln() + exponents as f64 * std::f64::consts::LN_2;
        moments
    }

    fn merge(self, other: Self) -> Self {
        Self {
            count: self.count + other.count,
            sum: self.sum + other.sum,
            sum_ln: self.sum_ln + other.sum_ln,
        }
    }
}

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
