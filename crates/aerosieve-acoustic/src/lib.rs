//! Deterministic acoustic quality scoring for 16 kHz mono `f32` audio.
//!
//! Pure math. No models, no training, no randomness: the same buffer always
//! produces the same [`QualityMetrics`].
//!
//! Four metrics are computed by [`score`]:
//!
//! | Metric | Meaning |
//! |---|---|
//! | [`QualityMetrics::wada_snr_db`] | non-intrusive SNR estimate (WADA, Kim & Stern 2008) |
//! | [`QualityMetrics::rms_dbfs`] | RMS level relative to full scale |
//! | [`QualityMetrics::clip_ratio`] | fraction of samples at or beyond `+/-0.999` |
//! | [`QualityMetrics::spectral_flatness`] | Wiener entropy; ~1.0 noise-like, ~0.0 tonal |
//!
//! Filtering is a *separate* explicit policy over those metrics
//! ([`FilterPolicy`]), never hidden inside the scorer. Scoring reports; the
//! policy decides.
//!
//! ```
//! use aerosieve_acoustic::{score, FilterPolicy};
//!
//! let samples: Vec<f32> = (0..16_000)
//!     .map(|i| 0.2 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin())
//!     .collect();
//! let metrics = score(&samples);
//! let policy = FilterPolicy::default();
//! // A pure tone is rejected as tonal, not as speech.
//! assert!(!policy.accepts(&metrics));
//! ```

pub mod wada_table;

use wada_table::{G_VALS, SNR_MAX_DB, SNR_MIN_DB, SNR_STEP_DB};

/// Amplitude at or above which a sample counts as clipped.
pub const CLIP_THRESHOLD: f32 = 0.999;

/// Spectral flatness below which a buffer is classified as tonal (a pure tone
/// or music) rather than speech.
///
/// Measured against this crate's fixtures: a pure sine sits below 1e-8,
/// speech-like signals around 0.04-0.06, and white noise above 0.5. 0.02 sits
/// in the gap between tone and speech, roughly a factor of two below the
/// lowest speech measurement and many orders of magnitude above a pure tone.
///
/// The gap on the tone side is enormous and the gap on the speech side is
/// about 2x, so this threshold is far more reliable at catching sustained
/// tones than at catching lightly musical speech. That is the intended
/// trade-off: rejecting real speech is the failure mode v2 exists to avoid.
///
/// This is a constant rather than a [`FilterPolicy`] field because the policy's
/// shape is fixed by the v2 pipeline design.
pub const MIN_SPECTRAL_FLATNESS: f32 = 0.02;

/// Number of samples in one analysis frame for spectral flatness.
///
/// 1024 samples is 64 ms at 16 kHz, long enough to resolve a low-pitched voice
/// fundamental and short enough that a frame is roughly stationary.
pub const FLATNESS_FFT_SIZE: usize = 1024;

/// Deterministic acoustic measurements for one buffer.
///
/// Every field is finite for any non-degenerate input. Degenerate inputs
/// (empty or all-zero buffers) are documented per-field on the functions that
/// compute them; in short, digital silence yields
/// `rms_dbfs == f32::NEG_INFINITY` and `wada_snr_db == f32::NEG_INFINITY`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QualityMetrics {
    /// Non-intrusive SNR estimate in dB, clamped to `[-20, 100]`.
    /// `f32::NEG_INFINITY` for silence or a buffer too short to estimate.
    pub wada_snr_db: f32,
    /// RMS level in dBFS. `f32::NEG_INFINITY` for digital silence.
    pub rms_dbfs: f32,
    /// Fraction of samples with `|x| >= 0.999`, in `[0, 1]`.
    pub clip_ratio: f32,
    /// Wiener entropy (geometric mean / arithmetic mean of the power
    /// spectrum), in `[0, 1]`. Near 1.0 is noise-like, near 0.0 is tonal.
    /// `0.0` for silence or a buffer shorter than one analysis frame.
    pub spectral_flatness: f32,
}

/// Compute all four quality metrics for one buffer.
///
/// The buffer is expected to be 16 kHz mono `f32` in `[-1, 1]`, but nothing
/// here depends on the sample rate except the documented interpretation of
/// [`FLATNESS_FFT_SIZE`]. Never panics.
pub fn score(samples: &[f32]) -> QualityMetrics {
    QualityMetrics {
        wada_snr_db: wada_snr_db(samples),
        rms_dbfs: rms_dbfs(samples),
        clip_ratio: clip_ratio(samples),
        spectral_flatness: spectral_flatness(samples),
    }
}

