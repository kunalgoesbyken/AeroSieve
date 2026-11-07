use std::f32::consts::TAU;

use aerosieve::RejectReason;
use aerosieve::decode::{DecodeError, resample_mono};

use crate::corpus::{REFERENCE_DBFS, SAMPLE_RATE, Utterance, at_level};
use crate::noise::{NoiseKind, mix_at_snr};

const NOISE_SNRS_DB: [f32; 6] = [30.0, 20.0, 15.0, 10.0, 5.0, 0.0];
const OVERDRIVE_GAINS: [f32; 4] = [2.0, 4.0, 8.0, 16.0];
/// Quiet enough to fail the level floor even after leveling adds 3 dB, loud
/// enough that the segmenter still cuts clips for the filters to judge.
const QUIET_DBFS: f32 = -46.0;
const PHONE_RATE: u32 = 8_000;
const MU: f32 = 255.0;

/// At or above this SNR, speech is clean enough to train on.
const KEEP_SNR_DB: f32 = 20.0;
/// At or below this SNR, speech is too noisy to train on.
const REJECT_SNR_DB: f32 = 5.0;
/// Below this fraction of clipped samples, clipping is inaudible.
const KEEP_CLIPPED: f32 = 0.005;
/// Above this fraction of clipped samples, the clip is damaged.
const REJECT_CLIPPED: f32 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Expect {
    Keep,
    Reject,
    Borderline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Control,
    Noise,
    Clipping,
    Level,
    Tone,
}

