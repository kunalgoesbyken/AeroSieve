//! Voice-activity segmentation: cut decoded audio into variable-length
//! speech clips.
//!
//! A per-window speech/silence decision flickers: one quiet window mid-word
//! splits an utterance. [`Segmenter`] instead runs a hysteresis state machine
//! with Silero VAD's semantics, so a brief dip is bridged rather than treated
//! as an ending.
//!
//! [`EnergyDetector`] is the default detector and has no model. The `silero`
//! feature adds `SileroDetector`, which runs Silero VAD through ONNX Runtime.
//!
//! ```
//! use aerosieve_segment::{EnergyDetector, SegmentConfig, Segmenter};
//!
//! let sample_rate = 16_000;
//! let second = sample_rate as usize;
//! let mut samples = vec![0.0f32; second];
//! samples.extend((0..5 * second).map(|i| (i as f32 * 0.05).sin() * 0.3));
//! samples.extend(vec![0.0f32; second]);
//!
//! let mut segmenter = Segmenter::new(
//!     EnergyDetector::new(sample_rate),
//!     SegmentConfig { sample_rate, ..Default::default() },
//! );
//! let segments = segmenter.segment(&samples)?;
//!
//! assert_eq!(segments.len(), 1);
//! assert!(segments[0].duration_s(sample_rate) > 4.0);
//! # Ok::<(), aerosieve_segment::SegmentError>(())
//! ```

#![warn(missing_docs)]

mod detector;
mod error;
mod segmenter;
#[cfg(feature = "silero")]
mod silero;

#[cfg(test)]
mod tests;

pub use detector::{DEFAULT_ENERGY_THRESHOLD_DBFS, EnergyDetector, SpeechDetector, WINDOW_MS};
pub use error::SegmentError;
pub use segmenter::{Segment, SegmentConfig, Segmenter};
#[cfg(feature = "silero")]
pub use silero::SileroDetector;
