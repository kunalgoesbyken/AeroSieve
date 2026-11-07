//! A streaming speech-quality gate.
//!
//! Clients open a WebSocket at `/v1/stream`, send audio as binary frames of
//! 16-bit little-endian mono PCM at the advertised sample rate, and receive
//! one JSON verdict per clip as soon as the pause that ends it is heard.
//! Verdicts match what batch curation reaches on the same audio.
//!
//! ```text
//! server  {"type":"ready","sample_rate":16000,"encoding":"pcm_s16le",...}
//! client  <binary PCM> <binary PCM> ...
//! server  {"type":"clip","decided_at_s":5.1,"start_s":0.47,"verdict":"accept",...}
//! client  {"type":"end"}
//! server  {"type":"summary","heard_s":12.0,"clips":2,"accepted":1}
//! ```
//!
//! `/metrics` serves Prometheus counters and a decision-latency histogram;
//! `/healthz` answers `ok`.

mod metrics;
mod protocol;
mod session;

use std::sync::Arc;
use std::time::Duration;

use aerosieve::{DecodeConfig, FilterPolicy, Inspector, PipelineError, SegmentConfig};
use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::sync::Semaphore;

pub use protocol::{ClientMessage, ENCODING, ProtocolError, ServerEvent, decode_pcm, encode_pcm};

use metrics::Metrics;

/// How the server judges audio and what it will accept from clients.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub segment: SegmentConfig,
    pub policy: FilterPolicy,
    /// Concurrent sessions; further upgrades are refused with 503.
    pub max_sessions: usize,
    /// Longest run of speech, without a clip-ending pause, held in memory
    /// before the session is closed. Bounds memory per session.
    pub max_utterance_s: f32,
    /// A session that sends nothing for this long is closed.
    pub idle_timeout: Duration,
    /// Largest accepted WebSocket message, in bytes.
    pub max_frame_bytes: usize,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            segment: SegmentConfig::default(),
            policy: FilterPolicy::default(),
            max_sessions: 256,
            max_utterance_s: 120.0,
            idle_timeout: Duration::from_secs(30),
            max_frame_bytes: 1 << 20,
        }
    }
}

struct Limits {
    max_utterance_s: f32,
    idle_timeout: Duration,
    max_frame_bytes: usize,
}

struct AppState {
    inspector: Inspector,
    limits: Limits,
    slots: Arc<Semaphore>,
    metrics: Metrics,
}

/// The HTTP routes, ready to serve.
pub fn router(config: ServeConfig) -> Result<Router, PipelineError> {
    let decode = DecodeConfig {
        target_sample_rate: config.segment.sample_rate,
        ..DecodeConfig::default()
    };
    let state = AppState {
        inspector: Inspector::new(decode, config.segment, config.policy)?,
        limits: Limits {
            max_utterance_s: config.max_utterance_s,
            idle_timeout: config.idle_timeout,
            max_frame_bytes: config.max_frame_bytes,
        },
        slots: Arc::new(Semaphore::new(config.max_sessions)),
        metrics: Metrics::default(),
    };
    Ok(Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(render_metrics))
        .route("/v1/stream", get(open_stream))
        .with_state(Arc::new(state)))
}

async fn render_metrics(State(state): State<Arc<AppState>>) -> String {
    state.metrics.render()
}

async fn open_stream(State(state): State<Arc<AppState>>, upgrade: WebSocketUpgrade) -> Response {
    let Ok(slot) = Arc::clone(&state.slots).try_acquire_owned() else {
        state.metrics.session_refused();
        return (StatusCode::SERVICE_UNAVAILABLE, "at capacity, retry later").into_response();
    };
    upgrade
        .max_message_size(state.limits.max_frame_bytes)
        .on_upgrade(move |socket| async move {
            session::run(socket, state).await;
            drop(slot);
        })
}