impl Family {
    /// The rejection that names this defect correctly.
    pub fn reason(self) -> Option<RejectReason> {
        match self {
            Self::Control => None,
            Self::Noise => Some(RejectReason::LowSnr),
            Self::Clipping => Some(RejectReason::Clipping),
            Self::Level => Some(RejectReason::TooQuiet),
            Self::Tone => Some(RejectReason::Tonal),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToneKind {
    DialTone,
    TestTone,
    Hum,
}

impl ToneKind {
    fn id(self) -> &'static str {
        match self {
            Self::DialTone => "dial_tone",
            Self::TestTone => "test_tone",
            Self::Hum => "hum",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::DialTone => "dial tone",
            Self::TestTone => "1 kHz test tone",
            Self::Hum => "50 Hz hum",
        }
    }

    fn partials(self) -> Vec<(f32, f32)> {
        match self {
            Self::DialTone => vec![(350.0, 1.0), (440.0, 1.0)],
            Self::TestTone => vec![(1000.0, 1.0)],
            Self::Hum => (1..=20)
                .map(|n| (50.0 * n as f32, 1.0 / n as f32))
                .collect(),
        }
    }

    fn synthesize(self, len: usize) -> Vec<f32> {
        let partials = self.partials();
        let raw = (0..len)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                partials
                    .iter()
                    .map(|(freq, amp)| amp * (TAU * freq * t).sin())
                    .sum()
            })
            .collect();
        at_level(raw, REFERENCE_DBFS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Condition {
    Clean,
    PhoneLine,
    Noise { kind: NoiseKind, snr_db: f32 },
    Overdrive { gain: f32 },
    Quiet,
    Tone(ToneKind),
}

/// A damaged copy of an utterance and what a good filter should do with it.
pub struct Degraded {
    pub samples: Vec<f32>,
    pub expect: Expect,
}

impl Condition {
    pub fn all() -> Vec<Self> {
        let noise = NoiseKind::ALL.into_iter().flat_map(|kind| {
            NOISE_SNRS_DB
                .into_iter()
                .map(move |snr_db| Self::Noise { kind, snr_db })
        });
        let overdrive = OVERDRIVE_GAINS
            .into_iter()
            .map(|gain| Self::Overdrive { gain });
        let tones = [ToneKind::DialTone, ToneKind::TestTone, ToneKind::Hum].map(Self::Tone);
        [Self::Clean, Self::PhoneLine]
            .into_iter()
            .chain(noise)
            .chain(overdrive)
            .chain([Self::Quiet])
            .chain(tones)
            .collect()
    }

    /// Stable identifier for reports and tooling.
    pub fn id(&self) -> String {
        match self {
            Self::Clean => "clean".into(),
            Self::PhoneLine => "phone_line".into(),
            Self::Noise { kind, snr_db } => format!("{}_noise_{snr_db}db", kind.label()),
            Self::Overdrive { gain } => format!("overdrive_x{gain}"),
            Self::Quiet => "quiet".into(),
            Self::Tone(kind) => kind.id().into(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Clean => "clean".into(),
            Self::PhoneLine => "phone line (8 kHz μ-law)".into(),
            Self::Noise { kind, snr_db } => format!("{} noise at {snr_db} dB", kind.label()),
            Self::Overdrive { gain } => format!("overdriven ×{gain}"),
            Self::Quiet => format!("quiet ({QUIET_DBFS} dBFS)"),
            Self::Tone(kind) => kind.label().into(),
        }
    }

    pub fn family(&self) -> Family {
        match self {
            Self::Clean | Self::PhoneLine => Family::Control,
            Self::Noise { .. } => Family::Noise,
            Self::Overdrive { .. } => Family::Clipping,
            Self::Quiet => Family::Level,
            Self::Tone(_) => Family::Tone,
        }
    }

    pub fn noise(&self) -> Option<(NoiseKind, f32)> {
        match *self {
            Self::Noise { kind, snr_db } => Some((kind, snr_db)),
            _ => None,
        }
    }

    pub fn apply(&self, corpus: &[Utterance], index: usize) -> Result<Degraded, DecodeError> {
        let clean = &corpus[index].samples;
        let seed = 0x5EED_0000 + index as u64;
        Ok(match *self {
            Self::Clean => keep(clean.clone()),
            Self::PhoneLine => keep(phone_line(clean)?),
            Self::Noise { kind, snr_db } => {
                let noise = kind.generate(clean.len(), seed, corpus, index);
                Degraded {
                    samples: mix_at_snr(clean, &noise, snr_db),
                    expect: expect_for_snr(snr_db),
                }
            }
            Self::Overdrive { gain } => overdrive(clean, gain),
            Self::Quiet => Degraded {
                samples: at_level(clean.clone(), QUIET_DBFS),
                expect: Expect::Reject,
            },
            Self::Tone(kind) => Degraded {
                samples: kind.synthesize(clean.len()),
                expect: Expect::Reject,
            },
        })
    }
}

fn keep(samples: Vec<f32>) -> Degraded {
    Degraded {
        samples,
        expect: Expect::Keep,
    }
}

fn expect_for_snr(snr_db: f32) -> Expect {
    if snr_db >= KEEP_SNR_DB {
        Expect::Keep
    } else if snr_db <= REJECT_SNR_DB {
        Expect::Reject
    } else {
        Expect::Borderline
    }
}

fn overdrive(clean: &[f32], gain: f32) -> Degraded {
    let samples: Vec<f32> = clean.iter().map(|s| (s * gain).clamp(-1.0, 1.0)).collect();
    let clipped = samples.iter().filter(|s| s.abs() >= 1.0).count() as f32 / samples.len() as f32;
    let expect = if clipped > REJECT_CLIPPED {
        Expect::Reject
    } else if clipped < KEEP_CLIPPED {
        Expect::Keep
    } else {
        Expect::Borderline
    };
    Degraded { samples, expect }
}

fn phone_line(clean: &[f32]) -> Result<Vec<f32>, DecodeError> {
    let narrow = resample_mono(clean.to_vec(), SAMPLE_RATE, PHONE_RATE)?;
    let companded = narrow.into_iter().map(mu_law_round_trip).collect();
    resample_mono(companded, PHONE_RATE, SAMPLE_RATE)
}

fn mu_law_round_trip(sample: f32) -> f32 {
    let x = sample.clamp(-1.0, 1.0);
    let encoded = x.signum() * (1.0 + MU * x.abs()).ln() / (1.0 + MU).ln();
    let quantized = (encoded * 127.0).round() / 127.0;
    quantized.signum() * ((1.0 + MU).powf(quantized.abs()) - 1.0) / MU
}
