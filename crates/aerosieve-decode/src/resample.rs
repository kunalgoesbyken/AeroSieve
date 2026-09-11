//! Band-limited sample-rate conversion to the canonical rate.
//!
//! rubato's `SincFixedIn` is an FIR-based asynchronous resampler. Two facts
//! about it drive the code below, and getting either wrong silently truncates
//! the end of every file:
//!
//! 1. **Fixed input chunking.** `process` consumes exactly
//!    `input_frames_next()` frames per call. A file is almost never an exact
//!    multiple of that, so the trailing partial chunk must be flushed with
//!    `process_partial`, which zero-pads internally.
//! 2. **Filter latency.** The sinc filter is centered, so the first
//!    `output_delay()` output frames are the filter ramping up on zeros. They
//!    must be discarded, and an equal amount of extra output must be pulled out
//!    of the resampler at the end to recover the real tail.
//!
//! The net effect is that we drop `output_delay()` frames from the front and
//! truncate to the analytically expected length
//! `round(input_len * target_rate / source_rate)`.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::error::DecodeError;

/// Input frames handed to the resampler per `process` call.
const CHUNK_FRAMES: usize = 1024;

/// Length of the windowed-sinc interpolation filter.
const SINC_LEN: usize = 256;

fn params(ratio: f64) -> SincInterpolationParameters {
    // Cutoff is relative to the lower of the two Nyquist frequencies. When
    // downsampling, back the cutoff off slightly to keep the transition band
    // inside the new Nyquist limit and avoid aliasing.
    let f_cutoff = if ratio < 1.0 { 0.92 } else { 0.95 };
    SincInterpolationParameters {
        sinc_len: SINC_LEN,
        f_cutoff,
        oversampling_factor: 256,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    }
}

/// Resample a mono signal from `from_rate` to `to_rate`.
///
/// Returns the input untouched when the rates already match. Preserves duration
/// to within a sample: the output length is `round(len * to/from)`.
pub fn resample_mono(
    input: Vec<f32>,
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f32>, DecodeError> {
    if from_rate == to_rate || input.is_empty() {
        return Ok(input);
    }

    let ratio = f64::from(to_rate) / f64::from(from_rate);
    let expected_len = ((input.len() as f64) * ratio).round() as usize;

    let mut resampler = SincFixedIn::<f32>::new(ratio, 1.0, params(ratio), CHUNK_FRAMES, 1)
        .map_err(|e| DecodeError::Resample(e.to_string()))?;

    let delay = resampler.output_delay();
    // Room for the real signal plus the leading filter delay we will drop.
    let mut out: Vec<f32> = Vec::with_capacity(expected_len + delay + CHUNK_FRAMES);

    let mut pos = 0usize;
    let mut scratch = resampler.output_buffer_allocate(true);

    // Full chunks.
    loop {
        let need = resampler.input_frames_next();
        if pos + need > input.len() {
            break;
        }
        let chunk = [&input[pos..pos + need]];
        let (_, written) = resampler
            .process_into_buffer(&chunk, &mut scratch, None)
            .map_err(|e| DecodeError::Resample(e.to_string()))?;
        out.extend_from_slice(&scratch[0][..written]);
        pos += need;
    }

    // Trailing partial chunk: `process_partial` zero-pads it to the required
    // chunk size internally.
    if pos < input.len() {
        let chunk = [&input[pos..]];
        let (_, written) = resampler
            .process_partial_into_buffer(Some(&chunk), &mut scratch, None)
            .map_err(|e| DecodeError::Resample(e.to_string()))?;
        out.extend_from_slice(&scratch[0][..written]);
    }

    // Flush: keep pushing silence through until we have produced the delayed
    // frames too, otherwise the last `delay` output samples of real audio are
    // still sitting inside the filter.
    let target = expected_len + delay;
    while out.len() < target {
        let (_, written) = resampler
            .process_partial_into_buffer::<&[f32], _>(None, &mut scratch, None)
            .map_err(|e| DecodeError::Resample(e.to_string()))?;
        if written == 0 {
            break;
        }
        out.extend_from_slice(&scratch[0][..written]);
    }

    // Drop the leading filter ramp-up, then trim to the analytic length.
    let start = delay.min(out.len());
    let mut out = out.split_off(start);
    out.truncate(expected_len);
    // If the resampler ran dry slightly early, pad so duration is exact.
    out.resize(expected_len, 0.0);

    Ok(out)
}