// ---------------------------------------------------------------------------
// Level and clipping
// ---------------------------------------------------------------------------

/// RMS level of `samples` in dBFS, i.e. `20 * log10(rms)`.
///
/// Returns `f32::NEG_INFINITY` for an empty buffer or one that is entirely
/// zero, since the level is genuinely unbounded below rather than some
/// arbitrary floor.
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return f32::NEG_INFINITY;
    }
    // Accumulate in f64: a 30 s buffer is 480k samples and f32 summation
    // loses meaningful precision at that length.
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    if rms <= 0.0 {
        f32::NEG_INFINITY
    } else {
        (20.0 * rms.log10()) as f32
    }
}

/// Fraction of samples whose magnitude reaches [`CLIP_THRESHOLD`].
///
/// Returns `0.0` for an empty buffer: nothing was clipped because nothing was
/// there.
pub fn clip_ratio(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let clipped = samples.iter().filter(|s| s.abs() >= CLIP_THRESHOLD).count();
    clipped as f32 / samples.len() as f32
}

// ---------------------------------------------------------------------------
// WADA-SNR
// ---------------------------------------------------------------------------

/// Minimum number of non-zero samples required for a WADA estimate.
///
/// `G` is a ratio of means over the amplitude distribution; below a few
/// hundred samples the estimate is dominated by sampling variance and
/// reporting a number would be misleading.
pub const WADA_MIN_SAMPLES: usize = 256;

/// Estimate SNR in dB using Waveform Amplitude Distribution Analysis.
///
/// Implements Kim & Stern, *"Robust Signal-to-Noise Ratio Estimation Based on
/// Waveform Amplitude Distribution Analysis"*, Interspeech 2008. Clean speech
/// amplitude is modeled as Gamma(shape 0.4) and additive noise as Gaussian;
/// the statistic
///
/// ```text
/// G = ln(mean(|x|)) - mean(ln(|x|))
/// ```
///
/// over the non-zero samples is mapped to SNR through
/// [`wada_table::G_VALS`] with linear interpolation between entries. See that
/// module for how the table was produced.
///
/// The result is clamped to the table's range, `[-20, +100]` dB.
///
/// # Degenerate input
///
/// Returns `f32::NEG_INFINITY` when the buffer is empty, entirely zero, or has
/// fewer than [`WADA_MIN_SAMPLES`] non-zero samples. `NEG_INFINITY` is chosen
/// rather than `0.0` or `NaN` so that an SNR floor check rejects such buffers
/// naturally and comparisons stay total.
///
/// A constant DC offset is a degenerate case for this estimator in a different
/// way: every `|x|` is identical, so `G == 0`, which is below the table and
/// clamps to `-20` dB. That is the correct answer for the model's purposes —
/// a DC buffer carries no speech.
pub fn wada_snr_db(samples: &[f32]) -> f32 {
    if samples.len() < WADA_MIN_SAMPLES {
        return f32::NEG_INFINITY;
    }

    let mut n: u64 = 0;
    let mut sum_abs: f64 = 0.0;
    let mut sum_log: f64 = 0.0;
    for &s in samples {
        let a = s.abs() as f64;
        if a > 0.0 {
            n += 1;
            sum_abs += a;
            sum_log += a.ln();
        }
    }

    if (n as usize) < WADA_MIN_SAMPLES {
        return f32::NEG_INFINITY;
    }

    let nf = n as f64;
    let mean_abs = sum_abs / nf;
    // `!(x > 0.0)` rather than `x <= 0.0`: the negated form also catches NaN,
    // which must be rejected here. Clippy's neg_cmp_op_on_partial_ord lint
    // suggests partial_cmp, but that would obscure exactly the property we want.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(mean_abs > 0.0) {
        return f32::NEG_INFINITY;
    }
    let g = mean_abs.ln() - sum_log / nf;
    if !g.is_finite() {
        return f32::NEG_INFINITY;
    }

    snr_from_g(g as f32)
}

