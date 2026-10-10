//! Band-limited sample-rate conversion with rubato's `SincFixedIn`.
//!
//! Two properties of that resampler shape this module, and ignoring either
//! silently corrupts the end of every file:
//!
//! - It consumes a fixed number of input frames per call, so the trailing
//!   partial chunk has to go through `process_partial_into_buffer`.
//! - Its filter is centered, so the first `output_delay()` output frames are
//!   ramp-up on zeros. They are discarded, and the same amount of extra output
//!   is flushed at the end to recover the real tail.
//!
//! The output is then exactly `round(input_len * to_rate / from_rate)` frames.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::error::DecodeError;

const CHUNK_FRAMES: usize = 1024;
const SINC_LEN: usize = 256;
const OVERSAMPLING_FACTOR: usize = 256;

/// Cutoff relative to Nyquist. Downsampling backs off further so the
/// transition band stays inside the new Nyquist limit.
const UPSAMPLE_CUTOFF: f32 = 0.95;
const DOWNSAMPLE_CUTOFF: f32 = 0.92;

/// Resample a mono signal, preserving its duration to within one sample.
pub fn resample_mono(
    input: Vec<f32>,
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f32>, DecodeError> {
    if from_rate == to_rate || input.is_empty() {
        return Ok(input);
    }

    let ratio = f64::from(to_rate) / f64::from(from_rate);
    let expected_len = (input.len() as f64 * ratio).round() as usize;

    let mut resampler =
        SincFixedIn::<f32>::new(ratio, 1.0, sinc_parameters(ratio), CHUNK_FRAMES, 1)
            .map_err(resample_error)?;
    let delay = resampler.output_delay();
    let mut scratch = resampler.output_buffer_allocate(true);
    let mut output = Vec::with_capacity(expected_len + delay + CHUNK_FRAMES);

    for chunk in input.chunks(CHUNK_FRAMES) {
        let (_, written) = if chunk.len() == CHUNK_FRAMES {
            resampler.process_into_buffer(&[chunk], &mut scratch, None)
        } else {
            resampler.process_partial_into_buffer(Some(&[chunk]), &mut scratch, None)
        }
        .map_err(resample_error)?;
        output.extend_from_slice(&scratch[0][..written]);
    }

    while output.len() < expected_len + delay {
        let (_, written) = resampler
            .process_partial_into_buffer::<&[f32], _>(None, &mut scratch, None)
            .map_err(resample_error)?;
        if written == 0 {
            break;
        }
        output.extend_from_slice(&scratch[0][..written]);
    }

    output.drain(..delay.min(output.len()));
    output.resize(expected_len, 0.0);
    Ok(output)
}

fn sinc_parameters(ratio: f64) -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: SINC_LEN,
        f_cutoff: if ratio < 1.0 {
            DOWNSAMPLE_CUTOFF
        } else {
            UPSAMPLE_CUTOFF
        },
        oversampling_factor: OVERSAMPLING_FACTOR,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    }
}

fn resample_error(error: impl std::fmt::Display) -> DecodeError {
    DecodeError::Resample(error.to_string())
}
