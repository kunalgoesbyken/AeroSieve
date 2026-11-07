use aerosieve::RejectReason;
use aerosieve_fixtures::{SAMPLE_RATE, glottal_speech, samples, tone};

use crate::condition::{Condition, Expect, Family};
use crate::corpus::{Utterance, at_level, mean_power};
use crate::evaluate::{Observation, Verdict};
use crate::noise::{NoiseKind, mix_at_snr};
use crate::report::{RunInfo, build};

fn corpus() -> Vec<Utterance> {
    (0..3)
        .map(|index| Utterance {
            name: format!("u{index}"),
            samples: at_level(glottal_speech(samples(4.0 + index as f32)), -20.0),
        })
        .collect()
}

fn snr_db(clean: &[f32], mixed: &[f32]) -> f64 {
    let noise: Vec<f32> = mixed.iter().zip(clean).map(|(m, c)| m - c).collect();
    10.0 * (mean_power(clean) / mean_power(&noise)).log10()
}

#[test]
fn every_noise_kind_is_mixed_at_the_requested_snr() {
    let corpus = corpus();
    let clean = &corpus[0].samples;
    for kind in NoiseKind::ALL {
        let noise = kind.generate(clean.len(), 7, &corpus, 0);
        let mixed = mix_at_snr(clean, &noise, 10.0);
        assert!((snr_db(clean, &mixed) - 10.0).abs() < 0.01, "{kind:?}");
    }
}

#[test]
fn babble_never_contains_the_target_talker() {
    let corpus = corpus();
    let babble = NoiseKind::Babble.generate(100, 0, &corpus[..1], 0);
    assert!(babble.iter().all(|&s| s == 0.0));
}

#[test]
fn noise_expectations_follow_the_injected_snr() {
    let corpus = corpus();
    let expect = |snr_db| {
        Condition::Noise {
            kind: NoiseKind::White,
            snr_db,
        }
        .apply(&corpus, 0)
        .unwrap()
        .expect
    };
    assert_eq!(expect(20.0), Expect::Keep);
    assert_eq!(expect(10.0), Expect::Borderline);
    assert_eq!(expect(5.0), Expect::Reject);
}

#[test]
fn overdrive_expectation_follows_the_clipped_fraction() {
    let corpus = corpus();
    let expect = |gain| {
        Condition::Overdrive { gain }
            .apply(&corpus, 0)
            .unwrap()
            .expect
    };
    assert_eq!(expect(1.0), Expect::Keep);
    assert_eq!(expect(16.0), Expect::Reject);
}

#[test]
fn a_phone_line_preserves_in_band_audio() {
    let in_band = Utterance {
        name: "tone".into(),
        samples: at_level(tone(SAMPLE_RATE, 1000.0, 1.0, samples(2.0)), -20.0),
    };
    let corpus = [in_band];
    let clean = &corpus[0].samples;
    let phone = Condition::PhoneLine.apply(&corpus, 0).unwrap().samples;
    assert_eq!(phone.len(), clean.len());
    let settled = 1600..clean.len() - 1600;
    assert!(snr_db(&clean[settled.clone()], &phone[settled]) > 30.0);
}

#[test]
fn every_condition_has_a_unique_id_and_label() {
    for name in [Condition::id, Condition::label] {
        let mut names: Vec<String> = Condition::all().iter().map(name).collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}

#[test]
fn scores_count_misses_false_rejects_and_borderline_correctly() {
    let conditions = [Condition::Clean, Condition::Quiet];
    let observation = |condition, expect, verdict| Observation {
        condition,
        expect,
        verdict,
        wada_snr_db: None,
    };
    let observations = [
        observation(0, Expect::Keep, Verdict::Kept),
        observation(0, Expect::Keep, Verdict::Rejected(RejectReason::LowSnr)),
        observation(
            0,
            Expect::Borderline,
            Verdict::Rejected(RejectReason::LowSnr),
        ),
        observation(1, Expect::Reject, Verdict::NoClip),
        observation(1, Expect::Reject, Verdict::Kept),
    ];
    let info = RunInfo {
        corpus: "test".into(),
        source: "synthetic".into(),
        utterances: 1,
        hours: 0.0,
        elapsed_s: 0.0,
        policy: Default::default(),
    };

    let report = build(info, &conditions, &observations);

    assert_eq!(report.overall.should_reject, 2);
    assert_eq!(report.overall.caught, 1);
    assert_eq!(report.overall.falsely_rejected, 1);
    assert_eq!(report.overall.recall, 0.5);
    assert_eq!(report.overall.false_reject_rate, 0.5);
    let level = report
        .families
        .iter()
        .find(|f| f.family == Family::Level)
        .unwrap();
    assert_eq!(level.right_reason, 1);
}
