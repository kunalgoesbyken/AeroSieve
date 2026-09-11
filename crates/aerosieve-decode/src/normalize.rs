//! Loudness standardization.
//!
//! Emilia-Pipe (arXiv 2407.05361) stage 1 normalizes every source to
//! **-20 dBFS** with the applied gain clamped to **+/-3 dB**, so that badly
//! mastered sources are nudged toward the target without amplifying noise
//! floors or crushing an already-hot master into distortion.

/// RMS level of `samples` in dBFS. Returns `None` for empty or all-zero input.
pub fn rms_dbfs(samples: &[f32]) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let sum_sq: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    if rms <= 0.0 || !rms.is_finite() {
        return None;
    }
    Some(20.0 * (rms as f32).log10())
}

/// Apply RMS loudness normalization in place, returning the gain actually
/// applied in dB.
///
/// The gain needed to reach `target_dbfs` is clamped to `+/-max_gain_db`.
/// Digital silence and empty buffers are a no-op with 0 dB gain.
pub fn normalize_in_place(samples: &mut [f32], target_dbfs: f32, max_gain_db: f32) -> f32 {
    let Some(current) = rms_dbfs(samples) else {
        return 0.0;
    };
    let limit = max_gain_db.abs();
    let gain_db = (target_dbfs - current).clamp(-limit, limit);
    if gain_db == 0.0 {
        return 0.0;
    }
    let factor = 10f32.powf(gain_db / 20.0);
    for s in samples.iter_mut() {
        *s *= factor;
    }
    gain_db
}
