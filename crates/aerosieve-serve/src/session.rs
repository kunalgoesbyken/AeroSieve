use std::sync::Arc;
use std::time::{Duration, Instant};

use aerosieve::{LiveInspector, PipelineError, SegmentOutcome};
use axum::extract::ws::{Message, WebSocket};
use tokio::time::timeout;

use crate::AppState;
use crate::metrics::Metrics;
use crate::protocol::{ClientMessage, ENCODING, ProtocolError, ServerEvent, decode_pcm};

#[derive(Debug, thiserror::Error)]
enum Problem {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("text frames must be a control message such as {{\"type\":\"end\"}}")]
    UnknownControl,
    #[error("no message for {0:?}")]
    Idle(Duration),
    #[error("speech ran past {0} s without a pause long enough to end a clip")]
    Runaway(f32),
    #[error("segmentation failed: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("connection failed: {0}")]
    Socket(#[from] axum::Error),
}

enum Ending {
    /// The client asked for the pending verdicts and a summary.
    Ended,
    /// The client went away.
    Left,
}

/// Sends verdicts and counts them for the summary.
struct Reporter {
    sample_rate: u32,
    clips: usize,
    accepted: usize,
}

struct Session<'a> {
    live: LiveInspector,
    reporter: Reporter,
    state: &'a AppState,
}

pub(crate) async fn run(mut socket: WebSocket, state: Arc<AppState>) {
    let _active = state.metrics.session_started();
    let live = state.inspector.live();
    let mut session = Session {
        reporter: Reporter {
            sample_rate: live.sample_rate(),
            clips: 0,
            accepted: 0,
        },
        live,
        state: &state,
    };
    let ready = ServerEvent::Ready {
        sample_rate: session.live.sample_rate(),
        encoding: ENCODING,
        max_utterance_s: state.limits.max_utterance_s,
    };

    let result = match send(&mut socket, &ready).await {
        Ok(()) => session.listen(&mut socket).await,
        Err(error) => Err(error.into()),
    };
    let ended = match result {
        Ok(Ending::Ended) => session.conclude(&mut socket).await,
        Ok(Ending::Left) => Ok(()),
        Err(problem) => Err(problem),
    };
    if let Err(problem) = ended {
        tracing::info!(%problem, "session closed early");
        let message = problem.to_string();
        let _ = send(&mut socket, &ServerEvent::Error { message }).await;
    }
    let _ = socket.send(Message::Close(None)).await;
}

impl Session<'_> {
    async fn listen(&mut self, socket: &mut WebSocket) -> Result<Ending, Problem> {
        let idle = self.state.limits.idle_timeout;
        loop {
            let received = timeout(idle, socket.recv())
                .await
                .map_err(|_| Problem::Idle(idle))?;
            let Some(message) = received else {
                return Ok(Ending::Left);
            };
            match message? {
                Message::Binary(bytes) => self.hear(socket, &bytes).await?,
                Message::Text(text) => {
                    let ClientMessage::End =
                        serde_json::from_str(&text).map_err(|_| Problem::UnknownControl)?;
                    return Ok(Ending::Ended);
                }
                Message::Close(_) => return Ok(Ending::Left),
                Message::Ping(_) | Message::Pong(_) => {}
            }
        }
    }

    async fn hear(&mut self, socket: &mut WebSocket, bytes: &[u8]) -> Result<(), Problem> {
        let received = Instant::now();
        let samples = decode_pcm(bytes)?;
        let outcomes = self.live.push(&samples)?;
        let metrics = &self.state.metrics;
        metrics.audio_received(samples.len() as f32 / self.live.sample_rate() as f32);

        let limit = self.state.limits.max_utterance_s;
        if self.live.held_s() > limit {
            return Err(Problem::Runaway(limit));
        }
        if outcomes.is_empty() {
            return Ok(());
        }
        let decided_at_s = self.live.heard_s();
        self.reporter
            .report(socket, metrics, outcomes, decided_at_s)
            .await?;
        metrics.decision(received.elapsed());
        Ok(())
    }

    async fn conclude(self, socket: &mut WebSocket) -> Result<(), Problem> {
        let Session {
            live,
            mut reporter,
            state,
        } = self;
        let heard_s = live.heard_s();
        reporter
            .report(socket, &state.metrics, live.finish(), heard_s)
            .await?;
        let Reporter {
            clips, accepted, ..
        } = reporter;
        tracing::info!(heard_s, clips, accepted, "session ended");
        let summary = ServerEvent::Summary {
            heard_s,
            clips,
            accepted,
        };
        Ok(send(socket, &summary).await?)
    }
}

impl Reporter {
    async fn report(
        &mut self,
        socket: &mut WebSocket,
        metrics: &Metrics,
        outcomes: Vec<SegmentOutcome>,
        decided_at_s: f32,
    ) -> Result<(), Problem> {
        for outcome in outcomes {
            let clip = outcome.summary(self.sample_rate);
            self.clips += 1;
            self.accepted += usize::from(outcome.accepted());
            metrics.verdict(clip.verdict);
            send(socket, &ServerEvent::Clip { decided_at_s, clip }).await?;
        }
        Ok(())
    }
}

async fn send(socket: &mut WebSocket, event: &ServerEvent) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).expect("server events always serialize");
    socket.send(Message::Text(json.into())).await
}
