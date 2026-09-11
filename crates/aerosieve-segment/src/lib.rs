//! Hysteresis VAD segmentation: variable-length speech clips, not per-frame booleans.
//!
//! Stage 2 of the AeroSieve v2 batch curation pipeline. Takes a decoded
//! 16 kHz mono buffer and emits [`Segment`]s of speech, each bounded to
//! `[min_segment_s, max_segment_s]`.
//!
//! # Why a state machine
//!
//! A per-window accept/reject test flickers: a single quiet window mid-word
//! splits an utterance in two. Real VAD uses *hysteresis* — a high threshold to
//! enter speech, a lower one to leave it, and a minimum silence duration before
//! a dip counts as an ending. That is what [`Segmenter`] implements, following
//! Silero VAD's documented `get_speech_timestamps` semantics.
//!
//! # Algorithm
//!
//! 1. Walk the buffer in non-overlapping [`SpeechDetector::window_size`] windows.
//! 2. **Enter** speech when `p >= threshold`.
//! 3. While in speech, a run of windows with `p < neg_threshold` that lasts at
//!    least `min_silence_duration_ms` **exits** speech. The segment ends where
//!    that silence run *began*. Shorter dips are absorbed — this is what merges
//!    two bursts separated by a brief pause into one segment — and remembered
//!    as candidate split points.
//! 4. Raw runs shorter than `min_speech_duration_ms` are discarded.
//! 5. Survivors are padded by `speech_pad_ms` on both sides, clamped to the
//!    buffer.
//! 6. Segments longer than `max_segment_s` are split at the midpoint of their
//!    longest interior silence, recursively; with no interior silence, hard-split
//!    at `max_segment_s`.
//! 7. Segments shorter than `min_segment_s` are dropped.
//! 8. An in-progress segment is flushed at end of input.
//!
//! # Detectors
//!
//! [`EnergyDetector`] is the default: pure DSP, no models, builds on any
//! toolchain. [`SileroDetector`] is behind the opt-in `silero` feature and
//! needs an MSVC toolchain on Windows — see the crate README.
//!
//! # Example
//!
//! ```
//! use aerosieve_segment::{EnergyDetector, SegmentConfig, Segmenter};
//!
//! let sr = 16_000;
//! let mut samples = vec![0.0f32; sr as usize];           // 1 s of silence
//! samples.extend((0..sr as usize * 5).map(|i| {          // 5 s of tone
//!     (i as f32 * 0.05).sin() * 0.3
//! }));
//! samples.extend(std::iter::repeat(0.0).take(sr as usize));
//!
//! let mut seg = Segmenter::new(
//!     EnergyDetector::new(sr),
//!     SegmentConfig { sample_rate: sr, ..Default::default() },
//! );
//! let segments = seg.segment(&samples);
//! assert_eq!(segments.len(), 1);
//! assert!(segments[0].duration_s(sr) > 4.0);
//! ```

#![warn(missing_docs)]

mod detector;
mod error;
mod segmenter;

pub use detector::{
    DEFAULT_ENERGY_THRESHOLD_DBFS, EnergyDetector, SILERO_WINDOW_8K, SILERO_WINDOW_16K,
    SpeechDetector,
};
pub use error::SegmentError;
pub use segmenter::{Segment, SegmentConfig, Segmenter};

#[cfg(feature = "silero")]
mod silero;
#[cfg(feature = "silero")]
pub use silero::SileroDetector;

#[cfg(test)]
mod tests;
