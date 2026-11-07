use std::collections::BTreeMap;

use aerosieve::FilterPolicy;
use serde::Serialize;

use crate::condition::{Condition, Expect, Family};
use crate::evaluate::{Observation, Verdict};
use crate::noise::NoiseKind;

#[derive(Serialize)]
pub struct Report {
    pub corpus: String,
    pub source: String,
    pub utterances: usize,
    pub hours: f64,
    pub elapsed_s: f64,
    pub policy: FilterPolicy,
    pub overall: Scores,
    pub families: Vec<FamilyReport>,
    pub conditions: Vec<ConditionReport>,
    pub snr_estimates: Vec<SnrEstimate>,
}

#[derive(Serialize, Default)]
pub struct Scores {
    /// Recordings in which the segmenter found no clip. They never reach the
    /// filters, so they are counted here and nowhere else.
    pub unsegmented: usize,
    pub should_reject: usize,
    pub caught: usize,
    pub should_keep: usize,
    pub falsely_rejected: usize,
    pub recall: f64,
    pub precision: f64,
    pub false_reject_rate: f64,
}

#[derive(Serialize)]
pub struct FamilyReport {
    pub family: Family,
    #[serde(flatten)]
    pub scores: Scores,
    pub right_reason: usize,
}

#[derive(Serialize)]
pub struct ConditionReport {
    pub id: String,
    pub label: String,
    pub family: Family,
    pub clips: usize,
    pub kept: usize,
    pub no_clip: usize,
    pub rejected: BTreeMap<&'static str, usize>,
    pub expected: BTreeMap<&'static str, usize>,
}

#[derive(Serialize)]
pub struct SnrEstimate {
    pub noise: NoiseKind,
    pub injected_snr_db: f32,
    pub median_estimate_db: f32,
    pub p25_estimate_db: f32,
    pub p75_estimate_db: f32,
    pub kept_rate: f64,
}

pub struct RunInfo {
    pub corpus: String,
    pub source: String,
    pub utterances: usize,
    pub hours: f64,
    pub elapsed_s: f64,
    pub policy: FilterPolicy,
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

fn score<'a>(observations: impl Iterator<Item = &'a Observation> + Clone) -> Scores {
    let count = |expect: Expect, rejected: bool| {
        observations
            .clone()
            .filter(|o| o.expect == expect && o.verdict != Verdict::NoClip)
            .filter(|o| matches!(o.verdict, Verdict::Rejected(_)) == rejected)
            .count()
    };
    let caught = count(Expect::Reject, true);
    let missed = count(Expect::Reject, false);
    let falsely_rejected = count(Expect::Keep, true);
    let kept_right = count(Expect::Keep, false);
    Scores {
        unsegmented: observations
            .clone()
            .filter(|o| o.verdict == Verdict::NoClip)
            .count(),
        should_reject: caught + missed,
        caught,
        should_keep: falsely_rejected + kept_right,
        falsely_rejected,
        recall: ratio(caught, caught + missed),
        precision: ratio(caught, caught + falsely_rejected),
        false_reject_rate: ratio(falsely_rejected, falsely_rejected + kept_right),
    }
}

fn expect_name(expect: Expect) -> &'static str {
    match expect {
        Expect::Keep => "keep",
        Expect::Reject => "reject",
        Expect::Borderline => "borderline",
    }
}

fn condition_report(condition: &Condition, observations: &[&Observation]) -> ConditionReport {
    let mut rejected = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for observation in observations {
        *expected.entry(expect_name(observation.expect)).or_default() += 1;
        if let Verdict::Rejected(reason) = observation.verdict {
            *rejected.entry(reason.as_str()).or_default() += 1;
        }
    }
    ConditionReport {
        id: condition.id(),
        label: condition.label(),
        family: condition.family(),
        clips: observations
            .iter()
            .filter(|o| o.verdict != Verdict::NoClip)
            .count(),
        kept: observations
            .iter()
            .filter(|o| o.verdict == Verdict::Kept)
            .count(),
        no_clip: observations
            .iter()
            .filter(|o| o.verdict == Verdict::NoClip)
            .count(),
        rejected,
        expected,
    }
}

fn quantile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NAN;
    }
    sorted[((sorted.len() - 1) as f32 * q).round() as usize]
}

fn snr_estimate(kind: NoiseKind, snr_db: f32, observations: &[&Observation]) -> SnrEstimate {
    let mut estimates: Vec<f32> = observations
        .iter()
        .filter_map(|o| o.wada_snr_db)
        .filter(|v| v.is_finite())
        .collect();
    estimates.sort_by(f32::total_cmp);
    let kept = observations
        .iter()
        .filter(|o| o.verdict == Verdict::Kept)
        .count();
    SnrEstimate {
        noise: kind,
        injected_snr_db: snr_db,
        median_estimate_db: quantile(&estimates, 0.5),
        p25_estimate_db: quantile(&estimates, 0.25),
        p75_estimate_db: quantile(&estimates, 0.75),
        kept_rate: ratio(kept, observations.len()),
    }
}

fn family_report(
    family: Family,
    observations: &[Observation],
    conditions: &[Condition],
) -> FamilyReport {
    let members = observations
        .iter()
        .filter(move |o| conditions[o.condition].family() == family);
    let right_reason = members
        .clone()
        .filter(|o| o.expect == Expect::Reject)
        .filter(|o| match (o.verdict, family.reason()) {
            (Verdict::Rejected(reason), Some(expected)) => reason == expected,
            _ => false,
        })
        .count();
    FamilyReport {
        family,
        scores: score(members),
        right_reason,
    }
}

pub fn build(info: RunInfo, conditions: &[Condition], observations: &[Observation]) -> Report {
    let by_condition: Vec<Vec<&Observation>> = (0..conditions.len())
        .map(|index| {
            observations
                .iter()
                .filter(|o| o.condition == index)
                .collect()
        })
        .collect();
    let mut families: Vec<Family> = conditions.iter().map(Condition::family).collect();
    families.sort();
    families.dedup();

    Report {
        corpus: info.corpus,
        source: info.source,
        utterances: info.utterances,
        hours: info.hours,
        elapsed_s: info.elapsed_s,
        policy: info.policy,
        overall: score(observations.iter()),
        families: families
            .into_iter()
            .map(|family| family_report(family, observations, conditions))
            .collect(),
        conditions: conditions
            .iter()
            .zip(&by_condition)
            .map(|(condition, observed)| condition_report(condition, observed))
            .collect(),
        snr_estimates: conditions
            .iter()
            .zip(&by_condition)
            .filter_map(|(condition, observed)| {
                condition
                    .noise()
                    .map(|(kind, snr)| snr_estimate(kind, snr, observed))
            })
            .collect(),
    }
}
