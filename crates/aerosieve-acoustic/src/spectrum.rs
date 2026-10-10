//! Spectral flatness over fixed-size Hann-windowed frames.

use std::f32::consts::TAU;
use std::ops::RangeInclusive;

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

    let window = hann_window(FLATNESS_FFT_SIZE);
    let mut re = vec![0.0f32; FLATNESS_FFT_SIZE];
    let mut im = vec![0.0f32; FLATNESS_FFT_SIZE];
    let mut total = 0.0f64;
    let mut frames = 0u32;

    let (frames_of_samples, _partial) = samples.as_chunks::<FLATNESS_FFT_SIZE>();
    for frame in frames_of_samples {
        for ((bin, sample), weight) in re.iter_mut().zip(frame).zip(&window) {
            *bin = sample * weight;
        }
        im.fill(0.0);
        fft_in_place(&mut re, &mut im);

        if let Some(flatness) = band_flatness(&re[band.clone()], &im[band.clone()]) {
            total += flatness;
            frames += 1;
        }
    }

    if frames == 0 {
        return 0.0;
    }
    (total / f64::from(frames)) as f32
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
fn band_flatness(re: &[f32], im: &[f32]) -> Option<f64> {
    let (sum_ln, sum) = re
        .iter()
        .zip(im)
        .map(|(&re, &im)| f64::from(re).powi(2) + f64::from(im).powi(2) + POWER_FLOOR)
        .fold((0.0, 0.0), |(sum_ln, sum), power| {
            (sum_ln + power.ln(), sum + power)
        });

    let bins = re.len() as f64;
    let flatness = (sum_ln / bins).exp() / (sum / bins);
    flatness.is_finite().then(|| flatness.clamp(0.0, 1.0))
}

/// Periodic Hann window.
fn hann_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| 0.5 - 0.5 * (TAU * i as f32 / len as f32).cos())
        .collect()
}

/// Iterative radix-2 Cooley-Tukey FFT. Both slices must share one
/// power-of-two length.
pub(crate) fn fft_in_place(re: &mut [f32], im: &mut [f32]) {
    let len = re.len();
    debug_assert_eq!(len, im.len());
    debug_assert!(len.is_power_of_two());

    bit_reverse_permute(re, im);

    let mut span = 2;
    while span <= len {
        let half = span / 2;
        let angle = -TAU / span as f32;
        let (step_re, step_im) = (angle.cos(), angle.sin());

        for block in (0..len).step_by(span) {
            let (mut twiddle_re, mut twiddle_im) = (1.0f32, 0.0f32);
            for offset in 0..half {
                let even = block + offset;
                let odd = even + half;
                let odd_re = re[odd] * twiddle_re - im[odd] * twiddle_im;
                let odd_im = re[odd] * twiddle_im + im[odd] * twiddle_re;
                re[odd] = re[even] - odd_re;
                im[odd] = im[even] - odd_im;
                re[even] += odd_re;
                im[even] += odd_im;
                (twiddle_re, twiddle_im) = (
                    twiddle_re * step_re - twiddle_im * step_im,
                    twiddle_re * step_im + twiddle_im * step_re,
                );
            }
        }
        span *= 2;
    }
}

fn bit_reverse_permute(re: &mut [f32], im: &mut [f32]) {
    let len = re.len();
    let mut reversed = 0;
    for index in 1..len {
        let mut bit = len >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed |= bit;
        if index < reversed {
            re.swap(index, reversed);
            im.swap(index, reversed);
        }
    }
}
