use std::collections::BTreeMap;
use std::fmt;

use aerosieve::{FileOutcome, QualityMetrics, RejectReason};

const PERCENTILES: [u8; 3] = [5, 50, 95];

/// Totals and metric distributions across every clip inspected.
#[derive(Default)]
pub struct Summary {
    files: usize,
    failed_files: usize,
    accepted: usize,
    rejections: BTreeMap<RejectReason, usize>,
    metrics: Vec<QualityMetrics>,
}

impl Summary {
    pub fn add(&mut self, outcome: &FileOutcome) {
        self.files += 1;
        for clip in &outcome.segments {
            self.metrics.push(clip.metrics);
            match clip.rejected {
                Some(reason) => *self.rejections.entry(reason).or_default() += 1,
                None => self.accepted += 1,
            }
        }
    }

    pub fn add_failure(&mut self) {
        self.failed_files += 1;
    }

    fn write_distribution(
        &self,
        f: &mut fmt::Formatter<'_>,
        label: &str,
        metric: fn(&QualityMetrics) -> f32,
    ) -> fmt::Result {
        let mut values: Vec<f32> = self
            .metrics
            .iter()
            .map(metric)
            .filter(|value| value.is_finite())
            .collect();
        values.sort_by(f32::total_cmp);

        write!(f, "  {label:<10}")?;
        for percentile in PERCENTILES {
            match nearest_rank(&values, percentile) {
                Some(value) => write!(f, " {value:>8.3}")?,
                None => write!(f, " {:>8}", "-")?,
            }
        }
        writeln!(f)
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let clips = self.metrics.len();
        writeln!(
            f,
            "{} files inspected, {} skipped",
            self.files, self.failed_files
        )?;
        writeln!(f, "{} of {clips} clips accepted", self.accepted)?;
        for (reason, count) in &self.rejections {
            writeln!(f, "  rejected as {reason:<10} {count}")?;
        }
        if clips == 0 {
            return Ok(());
        }

        write!(f, "  {:<10}", "PERCENTILE")?;
        for percentile in PERCENTILES {
            write!(f, " {:>8}", format!("p{percentile}"))?;
        }
        writeln!(f)?;
        self.write_distribution(f, "SNR dB", |metrics| metrics.wada_snr_db)?;
        self.write_distribution(f, "dBFS", |metrics| metrics.rms_dbfs)?;
        self.write_distribution(f, "flatness", |metrics| metrics.spectral_flatness)
    }
}

/// Nearest-rank percentile of ascending `sorted`, or `None` if it is empty.
fn nearest_rank(sorted: &[f32], percentile: u8) -> Option<f32> {
    let rank = (usize::from(percentile) * sorted.len()).div_ceil(100);
    sorted.get(rank.saturating_sub(1)).copied()
}
