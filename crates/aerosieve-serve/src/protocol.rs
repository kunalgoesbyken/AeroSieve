use aerosieve::ClipSummary;
use serde::{Deserialize, Serialize};

/// Audio format of every binary frame a client sends.
pub const ENCODING: &str = "pcm_s16le";

const BYTES_PER_SAMPLE: usize = 2;

/// A message the server sends, as one JSON text frame.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// First message of every session: the audio the server expects.
    Ready {
        sample_rate: u32,
        encoding: &'static str,
        max_utterance_s: f32,
    },
    /// A clip was cut and judged.
    Clip {
        /// Stream position, in seconds, when the verdict was reached.
        decided_at_s: f32,
        #[serde(flatten)]
        clip: ClipSummary,
    },
    /// Last message of a session the client ended.
    Summary {
        heard_s: f32,
        clips: usize,
        accepted: usize,
    },
    /// The session was refused or broke a limit and is closing.
    Error { message: String },
}

/// A control message a client sends as a JSON text frame.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// No more audio: judge what is pending, send a summary, and close.
    End,
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("audio frames must hold whole {ENCODING} samples; got {0} bytes")]
    PartialSample(usize),
}

/// Little-endian signed 16-bit PCM to samples in `-1.0..1.0`.
pub fn decode_pcm(bytes: &[u8]) -> Result<Vec<f32>, ProtocolError> {
    let (samples, partial) = bytes.as_chunks::<BYTES_PER_SAMPLE>();
    if !partial.is_empty() {
        return Err(ProtocolError::PartialSample(bytes.len()));
    }
    Ok(samples
        .iter()
        .map(|&pair| f32::from(i16::from_le_bytes(pair)) / -f32::from(i16::MIN))
        .collect())
}

/// Samples to little-endian signed 16-bit PCM, clamping to full scale.
pub fn encode_pcm(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| {
            let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
            scaled.to_le_bytes()
        })
        .collect()
}