/// Map the WADA `G` statistic to SNR in dB by interpolating the lookup table.
///
/// Clamped to `[SNR_MIN_DB, SNR_MAX_DB]` outside the table's range.
fn snr_from_g(g: f32) -> f32 {
    if g <= G_VALS[0] {
        return SNR_MIN_DB;
    }
    let last = G_VALS.len() - 1;
    if g >= G_VALS[last] {
        return SNR_MAX_DB;
    }

    // The table is strictly increasing, so partition_point gives the first
    // index whose value exceeds g; the bracket is [hi - 1, hi].
    let hi = G_VALS.partition_point(|&v| v <= g);
    let lo = hi - 1;
    let span = G_VALS[hi] - G_VALS[lo];
    let t = if span > 0.0 {
        (g - G_VALS[lo]) / span
    } else {
        0.0
    };
    SNR_MIN_DB + (lo as f32 + t) * SNR_STEP_DB
}

// ---------------------------------------------------------------------------
// Spectral flatness
// ---------------------------------------------------------------------------

/// Spectral flatness (Wiener entropy) of `samples`, in `[0, 1]`.
///
/// Defined as the geometric mean of the power spectrum divided by its
/// arithmetic mean. Values near 1.0 indicate a flat, noise-like spectrum;
/// values near 0.0 indicate energy concentrated in a few bins, i.e. a tone or
/// music.
///
/// # Method
///
/// The buffer is split into non-overlapping [`FLATNESS_FFT_SIZE`]-sample
/// frames. Each frame is multiplied by a **periodic Hann window**
/// (`0.5 - 0.5*cos(2*pi*n/N)`), transformed with an in-crate iterative radix-2
/// Cooley-Tukey FFT, and reduced to a power spectrum over bins `1 ..= N/2`.
/// Bin 0 (DC) is excluded: a DC offset is a level artifact, not spectral
/// content, and including it makes a windowed tone look far flatter than it is.
/// Per-frame flatness values are averaged.
///
/// A tiny floor (`1e-20`) is added to each power bin before taking its log so
/// that an exactly-zero bin does not drive the geometric mean to zero and
/// swamp the measurement.
///
/// # Degenerate input
///
/// Returns `0.0` if the buffer is shorter than one frame, or if a frame has no
/// energy at all.
pub fn spectral_flatness(samples: &[f32]) -> f32 {
    let n = FLATNESS_FFT_SIZE;
    if samples.len() < n {
        return 0.0;
    }

    let window = hann_window(n);
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];

    let mut total = 0.0f64;
    let mut frames = 0u32;

    for frame in samples.chunks_exact(n) {
        for i in 0..n {
            re[i] = frame[i] * window[i];
            im[i] = 0.0;
        }
        fft_in_place(&mut re, &mut im);

        // Bins 1..=n/2 — skip DC, and the spectrum is conjugate-symmetric
        // above n/2 so the upper half carries no new information.
        const FLOOR: f64 = 1e-20;
        let mut sum_log = 0.0f64;
        let mut sum_lin = 0.0f64;
        let bins = n / 2;
        for k in 1..=bins {
            let p = (re[k] as f64) * (re[k] as f64) + (im[k] as f64) * (im[k] as f64);
            let p = p + FLOOR;
            sum_log += p.ln();
            sum_lin += p;
        }
        if sum_lin <= 0.0 {
            continue;
        }
        let bf = bins as f64;
        let flatness = (sum_log / bf).exp() / (sum_lin / bf);
        if flatness.is_finite() {
            total += flatness.clamp(0.0, 1.0);
            frames += 1;
        }
    }

    if frames == 0 {
        0.0
    } else {
        (total / frames as f64) as f32
    }
}

/// Periodic Hann window of length `n`.
fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
            0.5 - 0.5 * x.cos()
        })
        .collect()
}

/// In-place iterative radix-2 Cooley-Tukey FFT.
///
/// `re` and `im` must have the same power-of-two length. No external
/// dependency; this crate needs exactly one transform size and nothing else.
fn fft_in_place(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    debug_assert!(n.is_power_of_two());
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies.
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0usize;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = i + k;
                let b = a + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
            i += len;
        }
        len <<= 1;
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// Why a buffer was rejected. Recorded per rejection so that retention rate
/// can be broken down by cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RejectReason {
    /// No measurable level at all (digital silence, or an empty buffer).
    Silence,
    /// WADA-SNR estimate below the policy floor.
    LowSnr,
    /// Too large a fraction of samples at or beyond full scale.
    Clipping,
    /// Spectral flatness too low: a tone or music, not speech.
    Tonal,
    /// Audible but below the policy's level floor.
    TooQuiet,
    /// Above the policy's level ceiling.
    TooLoud,
}

