//! WebAssembly bindings: run `aerosieve inspect` in a browser.
//!
//! The file's bytes go through the same decode, segment, and score code as
//! the command-line tool, so a verdict in the browser is the verdict the CLI
//! would give. Decoding and scoring happen once; [`Inspection::clips`] then
//! re-applies any policy to the stored measurements, which is cheap enough
//! to call on every slider movement.

use std::io::Cursor;
use std::path::Path;

use aerosieve::decode::decode_reader;
use aerosieve::{
    ClipSummary, DecodeConfig, DecodedAudio, FileOutcome, FilterPolicy, Inspector, SegmentConfig,
    SegmentOutcome,
};
use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::prelude::*;

/// Longest recording the browser demo will decode, in seconds.
const MAX_DURATION_S: f32 = 15.0 * 60.0;

/// A recording that has been decoded, segmented, and scored.
#[wasm_bindgen]
pub struct Inspection {
    outcome: FileOutcome,
    audio: DecodedAudio,
}

#[wasm_bindgen]
impl Inspection {
    /// Decode `bytes` (wav, mp3, flac, ogg, or mp4) and score every clip.
    ///
    /// `extension` is a probing hint such as `"mp3"` and may be empty.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>, extension: &str) -> Result<Inspection, JsError> {
        let decode = DecodeConfig {
            max_duration_s: Some(MAX_DURATION_S),
            ..DecodeConfig::default()
        };
        let hint = (!extension.is_empty()).then_some(extension);
        let audio = decode_reader(Cursor::new(bytes), hint, &decode)?;
        let mut inspector =
            Inspector::new(decode, SegmentConfig::default(), FilterPolicy::default())?;
        let outcome = inspector.inspect_audio(Path::new("upload"), &audio)?;
        Ok(Self { outcome, audio })
    }

    #[wasm_bindgen(getter)]
    pub fn duration_s(&self) -> f32 {
        self.outcome.duration_s
    }

    #[wasm_bindgen(getter)]
    pub fn source_sample_rate(&self) -> u32 {
        self.outcome.source_sample_rate
    }

    #[wasm_bindgen(getter)]
    pub fn sample_rate(&self) -> u32 {
        self.audio.sample_rate
    }

    /// Every clip with its measurements and the verdict `policy` gives it.
    ///
    /// `policy` is an object with any of the `FilterPolicy` fields
    /// (`min_snr_db`, `max_clip_ratio`, ...); missing fields keep their
    /// defaults. Pass `undefined` for the defaults.
    pub fn clips(&self, policy: JsValue) -> Result<JsValue, JsError> {
        let policy: FilterPolicy = if policy.is_undefined() || policy.is_null() {
            FilterPolicy::default()
        } else {
            serde_wasm_bindgen::from_value(policy)?
        };
        let clips: Vec<ClipSummary> = self
            .outcome
            .segments
            .iter()
            .map(|clip| {
                SegmentOutcome {
                    rejected: policy.evaluate(&clip.metrics),
                    ..clip.clone()
                }
                .summary(self.outcome.sample_rate)
            })
            .collect();
        Ok(clips.serialize(&Serializer::json_compatible())?)
    }

    /// Peak amplitude in `points` equal buckets, for drawing a waveform.
    pub fn envelope(&self, points: usize) -> Vec<f32> {
        self.audio.peaks(points)
    }

    /// The normalized 16 kHz mono samples, for playback.
    pub fn samples(&self) -> Vec<f32> {
        self.audio.samples.clone()
    }
}

/// The default filter thresholds, as an object.
#[wasm_bindgen(js_name = defaultPolicy)]
pub fn default_policy() -> Result<JsValue, JsError> {
    Ok(FilterPolicy::default().serialize(&Serializer::json_compatible())?)
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
