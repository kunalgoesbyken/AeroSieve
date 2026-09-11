//! Deterministic audio fixture generation, shared by integration tests.
//!
//! Every signal here is produced from a fixed seed so test failures are
//! reproducible. No `rand` dependency: a small xorshift PRNG is enough for
//! noise, and using one keeps fixtures identical across platforms and runs.

#![allow(dead_code)]

pub const SAMPLE_RATE: u32 = 16_000;

/// xorshift64*, seeded. Deterministic across platforms.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero seed is a fixed point for xorshift; steer away from it.
        Self(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform in [-1.0, 1.0).
    pub fn uniform(&mut self) -> f32 {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) as f32 * 2.0 - 1.0
    }

    /// Approximately standard normal, via the central limit theorem.
    pub fn gaussian(&mut self) -> f32 {
        let sum: f32 = (0..12).map(|_| (self.uniform() + 1.0) / 2.0).sum();
        sum - 6.0
    }
}

/// Digital silence.
pub fn silence(len: usize) -> Vec<f32> {
    vec![0.0; len]
}

/// A pure tone. Tonal, so spectral flatness should be low.
pub fn sine(len: usize, freq_hz: f32, amplitude: f32) -> Vec<f32> {
    let step = std::f32::consts::TAU * freq_hz / SAMPLE_RATE as f32;
    (0..len).map(|i| (i as f32 * step).sin() * amplitude).collect()
}

/// Speech-like: a few harmonics with an amplitude envelope, plus a little
/// noise. Not real speech, but it exercises the same code paths — broadband,
/// time-varying, not a pure tone.
pub fn speech_like(len: usize, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let f0 = 120.0;
    (0..len)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            // Syllable-rate envelope, never fully closing.
            let env = 0.5 + 0.5 * (std::f32::consts::TAU * 4.0 * t).sin().abs();
            let harmonics: f32 = [1.0, 2.0, 3.0, 4.5, 6.0]
                .iter()
                .enumerate()
                .map(|(k, mult)| {
                    let gain = 1.0 / (k as f32 + 1.0);
                    (std::f32::consts::TAU * f0 * mult * t).sin() * gain
                })
                .sum();
            (harmonics * 0.35 * env + rng.gaussian() * 0.01) * amplitude
        })
        .collect()
}

/// White noise at a given amplitude.
pub fn white_noise(len: usize, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..len).map(|_| rng.gaussian() * amplitude).collect()
}

/// Speech-like signal driven hard into the rails.
pub fn clipped(len: usize, seed: u64) -> Vec<f32> {
    speech_like(len, 1.0, seed)
        .into_iter()
        .map(|x| (x * 4.0).clamp(-1.0, 1.0))
        .collect()
}

/// Speech-like signal mixed with noise at an approximate target SNR.
pub fn speech_at_snr(len: usize, snr_db: f32, seed: u64) -> Vec<f32> {
    let speech = speech_like(len, 0.5, seed);
    let speech_power: f32 =
        speech.iter().map(|x| x * x).sum::<f32>() / speech.len().max(1) as f32;
    let noise_power = speech_power / 10f32.powf(snr_db / 10.0);
    let mut rng = Rng::new(seed ^ 0xDEAD_BEEF);
    speech
        .into_iter()
        .map(|s| s + rng.gaussian() * noise_power.sqrt())
        .collect()
}

/// Speech bursts separated by silence. Returns the signal and the expected
/// (start, end) sample bounds of each burst, for asserting on segmentation.
pub fn bursts(
    burst_lens: &[usize],
    gap_len: usize,
    seed: u64,
) -> (Vec<f32>, Vec<(usize, usize)>) {
    let mut out = Vec::new();
    let mut bounds = Vec::new();
    out.extend_from_slice(&silence(gap_len));
    for (i, &blen) in burst_lens.iter().enumerate() {
        let start = out.len();
        out.extend_from_slice(&speech_like(blen, 0.6, seed.wrapping_add(i as u64)));
        bounds.push((start, out.len()));
        out.extend_from_slice(&silence(gap_len));
    }
    (out, bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn rng_is_deterministic() {
        let a: Vec<f32> = (0..32).map(|_| Rng::new(42).uniform()).collect();
        let b: Vec<f32> = (0..32).map(|_| Rng::new(42).uniform()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn rng_stays_in_range() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            let v = rng.uniform();
            assert!((-1.0..1.0).contains(&v), "uniform out of range: {v}");
        }
    }

    #[test]
    fn silence_is_silent() {
        assert_eq!(rms(&silence(1000)), 0.0);
    }

    #[test]
    fn speech_like_is_audible_and_bounded() {
        let s = speech_like(SAMPLE_RATE as usize, 0.5, 1);
        assert!(rms(&s) > 0.01, "speech fixture too quiet: {}", rms(&s));
        assert!(s.iter().all(|x| x.abs() <= 1.0), "speech fixture clipped");
    }

    #[test]
    fn clipped_fixture_actually_clips() {
        let c = clipped(SAMPLE_RATE as usize, 2);
        let at_rail = c.iter().filter(|x| x.abs() >= 0.999).count();
        assert!(
            at_rail > c.len() / 100,
            "expected substantial clipping, got {at_rail} of {}",
            c.len()
        );
    }

    #[test]
    fn snr_fixture_tracks_requested_ratio() {
        // Higher requested SNR must yield a measurably cleaner signal than
        // lower, which is what the acoustic tests rely on.
        let clean = speech_at_snr(SAMPLE_RATE as usize, 30.0, 3);
        let noisy = speech_at_snr(SAMPLE_RATE as usize, 0.0, 3);
        assert!(rms(&noisy) > rms(&clean));
    }

    #[test]
    fn bursts_have_expected_shape() {
        let (sig, bounds) = bursts(&[8000, 8000], 4000, 5);
        assert_eq!(bounds.len(), 2);
        assert_eq!(sig.len(), 4000 * 3 + 8000 * 2);
        for (start, end) in bounds {
            assert!(rms(&sig[start..end]) > 0.01, "burst should be audible");
        }
    }
}
