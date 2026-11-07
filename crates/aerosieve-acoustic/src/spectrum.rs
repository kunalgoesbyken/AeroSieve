//! Spectral flatness over fixed-size Hann-windowed frames.

use std::f32::consts::TAU;
use std::ops::RangeInclusive;
use std::sync::{Arc, OnceLock};

use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};

/// Samples per analysis frame.
pub const FLATNESS_FFT_SIZE: usize = 1024;

/// The band flatness is measured over: the telephone speech band, which is
/// present in every source from narrowband telephony upwards.
///
/// Measuring the whole spectrum instead would penalize resampled audio. A
/// resampler's low-pass leaves the bins near Nyquist empty, and upsampled
/// narrowband audio has nothing in its upper half; either drags the geometric
/// mean down until speech reads as a tone.
pub const FLATNESS_BAND_HZ: RangeInclusive<f32> = 300.0..=3400.0;

/// Added to every power bin so an exactly-zero bin cannot drive the geometric
/// mean to zero.
const POWER_FLOOR: f64 = 1e-20;

/// Spectral flatness (Wiener entropy) within [`FLATNESS_BAND_HZ`]: the
/// geometric mean of the power spectrum over its arithmetic mean, in `[0, 1]`.
///
/// Near 1.0 is noise-like, near 0.0 is tonal. The buffer is cut into
/// non-overlapping [`FLATNESS_FFT_SIZE`] frames and the per-frame values are
/// averaged; a trailing partial frame is ignored.
///
/// Returns `0.0` for a buffer shorter than one frame, or when `sample_rate`
/// is too low for the band to cover any bin.
pub fn spectral_flatness(samples: &[f32], sample_rate: u32) -> f32 {
    let band = band_bins(sample_rate);
    if band.is_empty() {
        return 0.0;
    }

    let plan = forward_plan();
    let mut frame = plan.make_input_vec();
    let mut spectrum = plan.make_output_vec();
    let mut scratch = plan.make_scratch_vec();
    let mut total = 0.0f64;
    let mut frames = 0u32;

    let (frames_of_samples, _partial) = samples.as_chunks::<FLATNESS_FFT_SIZE>();
    for samples in frames_of_samples {
        for ((slot, sample), weight) in frame.iter_mut().zip(samples).zip(hann_window()) {
            *slot = sample * weight;
        }
        plan.process_with_scratch(&mut frame, &mut spectrum, &mut scratch)
            .expect("buffers are sized by the plan");

        if let Some(flatness) = band_flatness(&spectrum[band.clone()]) {
            total += flatness;
            frames += 1;
        }
    }

    if frames == 0 {
        return 0.0;
    }
    (total / f64::from(frames)) as f32
}

fn forward_plan() -> &'static dyn RealToComplex<f32> {
    static PLAN: OnceLock<Arc<dyn RealToComplex<f32>>> = OnceLock::new();
    PLAN.get_or_init(|| RealFftPlanner::new().plan_fft_forward(FLATNESS_FFT_SIZE))
        .as_ref()
}

/// FFT bins covering [`FLATNESS_BAND_HZ`], clamped below Nyquist.
fn band_bins(sample_rate: u32) -> RangeInclusive<usize> {
    let bin_hz = sample_rate as f32 / FLATNESS_FFT_SIZE as f32;
    let nyquist_bin = FLATNESS_FFT_SIZE / 2;
    let first = (FLATNESS_BAND_HZ.start() / bin_hz).ceil() as usize;
    let last = (FLATNESS_BAND_HZ.end() / bin_hz).floor() as usize;
    first.max(1)..=last.min(nyquist_bin)
}

/// Flatness of one frame's in-band bins, or `None` if the frame held
/// non-finite samples.
fn band_flatness(bins: &[Complex32]) -> Option<f64> {
    let (sum_ln, sum) = bins
        .iter()
        .map(|bin| f64::from(bin.norm_sqr()) + POWER_FLOOR)
        .fold((0.0, 0.0), |(sum_ln, sum), power| {
            (sum_ln + power.ln(), sum + power)
        });

    let count = bins.len() as f64;
    let flatness = (sum_ln / count).exp() / (sum / count);
    flatness.is_finite().then(|| flatness.clamp(0.0, 1.0))
}

/// Periodic Hann window of [`FLATNESS_FFT_SIZE`] points.
fn hann_window() -> &'static [f32] {
    static WINDOW: OnceLock<Vec<f32>> = OnceLock::new();
    WINDOW.get_or_init(|| {
        (0..FLATNESS_FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / FLATNESS_FFT_SIZE as f32).cos())
            .collect()
    })
}
