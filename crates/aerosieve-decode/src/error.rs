//! Error type for the decode stage.

use std::path::PathBuf;

/// Errors produced while decoding, resampling, or normalizing audio.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The input file could not be opened or read.
    #[error("io error on {path:?}: {source}")]
    Io {
        /// Path that failed to open, if the source was a file.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// symphonia could not identify the container or has no enabled decoder for it.
    #[error("unsupported or unrecognized audio format: {0}")]
    UnsupportedFormat(String),

    /// The container was parsed but contains no decodable audio track.
    #[error("no audio track found in input")]
    NoAudioTrack,

    /// The audio track declares no sample rate, so resampling cannot be planned.
    #[error("audio track has no declared sample rate")]
    MissingSampleRate,

    /// The audio track declares zero channels.
    #[error("audio track has no channels")]
    NoChannels,

    /// An unrecoverable error was raised while decoding packets.
    #[error("decode error: {0}")]
    Decode(String),

    /// The resampler could not be constructed or failed mid-stream.
    #[error("resample error: {0}")]
    Resample(String),

    /// The requested configuration is not usable (e.g. zero target sample rate).
    #[error("invalid decode config: {0}")]
    InvalidConfig(String),
}
