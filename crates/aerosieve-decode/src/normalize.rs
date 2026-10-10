use aerosieve_acoustic::rms_dbfs;

/// Move the RMS level toward `target_dbfs`, applying at most `max_gain_db` of
/// gain in either direction. Returns the gain applied, in dB.
///
/// The clamp keeps a quiet source from having its noise floor amplified and a
/// hot one from being crushed. Silence is left untouched.
pub fn normalize_in_place(samples: &mut [f32], target_dbfs: f32, max_gain_db: f32) -> f32 {
    let current_dbfs = rms_dbfs(samples);
    if !current_dbfs.is_finite() {
        return 0.0;
    }
    let limit = max_gain_db.abs();
    let gain_db = (target_dbfs - current_dbfs).clamp(-limit, limit);
    let factor = 10f32.powf(gain_db / 20.0);
    samples.iter_mut().for_each(|sample| *sample *= factor);
    gain_db
}