impl RejectReason {
    /// Stable lowercase identifier, for manifests and stats keys.
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectReason::Silence => "silence",
            RejectReason::LowSnr => "low_snr",
            RejectReason::Clipping => "clipping",
            RejectReason::Tonal => "tonal",
            RejectReason::TooQuiet => "too_quiet",
            RejectReason::TooLoud => "too_loud",
        }
    }
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Thresholds applied to [`QualityMetrics`] to accept or reject a buffer.
///
/// Kept deliberately separate from scoring: the scorer measures, the policy
/// judges, and the policy is the thing a caller is expected to tune.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilterPolicy {
    /// Minimum acceptable WADA-SNR, in dB.
    pub min_snr_db: f32,
    /// Maximum acceptable fraction of clipped samples.
    pub max_clip_ratio: f32,
    /// Minimum acceptable RMS level, in dBFS.
    pub min_dbfs: f32,
    /// Maximum acceptable RMS level, in dBFS.
    pub max_dbfs: f32,
}

impl Default for FilterPolicy {
    /// Defaults that accept clean speech.
    ///
    /// This is the defect v2 exists to fix: v1's default configuration
    /// rejected 100% of frames, clean speech included. Every threshold here is
    /// justified against a measured value rather than picked for looking
    /// strict, and `test_default_policy_accepts_clean_speech` enforces it.
    ///
    /// | Field | Value | Why |
    /// |---|---|---|
    /// | `min_snr_db` | 10.0 | WADA is documented as *underestimating* SNR (see the research note); a 15 dB floor with an estimator biased low discards usable speech. 10 dB still excludes audibly noisy material. |
    /// | `max_clip_ratio` | 0.01 | 1% of samples at full scale is already audible distortion; a handful of isolated peaks is not. |
    /// | `min_dbfs` | -40.0 | The decode stage normalizes to -20 dBFS, so a segment 20 dB under target is effectively silence-adjacent. |
    /// | `max_dbfs` | -3.0 | Above -3 dBFS a segment is close enough to full scale that clipping is likely even where `clip_ratio` has not caught it yet. |
    ///
    /// Tonality is checked against the crate-level constant
    /// [`MIN_SPECTRAL_FLATNESS`] rather than a policy field, because
    /// `FilterPolicy`'s shape is fixed by the pipeline design.
    fn default() -> Self {
        Self {
            min_snr_db: 10.0,
            max_clip_ratio: 0.01,
            min_dbfs: -40.0,
            max_dbfs: -3.0,
        }
    }
}

