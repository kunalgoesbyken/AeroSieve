use std::path::PathBuf;

/// Errors produced while decoding, resampling, or normalizing audio.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The input file could not be opened.
    #[error("cannot open {}: {source}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The underlying reader failed mid-stream.
    #[error("read failed: {0}")]
    Read(#[source] std::io::Error),

    /// The container was not recognized, or no decoder exists for its codec.
    #[error("unsupported or unrecognized audio format: {0}")]
    UnsupportedFormat(String),

    #[error("no audio track found in input")]
    NoAudioTrack,

    #[error("audio track has no declared sample rate")]
    MissingSampleRate,

    #[error("audio track declares an unsupported sample rate of {0} Hz")]
    UnsupportedSampleRate(u32),

    #[error("audio is longer than the configured limit of {limit_s} s")]
    TooLong { limit_s: f32 },

    /// An unrecoverable error was raised while decoding packets.
    #[error("decode error: {0}")]
    Decode(String),

    #[error("resample error: {0}")]
    Resample(String),

    #[error("invalid decode config: {0}")]
    InvalidConfig(&'static str),
}
