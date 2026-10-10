//! Deterministic synthetic audio for tests, benches, and examples.
//!
//! Every signal is a pure function of its arguments, so a failure reproduces
//! on any machine. All generators except [`tone`] produce 16 kHz mono.

use std::f32::consts::TAU;

pub const SAMPLE_RATE: u32 = 16_000;

/// Number of samples in `seconds` of audio at [`SAMPLE_RATE`].
pub fn samples(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round() as usize
}

/// xorshift64* generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
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
    pub fn uniform(&mut self) -> f32 {
        (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar(&mut self) -> f32 {
        self.uniform() * 2.0 - 1.0
    }

    /// Standard normal, by Box-Muller.
    pub fn normal(&mut self) -> f32 {
        let radius = (-2.0 * self.uniform().max(1e-9).ln()).sqrt();
        radius * (TAU * self.uniform()).cos()
    }
}

pub fn silence(len: usize) -> Vec<f32> {
    vec![0.0; len]
}

pub fn tone(sample_rate: u32, freq_hz: f32, amplitude: f32, len: usize) -> Vec<f32> {
    let step = TAU * freq_hz / sample_rate as f32;
    (0..len)
        .map(|i| amplitude * (step * i as f32).sin())
        .collect()
}

/// Gaussian noise with standard deviation `amplitude`.
pub fn white_noise(len: usize, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..len).map(|_| amplitude * rng.normal()).collect()
}

/// A glottal pulse train with aspiration noise under a syllabic envelope.
///
/// Peaky in time and broadband in frequency, like real speech. WADA-SNR
/// models clean speech as Gamma(0.4), so a sum of steady harmonics scores as
/// noise and is rejected as tonal; this passes the default filter policy.
pub fn glottal_speech(len: usize) -> Vec<f32> {
    const PITCH_HZ: f32 = 120.0;
    const SYLLABLE_HZ: f32 = 3.5;

    let period = (SAMPLE_RATE as f32 / PITCH_HZ) as usize;
    let mut rng = Rng::new(0x5_DEEC_E66D);
    (0..len)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let envelope = 0.35 + 0.65 * (TAU * SYLLABLE_HZ * t).sin().abs();
            let phase = (i % period) as f32 / period as f32;
            let pulse = (-6.0 * phase).exp() * (TAU * phase).sin();
            (pulse * 0.8 + rng.bipolar() * 0.04) * envelope * 0.8
        })
        .collect()
}

/// Voiced harmonics with a wandering pitch and an envelope that closes fully
/// between syllables, plus faint breath noise.
///
/// Nearly noiseless, so it serves as the clean reference when mixing to a
/// known SNR.
pub fn harmonic_speech(len: usize, seed: u64) -> Vec<f32> {
    const HARMONIC_GAINS: [f32; 5] = [1.0, 0.55, 0.32, 0.20, 0.12];

    let mut rng = Rng::new(seed);
    (0..len)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let pitch_hz = 130.0 + 12.0 * (TAU * 1.7 * t).sin();
            let phase = TAU * pitch_hz * t;
            let voiced: f32 = HARMONIC_GAINS
                .iter()
                .zip(1u8..)
                .map(|(gain, harmonic)| gain * (f32::from(harmonic) * phase).sin())
                .sum();
            let envelope = (0.5 - 0.5 * (TAU * 4.0 * t).cos()).powi(2);
            0.18 * envelope * voiced + 0.01 * rng.normal() * envelope.max(0.05)
        })
        .collect()
}

/// Six-second runs separated by half-second gaps, cycling through two runs
/// of usable speech, then one each of overdriven speech, a sustained tone, and
/// broadband noise.
///
/// A curation benchmark on uniformly clean audio reports 100% retention and
/// measures nothing; this plants a known share of each rejection reason.
pub fn mixed_corpus(len: usize) -> Vec<f32> {
    const NOISE_SEED: u64 = 0xC0_FFEE;

    let run = glottal_speech(samples(6.0));
    let gap = silence(samples(0.5));
    let kinds = [
        run.clone(),
        run.clone(),
        overdrive(&run, 6.0),
        tone(SAMPLE_RATE, 255.0, 0.5, run.len()),
        white_noise(run.len(), 0.1, NOISE_SEED),
    ];

    let mut corpus = Vec::with_capacity(len + gap.len() + run.len());
    for kind in kinds.iter().cycle() {
        if corpus.len() >= len {
            break;
        }
        corpus.extend_from_slice(&gap);
        corpus.extend_from_slice(kind);
    }
    corpus.truncate(len);
    corpus
}

/// Amplify by `gain` and hard-limit at full scale.
pub fn overdrive(samples: &[f32], gain: f32) -> Vec<f32> {
    samples
        .iter()
        .map(|s| (s * gain).clamp(-1.0, 1.0))
        .collect()
}

/// Add Gaussian noise to `clean` at the requested signal-to-noise ratio.
pub fn mix_at_snr(clean: &[f32], snr_db: f32, seed: u64) -> Vec<f32> {
    let noise = white_noise(clean.len(), 1.0, seed);
    let noise_power = mean_power(clean) / 10f64.powf(f64::from(snr_db) / 10.0);
    let scale = (noise_power / mean_power(&noise)).sqrt() as f32;
    clean
        .iter()
        .zip(&noise)
        .map(|(signal, noise)| signal + scale * noise)
        .collect()
}

/// Root-mean-square amplitude; `0.0` for an empty buffer.
pub fn rms(samples: &[f32]) -> f32 {
    mean_power(samples).sqrt() as f32
}

/// Rescale so the RMS level lands on `target_dbfs`. Silence is returned as is.
pub fn at_dbfs(mut samples: Vec<f32>, target_dbfs: f32) -> Vec<f32> {
    let current = rms(&samples);
    if current == 0.0 {
        return samples;
    }
    let gain = 10f32.powf(target_dbfs / 20.0) / current;
    samples.iter_mut().for_each(|s| *s *= gain);
    samples
}

fn mean_power(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    sum / samples.len() as f64
}