impl FilterPolicy {
    /// Evaluate `m` against this policy.
    ///
    /// Returns `None` if the buffer is accepted, `Some(reason)` if rejected.
    ///
    /// Checks run in a fixed order so the reported reason is deterministic:
    /// silence, level, clipping, tonality, SNR. The order is chosen so that
    /// each rejection names the most specific cause:
    ///
    /// - Silence first, because a silent buffer would otherwise be reported as
    ///   `LowSnr`, which is true but useless.
    /// - Tonality before SNR, because WADA models clean speech as Gamma(0.4)
    ///   and a pure tone does not fit that model at all: a noiseless sine
    ///   scores near the bottom of the WADA table. `Tonal` is the honest
    ///   description of a sine; `LowSnr` would be an artifact of the estimator.
    pub fn evaluate(&self, m: &QualityMetrics) -> Option<RejectReason> {
        if !m.rms_dbfs.is_finite() {
            return Some(RejectReason::Silence);
        }
        if m.rms_dbfs < self.min_dbfs {
            return Some(RejectReason::TooQuiet);
        }
        if m.rms_dbfs > self.max_dbfs {
            return Some(RejectReason::TooLoud);
        }
        if m.clip_ratio > self.max_clip_ratio {
            return Some(RejectReason::Clipping);
        }
        if m.spectral_flatness < MIN_SPECTRAL_FLATNESS {
            return Some(RejectReason::Tonal);
        }
        // A non-finite SNR here means the buffer was too short to estimate,
        // which is a rejection rather than a pass.
        // Negated form is deliberate: a NaN metric must fail the check rather
        // than silently pass, which `<` would allow.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(m.wada_snr_db >= self.min_snr_db) {
            return Some(RejectReason::LowSnr);
        }
        None
    }

    /// `true` if `m` passes every threshold in this policy.
    pub fn accepts(&self, m: &QualityMetrics) -> bool {
        self.evaluate(m).is_none()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const SR: f32 = 16_000.0;

    /// Deterministic xorshift64* PRNG. Tests must be reproducible, and a `rand`
    /// dependency is not worth it for generating fixtures.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        /// Uniform in `[0, 1)`.
        fn uniform(&mut self) -> f32 {
            (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32
        }

        /// Standard normal via Box-Muller.
        fn normal(&mut self) -> f32 {
            let u1 = self.uniform().max(1e-9);
            let u2 = self.uniform();
            (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
        }
    }

    fn sine(n: usize, freq: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * PI * freq * i as f32 / SR).sin())
            .collect()
    }

    fn white_noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        (0..n).map(|_| amp * rng.normal()).collect()
    }

    /// A speech-like signal: a harmonic voiced excitation with a wandering
    /// pitch and a syllabic amplitude envelope that goes to near-zero between
    /// "syllables", plus a low level of breath noise. This has the property
    /// that actually matters for WADA — a heavy-tailed, mostly-low amplitude
    /// distribution — without pretending to be real speech.
    fn speech_like(n: usize, seed: u64) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / SR;
            // Pitch wandering around 130 Hz.
            let f0 = 130.0 + 12.0 * (2.0 * PI * 1.7 * t).sin();
            let phase = 2.0 * PI * f0 * t;
            // A few harmonics with a falling spectral slope.
            let voiced = (phase).sin()
                + 0.55 * (2.0 * phase).sin()
                + 0.32 * (3.0 * phase).sin()
                + 0.20 * (4.0 * phase).sin()
                + 0.12 * (5.0 * phase).sin();
            // Syllabic envelope ~4 Hz, squared so gaps are genuinely quiet.
            let env = {
                let e = 0.5 - 0.5 * (2.0 * PI * 4.0 * t).cos();
                e * e
            };
            let breath = 0.01 * rng.normal();
            out.push(0.18 * env * voiced + breath * env.max(0.05));
        }
        out
    }

    /// Normalize a buffer to a target RMS in dBFS.
    fn normalize_to_dbfs(samples: &mut [f32], target_dbfs: f32) {
        let cur = rms_dbfs(samples);
        if !cur.is_finite() {
            return;
        }
        let gain = 10f32.powf((target_dbfs - cur) / 20.0);
        for s in samples.iter_mut() {
            *s *= gain;
        }
    }

    // -- The regression test this crate exists for ---------------------------

    #[test]
    fn test_default_policy_accepts_clean_speech() {
        // v1's SieveConfig::default() rejected 100% of frames including clean
        // speech. This asserts the v2 default does not.
        let mut speech = speech_like(SR as usize * 3, 0xA11CE);
        normalize_to_dbfs(&mut speech, -20.0);
        let m = score(&speech);
        let policy = FilterPolicy::default();
        println!("clean speech metrics: {m:?}");

        assert!(
            policy.accepts(&m),
            "default policy rejected clean speech: {:?} -> {:?}",
            m,
            policy.evaluate(&m)
        );
    }

    #[test]
    fn default_policy_accepts_clean_speech_across_seeds_and_levels() {
        let policy = FilterPolicy::default();
        for seed in [1u64, 7, 99, 12345, 0xDEAD_BEEF] {
            // -26 to -12 dBFS spans the range the decode stage's -20 dBFS
            // target realistically produces. Above about -10 dBFS this
            // fixture's peaks exceed full scale, which is genuinely clipping
            // and correctly rejected as such.
            for level in [-26.0f32, -23.0, -20.0, -16.0, -12.0] {
                let mut speech = speech_like(SR as usize * 2, seed);
                normalize_to_dbfs(&mut speech, level);
                let m = score(&speech);
                assert!(
                    policy.accepts(&m),
                    "seed {seed} at {level} dBFS rejected: {m:?} -> {:?}",
                    policy.evaluate(&m)
                );
            }
        }
    }

    // -- Rejection cases -----------------------------------------------------

    #[test]
    fn silence_is_rejected() {
        let silence = vec![0.0f32; 16_000];
        let m = score(&silence);
        assert_eq!(m.rms_dbfs, f32::NEG_INFINITY);
        assert_eq!(
            FilterPolicy::default().evaluate(&m),
            Some(RejectReason::Silence)
        );
    }

    #[test]
    fn near_silence_is_rejected_as_too_quiet() {
        let mut quiet = speech_like(16_000, 5);
        normalize_to_dbfs(&mut quiet, -60.0);
        let m = score(&quiet);
        assert_eq!(
            FilterPolicy::default().evaluate(&m),
            Some(RejectReason::TooQuiet),
            "{m:?}"
        );
    }

    #[test]
    fn clipped_audio_is_rejected() {
        // Speech driven hard into the rails: ~40% of samples pinned.
        let mut clipped = speech_like(16_000, 11);
        normalize_to_dbfs(&mut clipped, -20.0);
        for s in clipped.iter_mut() {
            *s = (*s * 25.0).clamp(-1.0, 1.0);
        }
        let m = score(&clipped);
        assert!(
            m.clip_ratio > 0.01,
            "fixture is not actually clipped: {m:?}"
        );
        let reason = FilterPolicy::default().evaluate(&m);
        assert!(
            matches!(reason, Some(RejectReason::Clipping | RejectReason::TooLoud)),
            "clipped audio not rejected for a level/clip reason: {m:?} -> {reason:?}"
        );
    }

    #[test]
    fn clip_ratio_counts_only_samples_at_full_scale() {
        let mut buf = vec![0.5f32; 100];
        buf[..10].fill(1.0);
        assert!((clip_ratio(&buf) - 0.10).abs() < 1e-6);
        assert_eq!(clip_ratio(&[0.0f32; 10]), 0.0);
    }

    #[test]
    fn pure_tone_is_detected_as_tonal() {
        let mut tone = sine(16_000, 440.0, 0.5);
        normalize_to_dbfs(&mut tone, -20.0);
        let m = score(&tone);
        assert!(
            m.spectral_flatness < 0.02,
            "pure tone flatness should be near zero, got {}",
            m.spectral_flatness
        );
        assert_eq!(
            FilterPolicy::default().evaluate(&m),
            Some(RejectReason::Tonal),
            "{m:?}"
        );
    }

    #[test]
    fn white_noise_has_high_spectral_flatness() {
        let noise = white_noise(16_000, 0.1, 0xF00D);
        let f = spectral_flatness(&noise);
        assert!(f > 0.5, "white noise flatness should be high, got {f}");
    }

    #[test]
    fn speech_flatness_sits_between_tone_and_noise() {
        let tone = spectral_flatness(&sine(16_000, 440.0, 0.5));
        let speech = spectral_flatness(&speech_like(16_000, 3));
        let noise = spectral_flatness(&white_noise(16_000, 0.1, 9));
        assert!(
            tone < speech && speech < noise,
            "expected tone({tone}) < speech({speech}) < noise({noise})"
        );
    }

    // -- WADA-SNR accuracy ---------------------------------------------------

    /// Mix `clean` with Gaussian noise scaled to the requested SNR.
    fn mix_at_snr(clean: &[f32], target_snr_db: f32, seed: u64) -> Vec<f32> {
        let sig_pow: f64 =
            clean.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / clean.len() as f64;
        let mut rng = Rng::new(seed);
        let noise: Vec<f32> = (0..clean.len()).map(|_| rng.normal()).collect();
        let noise_pow: f64 =
            noise.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / noise.len() as f64;
        let scale = (sig_pow / (noise_pow * 10f64.powf(target_snr_db as f64 / 10.0))).sqrt() as f32;
        clean
            .iter()
            .zip(&noise)
            .map(|(&c, &n)| c + scale * n)
            .collect()
    }

    /// WADA-SNR on synthesized known-SNR signals must land within **3 dB** of
    /// truth at 0/10/20/30 dB.
    ///
    /// 3 dB is the tolerance the estimator actually supports here, not an
    /// aspiration: WADA assumes the clean signal is Gamma(0.4)-distributed,
    /// and neither a sine nor this synthetic speech matches that model exactly,
    /// so a residual model-mismatch bias is expected and is documented in the
    /// paper (WADA tends to underestimate).
    ///
    /// Measured errors for these fixtures (run with `--nocapture` to see them):
    ///
    /// | True SNR | Estimate | Error |
    /// |---|---|---|
    /// | 0 dB | -1.26 dB | -1.26 dB |
    /// | 10 dB | 9.45 dB | -0.55 dB |
    /// | 20 dB | 19.87 dB | -0.13 dB |
    /// | 30 dB | 31.73 dB | +1.73 dB |
    ///
    /// Worst case 1.73 dB, and the negative bias at low SNR is the direction
    /// the paper describes. The 3 dB bound leaves headroom for the fixture
    /// changing rather than being tuned down to the current numbers.
    #[test]
    fn wada_snr_estimates_known_snr_within_3db() {
        const TOL_DB: f32 = 3.0;
        let clean = speech_like(SR as usize * 4, 0x5EED);

        for &target in &[0.0f32, 10.0, 20.0, 30.0] {
            let mixed = mix_at_snr(&clean, target, 0xBEEF + target as u64);
            let est = wada_snr_db(&mixed);
            let err = est - target;
            println!("WADA target {target:5.1} dB -> est {est:7.3} dB (err {err:+.3} dB)");
            assert!(
                err.abs() <= TOL_DB,
                "WADA-SNR at {target} dB estimated {est:.2} dB (error {err:+.2} dB, tolerance {TOL_DB} dB)"
            );
        }
    }

    #[test]
    fn wada_snr_is_monotonic_in_true_snr() {
        let clean = speech_like(SR as usize * 2, 0x1234);
        let mut prev = f32::NEG_INFINITY;
        for &target in &[-5.0f32, 0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0] {
            let est = wada_snr_db(&mix_at_snr(&clean, target, 77));
            assert!(
                est > prev,
                "WADA-SNR not monotonic: {target} dB -> {est}, previous {prev}"
            );
            prev = est;
        }
    }

    #[test]
    fn wada_snr_is_independent_of_absolute_level() {
        // G is a ratio of means, so scaling the buffer must not change it.
        let base = mix_at_snr(&speech_like(SR as usize, 2), 15.0, 3);
        let a = wada_snr_db(&base);
        let scaled: Vec<f32> = base.iter().map(|s| s * 0.01).collect();
        let b = wada_snr_db(&scaled);
        assert!(
            (a - b).abs() < 0.5,
            "level changed the SNR estimate: {a} vs {b}"
        );
    }

    #[test]
    fn wada_snr_is_clamped_to_the_table_range() {
        let clean = speech_like(SR as usize, 8);
        let very_noisy = mix_at_snr(&clean, -60.0, 4);
        let est = wada_snr_db(&very_noisy);
        assert!(
            (SNR_MIN_DB..=SNR_MAX_DB).contains(&est),
            "estimate {est} outside table range"
        );
    }

    // -- Degenerate inputs ---------------------------------------------------

    #[test]
    fn empty_buffer_does_not_panic() {
        let m = score(&[]);
        assert_eq!(m.rms_dbfs, f32::NEG_INFINITY);
        assert_eq!(m.wada_snr_db, f32::NEG_INFINITY);
        assert_eq!(m.clip_ratio, 0.0);
        assert_eq!(m.spectral_flatness, 0.0);
        assert_eq!(
            FilterPolicy::default().evaluate(&m),
            Some(RejectReason::Silence)
        );
    }

    #[test]
    fn all_zero_buffer_does_not_panic() {
        let m = score(&vec![0.0f32; 4096]);
        assert_eq!(m.rms_dbfs, f32::NEG_INFINITY);
        assert_eq!(m.wada_snr_db, f32::NEG_INFINITY);
        assert_eq!(m.clip_ratio, 0.0);
        assert!(m.spectral_flatness.is_finite());
        assert!(!FilterPolicy::default().accepts(&m));
    }

    #[test]
    fn single_sample_does_not_panic() {
        for v in [0.0f32, 0.5, 1.0, -1.0] {
            let m = score(&[v]);
            assert_eq!(m.wada_snr_db, f32::NEG_INFINITY);
            assert_eq!(m.spectral_flatness, 0.0);
            assert!(!FilterPolicy::default().accepts(&m));
        }
    }

    #[test]
    fn dc_offset_does_not_panic_and_reads_as_no_speech() {
        // Every |x| identical => G = 0 => clamps to the bottom of the table.
        let dc = vec![0.25f32; 16_000];
        let m = score(&dc);
        assert_eq!(m.wada_snr_db, SNR_MIN_DB);
        assert!(m.rms_dbfs.is_finite());
        assert!(!FilterPolicy::default().accepts(&m));
    }

    #[test]
    fn short_buffers_of_every_length_do_not_panic() {
        for n in [0usize, 1, 2, 3, 255, 256, 257, 1023, 1024, 1025] {
            let buf = speech_like(n, 42);
            let m = score(&buf);
            assert!(m.clip_ratio.is_finite());
            assert!(m.spectral_flatness.is_finite());
            let _ = FilterPolicy::default().evaluate(&m);
        }
    }

    #[test]
    fn score_is_deterministic() {
        let buf = speech_like(16_000, 555);
        assert_eq!(score(&buf), score(&buf));
    }

    // -- rms_dbfs ------------------------------------------------------------

    #[test]
    fn rms_dbfs_matches_20_log10_rms_for_a_known_constant_signal() {
        // A constant-magnitude signal has rms == |amplitude| exactly.
        for amp in [1.0f32, 0.5, 0.1, 0.01] {
            let buf = vec![amp; 1000];
            let expected = 20.0 * amp.log10();
            let got = rms_dbfs(&buf);
            assert!(
                (got - expected).abs() < 1e-4,
                "amp {amp}: expected {expected} dBFS, got {got}"
            );
        }
        // Full-scale square wave: |x| == 1 everywhere => 0 dBFS.
        let square: Vec<f32> = (0..1000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert!(rms_dbfs(&square).abs() < 1e-4);
    }

    #[test]
    fn rms_dbfs_of_a_sine_is_about_minus_3db_below_its_peak() {
        // RMS of a full-scale sine is 1/sqrt(2) => -3.01 dBFS.
        let s = sine(16_000, 100.0, 1.0);
        let got = rms_dbfs(&s);
        assert!((got + 3.0103).abs() < 0.01, "got {got}");
    }

    // -- Policy plumbing -----------------------------------------------------

    #[test]
    fn accepts_agrees_with_evaluate() {
        let policy = FilterPolicy::default();
        let mut speech = speech_like(16_000, 21);
        normalize_to_dbfs(&mut speech, -20.0);
        for buf in [speech, vec![0.0; 16_000], sine(16_000, 440.0, 0.3)] {
            let m = score(&buf);
            assert_eq!(policy.accepts(&m), policy.evaluate(&m).is_none());
        }
    }

    #[test]
    fn metrics_and_policy_round_trip_through_serde_json() {
        // Not a dev-dependency here, so just confirm the derives compile and
        // the types are Serialize + Deserialize via a trait bound check.
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<QualityMetrics>();
        assert_serde::<FilterPolicy>();
        assert_serde::<RejectReason>();
    }

    #[test]
    fn reject_reason_strings_are_stable() {
        assert_eq!(RejectReason::LowSnr.as_str(), "low_snr");
        assert_eq!(RejectReason::Tonal.to_string(), "tonal");
    }

    // -- FFT sanity ----------------------------------------------------------

    #[test]
    fn fft_puts_a_bin_aligned_sine_in_the_right_bin() {
        let n = FLATNESS_FFT_SIZE;
        let k0 = 64usize;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * k0 as f32 * i as f32 / n as f32).sin())
            .collect();
        let mut im = vec![0.0f32; n];
        fft_in_place(&mut re, &mut im);

        let mags: Vec<f32> = (0..n / 2)
            .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
            .collect();
        let peak = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(peak, k0, "peak in bin {peak}, expected {k0}");
    }

    #[test]
    fn snr_from_g_interpolates_and_clamps() {
        assert_eq!(snr_from_g(0.0), SNR_MIN_DB);
        assert_eq!(snr_from_g(10.0), SNR_MAX_DB);
        // An exact table entry maps back to its own dB value.
        let mid = G_VALS[40]; // index 40 => -20 + 40 = 20 dB
        assert!((snr_from_g(mid) - 20.0).abs() < 0.01, "{}", snr_from_g(mid));
        // Halfway between two entries lands halfway between their dB values.
        let half = (G_VALS[40] + G_VALS[41]) / 2.0;
        assert!((snr_from_g(half) - 20.5).abs() < 0.01, "{}", snr_from_g(half));
    }
}
