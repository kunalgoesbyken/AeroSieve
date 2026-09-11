//! Lookup table for WADA-SNR (Waveform Amplitude Distribution Analysis).
//!
//! Reference: Chanwoo Kim and Richard M. Stern, "Robust Signal-to-Noise Ratio
//! Estimation Based on Waveform Amplitude Distribution Analysis", Interspeech
//! 2008, pp. 2598-2601.
//!
//! # What the table is
//!
//! WADA models the amplitude of clean speech as a Gamma distribution with shape
//! parameter 0.4 and additive noise as zero-mean Gaussian. For a signal
//! `x = s + n`, the statistic
//!
//! ```text
//! G(x) = ln(mean(|x|)) - mean(ln(|x|))
//! ```
//!
//! (the log of the ratio between the arithmetic and geometric means of `|x|`)
//! depends only on the SNR, not on the absolute level of the signal. `G` is
//! monotonically increasing in SNR: a pure Gaussian saturates near 0.4094, and
//! a pure Gamma(0.4) signal tends toward roughly 1.63.
//!
//! # How this specific table was produced
//!
//! The published table in the reference implementation is a Monte-Carlo
//! tabulation of that model. This table was regenerated the same way rather
//! than transcribed, so its provenance is reproducible:
//!
//! - draw `N = 2e7` clean samples `s` with `|s| ~ Gamma(shape = 0.4, scale = 1)`
//!   and a uniformly random sign,
//! - draw `N` noise samples `n ~ Normal(0, 1)`,
//! - for each integer SNR from -20 dB to +100 dB, scale the noise to that SNR
//!   against the measured power of `s`, form `x = s + a*n`, and evaluate `G(x)`
//!   over the non-zero `|x|`.
//!
//! Below about -15 dB the curve is saturated (the signal is indistinguishable
//! from pure Gaussian noise) and the raw Monte-Carlo estimate is flat to within
//! ~1e-5, which is not strictly monotonic. A cumulative +1e-6 nudge was applied
//! so the table is strictly increasing and binary search over it is
//! well-defined; the adjustment is far below the estimator's own variance.
//!
//! The resulting values agree with the widely circulated canonical table to
//! about 1e-3 across the whole range (0.4094 at -20 dB, ~1.63 at +100 dB).

/// SNR in dB corresponding to `G_VALS[0]`.
pub const SNR_MIN_DB: f32 = -20.0;

/// SNR in dB corresponding to the last entry of `G_VALS`.
pub const SNR_MAX_DB: f32 = 100.0;

/// Step between consecutive table entries, in dB.
pub const SNR_STEP_DB: f32 = 1.0;

/// `G` statistic values for SNR = -20, -19, ..., +100 dB. Strictly increasing.
pub const G_VALS: [f32; 121] = [
    0.409356, 0.409363, 0.409364, 0.409395, 0.409499, 0.409597, 0.409794, 0.410042, 0.410479,
    0.411050, 0.411886, 0.413117, 0.414712, 0.416921, 0.419766, 0.423671, 0.428468, 0.434466,
    0.441846, 0.450899, 0.461664, 0.474175, 0.488660, 0.505138, 0.523411, 0.543485, 0.565351,
    0.588833, 0.613850, 0.640021, 0.667299, 0.695439, 0.724156, 0.753389, 0.782878, 0.812308,
    0.841815, 0.870932, 0.899821, 0.928215, 0.956052, 0.983098, 1.009571, 1.035477, 1.060501,
    1.084701, 1.108144, 1.130780, 1.152685, 1.173775, 1.194116, 1.213637, 1.232440, 1.250499,
    1.267843, 1.284444, 1.300258, 1.315437, 1.329979, 1.343975, 1.357288, 1.370145, 1.382320,
    1.394077, 1.405324, 1.416013, 1.426282, 1.436094, 1.445454, 1.454384, 1.462946, 1.471132,
    1.478914, 1.486332, 1.493422, 1.500241, 1.506728, 1.512956, 1.518853, 1.524528, 1.529951,
    1.535122, 1.540090, 1.544799, 1.549312, 1.553581, 1.557703, 1.561614, 1.565408, 1.568987,
    1.572426, 1.575654, 1.578772, 1.581740, 1.584584, 1.587298, 1.589870, 1.592349, 1.594686,
    1.596950, 1.599084, 1.601153, 1.603094, 1.604974, 1.606760, 1.608457, 1.610090, 1.611648,
    1.613130, 1.614531, 1.615885, 1.617204, 1.618438, 1.619623, 1.620777, 1.621856, 1.622879,
    1.623861, 1.624801, 1.625695, 1.626555,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_strictly_increasing() {
        for w in G_VALS.windows(2) {
            assert!(w[1] > w[0], "table not strictly increasing at {w:?}");
        }
    }

    #[test]
    fn table_spans_the_documented_snr_range() {
        let expected = ((SNR_MAX_DB - SNR_MIN_DB) / SNR_STEP_DB) as usize + 1;
        assert_eq!(G_VALS.len(), expected);
    }
}
