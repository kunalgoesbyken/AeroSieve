use aerosieve_fixtures::Rng;

use crate::corpus::{Utterance, mean_power};

const BABBLE_TALKERS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseKind {
    White,
    Pink,
    Babble,
}

impl NoiseKind {
    pub const ALL: [Self; 3] = [Self::White, Self::Pink, Self::Babble];

    pub fn label(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Pink => "pink",
            Self::Babble => "babble",
        }
    }

    /// `len` samples of this noise. `index` picks which other utterances
    /// make up babble, so the target talker is never in its own background.
    pub fn generate(self, len: usize, seed: u64, corpus: &[Utterance], index: usize) -> Vec<f32> {
        match self {
            Self::White => white(len, seed),
            Self::Pink => pink(len, seed),
            Self::Babble => babble(len, corpus, index),
        }
    }
}

fn white(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..len).map(|_| rng.normal()).collect()
}

/// Paul Kellet's refined pink-noise filter over white noise.
fn pink(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut b = [0f32; 7];
    (0..len)
        .map(|_| {
            let w = rng.normal();
            b[0] = 0.99886 * b[0] + w * 0.055_517_9;
            b[1] = 0.99332 * b[1] + w * 0.075_075_9;
            b[2] = 0.96900 * b[2] + w * 0.153_852;
            b[3] = 0.86650 * b[3] + w * 0.310_485_6;
            b[4] = 0.55000 * b[4] + w * 0.532_952_2;
            b[5] = -0.7616 * b[5] - w * 0.016_898;
            let sample = b.iter().sum::<f32>() + w * 0.5362;
            b[6] = w * 0.115_926;
            sample
        })
        .collect()
}

fn babble(len: usize, corpus: &[Utterance], index: usize) -> Vec<f32> {
    let others = corpus.len().saturating_sub(1);
    if others == 0 {
        return vec![0.0; len];
    }
    let mut mix = vec![0f32; len];
    for talker in 0..BABBLE_TALKERS.min(others) {
        let source = &corpus[(index + 1 + talker) % corpus.len()].samples;
        let offset = (talker * source.len()) / BABBLE_TALKERS;
        for (position, sample) in mix.iter_mut().enumerate() {
            *sample += source[(offset + position) % source.len()];
        }
    }
    mix
}

/// `clean` plus `noise`, scaled so that their power ratio is `snr_db`.
pub fn mix_at_snr(clean: &[f32], noise: &[f32], snr_db: f32) -> Vec<f32> {
    let noise_power = mean_power(noise);
    if noise_power == 0.0 {
        return clean.to_vec();
    }
    let target = mean_power(clean) / 10f64.powf(f64::from(snr_db) / 10.0);
    let scale = (target / noise_power).sqrt() as f32;
    clean
        .iter()
        .zip(noise)
        .map(|(s, n)| s + scale * n)
        .collect()
}
