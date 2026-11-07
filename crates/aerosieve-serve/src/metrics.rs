use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const LATENCY_BUCKETS_S: [f64; 10] = [
    0.000_1, 0.000_25, 0.000_5, 0.001, 0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1,
];

/// Process-wide counters, rendered in the Prometheus text format.
#[derive(Default)]
pub struct Metrics {
    active: AtomicU64,
    sessions: AtomicU64,
    refused: AtomicU64,
    audio_ms: AtomicU64,
    verdicts: Mutex<BTreeMap<&'static str, u64>>,
    decision: Histogram,
}

#[derive(Default)]
struct Histogram {
    buckets: [AtomicU64; LATENCY_BUCKETS_S.len()],
    count: AtomicU64,
    sum_us: AtomicU64,
}

/// Counts a session as active until dropped.
pub struct ActiveSession<'a>(&'a Metrics);

impl Drop for ActiveSession<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Metrics {
    pub fn session_started(&self) -> ActiveSession<'_> {
        self.sessions.fetch_add(1, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
        ActiveSession(self)
    }

    pub fn session_refused(&self) {
        self.refused.fetch_add(1, Ordering::Relaxed);
    }

    pub fn audio_received(&self, seconds: f32) {
        self.audio_ms
            .fetch_add((seconds * 1000.0).round() as u64, Ordering::Relaxed);
    }

    pub fn verdict(&self, verdict: &'static str) {
        let mut verdicts = self
            .verdicts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *verdicts.entry(verdict).or_default() += 1;
    }

    /// Time from receiving the frame that completed a clip to sending its
    /// verdict.
    pub fn decision(&self, elapsed: Duration) {
        let seconds = elapsed.as_secs_f64();
        let histogram = &self.decision;
        for (bound, bucket) in LATENCY_BUCKETS_S.iter().zip(&histogram.buckets) {
            if seconds <= *bound {
                bucket.fetch_add(1, Ordering::Relaxed);
            }
        }
        histogram.count.fetch_add(1, Ordering::Relaxed);
        histogram
            .sum_us
            .fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        gauge(
            &mut out,
            "aerosieve_sessions_active",
            "Open streaming sessions.",
            load(&self.active),
        );
        counter(
            &mut out,
            "aerosieve_sessions_total",
            "Sessions accepted.",
            load(&self.sessions),
        );
        counter(
            &mut out,
            "aerosieve_sessions_refused_total",
            "Sessions refused at capacity.",
            load(&self.refused),
        );
        let _ = writeln!(out, "# HELP aerosieve_audio_seconds_total Audio received.");
        let _ = writeln!(out, "# TYPE aerosieve_audio_seconds_total counter");
        let _ = writeln!(
            out,
            "aerosieve_audio_seconds_total {}",
            load(&self.audio_ms) as f64 / 1000.0
        );

        let _ = writeln!(
            out,
            "# HELP aerosieve_clips_total Clips judged, by verdict."
        );
        let _ = writeln!(out, "# TYPE aerosieve_clips_total counter");
        let verdicts = self
            .verdicts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for (verdict, count) in verdicts.iter() {
            let _ = writeln!(
                out,
                "aerosieve_clips_total{{verdict=\"{verdict}\"}} {count}"
            );
        }

        let histogram = &self.decision;
        let _ = writeln!(
            out,
            "# HELP aerosieve_decision_seconds Frame received to verdict sent."
        );
        let _ = writeln!(out, "# TYPE aerosieve_decision_seconds histogram");
        for (bound, bucket) in LATENCY_BUCKETS_S.iter().zip(&histogram.buckets) {
            let _ = writeln!(
                out,
                "aerosieve_decision_seconds_bucket{{le=\"{bound}\"}} {}",
                load(bucket)
            );
        }
        let count = load(&histogram.count);
        let _ = writeln!(
            out,
            "aerosieve_decision_seconds_bucket{{le=\"+Inf\"}} {count}"
        );
        let _ = writeln!(
            out,
            "aerosieve_decision_seconds_sum {}",
            load(&histogram.sum_us) as f64 / 1e6
        );
        let _ = writeln!(out, "aerosieve_decision_seconds_count {count}");
        out
    }
}

fn gauge(out: &mut String, name: &str, help: &str, value: u64) {
    sample(out, name, help, "gauge", value);
}

fn counter(out: &mut String, name: &str, help: &str, value: u64) {
    sample(out, name, help, "counter", value);
}

fn sample(out: &mut String, name: &str, help: &str, kind: &str, value: u64) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}"
    );
}
