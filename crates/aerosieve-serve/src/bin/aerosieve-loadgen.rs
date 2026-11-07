//! Replays a recording from many concurrent clients against a running
//! `aerosieve-serve` and reports verdict latency.
//!
//! Latency is measured per clip, from sending the frame that completed it to
//! receiving its verdict: network, queueing, and scoring together. The
//! algorithmic hold (the pause the segmenter must hear before it can end a
//! clip) is reported separately.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aerosieve::DecodeConfig;
use aerosieve_serve::encode_pcm;
use clap::{Parser, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Parser)]
struct Args {
    /// Recording to replay at the rate the server expects, at its recorded
    /// level: a live client sends what its microphone hears.
    audio: PathBuf,
    #[arg(long, default_value = "ws://127.0.0.1:8080/v1/stream")]
    url: String,
    /// Concurrent client sessions.
    #[arg(long, default_value_t = 16)]
    streams: usize,
    /// Seconds over which client starts are spread evenly. Zero starts every
    /// client at once, so identical replays end their utterances in the
    /// same instant: a worst case live traffic does not produce.
    #[arg(long, default_value_t = 0.0)]
    ramp_s: f64,
    /// Times each client replays the recording.
    #[arg(long, default_value_t = 1)]
    repeat: usize,
    #[arg(long, default_value_t = 20)]
    frame_ms: u32,
    #[arg(long, value_enum, default_value_t = Pace::Realtime)]
    pace: Pace,
    /// Also write the report here as JSON.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum Pace {
    /// One frame per frame duration, like a live microphone.
    Realtime,
    /// As fast as the server accepts frames.
    Max,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    sample_rate: Option<u32>,
    decided_at_s: Option<f64>,
    start_s: Option<f64>,
    duration_s: Option<f64>,
    verdict: Option<String>,
    message: Option<String>,
}

#[derive(Default)]
struct StreamResult {
    latencies_ms: Vec<f64>,
    holds_ms: Vec<f64>,
    verdicts: BTreeMap<String, usize>,
    error: Option<String>,
}

#[derive(Serialize)]
struct Report {
    streams: usize,
    ramp_s: f64,
    pace: Pace,
    frame_ms: u32,
    audio_s_per_stream: f64,
    wall_s: f64,
    realtime_factor: f64,
    clips: usize,
    verdicts: BTreeMap<String, usize>,
    latency_ms: Percentiles,
    hold_ms: Percentiles,
    failed_streams: usize,
    first_error: Option<String>,
}

#[derive(Serialize)]
struct Percentiles {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

impl Percentiles {
    fn of(mut values: Vec<f64>) -> Self {
        values.sort_by(f64::total_cmp);
        let rank = |p: f64| {
            let index = ((p * values.len() as f64).ceil() as usize).saturating_sub(1);
            values.get(index).copied().unwrap_or(f64::NAN)
        };
        Self {
            p50: rank(0.50),
            p95: rank(0.95),
            p99: rank(0.99),
            max: values.last().copied().unwrap_or(f64::NAN),
        }
    }
}

struct Replay {
    url: String,
    sample_rate: u32,
    frames: Arc<Vec<Vec<u8>>>,
    frame_samples: usize,
    frame: Duration,
    pace: Pace,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let decode = DecodeConfig {
        normalize: false,
        ..DecodeConfig::default()
    };
    let sample_rate = decode.target_sample_rate;
    let recording = aerosieve_decode::decode_file(&args.audio, &decode)?.samples;
    let pause = vec![0.0; sample_rate as usize];
    let once = [recording.as_slice(), &pause].concat();
    let audio: Vec<f32> = std::iter::repeat_n(once, args.repeat).flatten().collect();
    let frame_samples = (sample_rate * args.frame_ms / 1000) as usize;
    let replay = Arc::new(Replay {
        url: args.url.clone(),
        sample_rate,
        frames: Arc::new(audio.chunks(frame_samples).map(encode_pcm).collect()),
        frame_samples,
        frame: Duration::from_millis(u64::from(args.frame_ms)),
        pace: args.pace,
    });

    let started = Instant::now();
    let mut clients = JoinSet::new();
    for index in 0..args.streams {
        let replay = Arc::clone(&replay);
        let delay = Duration::from_secs_f64(args.ramp_s * index as f64 / args.streams as f64);
        clients.spawn(async move {
            tokio::time::sleep(delay).await;
            replay.run().await
        });
    }
    let results = clients.join_all().await;
    let wall_s = started.elapsed().as_secs_f64();

    let audio_s = audio.len() as f64 / f64::from(sample_rate);
    let report = Report::new(results, &args, audio_s, wall_s);
    let json = serde_json::to_string_pretty(&report)?;
    println!("{json}");
    if let Some(path) = &args.out {
        std::fs::write(path, json + "\n")?;
    }
    Ok(())
}

impl Replay {
    async fn run(&self) -> StreamResult {
        let mut result = StreamResult::default();
        if let Err(error) = self.stream(&mut result).await {
            result.error = Some(error.to_string());
        }
        result
    }

    async fn stream(&self, result: &mut StreamResult) -> Result<(), BoxError> {
        let (socket, _) = connect_async(&self.url).await?;
        let (mut sink, mut events) = socket.split();
        let ready = next_event(&mut events).await?;
        if ready.sample_rate != Some(self.sample_rate) {
            let expected = ready.sample_rate;
            return Err(format!("server expects {expected:?} Hz, not {}", self.sample_rate).into());
        }

        let sent_at = Arc::new(Mutex::new(Vec::with_capacity(self.frames.len())));
        let sender = {
            let sent_at = Arc::clone(&sent_at);
            let frames = Arc::clone(&self.frames);
            let (frame, pace) = (self.frame, self.pace);
            tokio::spawn(async move {
                let start = tokio::time::Instant::now();
                for (index, bytes) in frames.iter().enumerate() {
                    if let Pace::Realtime = pace {
                        tokio::time::sleep_until(start + frame * index as u32).await;
                    }
                    sent_at.lock().expect("never poisoned").push(Instant::now());
                    sink.send(Message::binary(bytes.clone())).await?;
                }
                sink.send(Message::text(r#"{"type":"end"}"#)).await
            })
        };

        loop {
            let event = next_event(&mut events).await?;
            match event.kind.as_str() {
                "clip" => self.record(&event, &sent_at, result),
                "summary" => break,
                _ => return Err(event.message.unwrap_or(event.kind).into()),
            }
        }
        sender.await??;
        Ok(())
    }

    fn record(&self, event: &Event, sent_at: &Mutex<Vec<Instant>>, result: &mut StreamResult) {
        let received = Instant::now();
        let (Some(decided), Some(start), Some(duration)) =
            (event.decided_at_s, event.start_s, event.duration_s)
        else {
            return;
        };
        let frames_heard = decided * f64::from(self.sample_rate) / self.frame_samples as f64;
        let sent_at = sent_at.lock().expect("never poisoned");
        let completing = (frames_heard.round() as usize).checked_sub(1);
        if let Some(sent) = completing.and_then(|index| sent_at.get(index)) {
            let latency = received.duration_since(*sent);
            result.latencies_ms.push(latency.as_secs_f64() * 1000.0);
        }
        result.holds_ms.push((decided - start - duration) * 1000.0);
        let verdict = event.verdict.clone().unwrap_or_default();
        *result.verdicts.entry(verdict).or_default() += 1;
    }
}

async fn next_event<S>(events: &mut S) -> Result<Event, BoxError>
where
    S: StreamExt<Item = Result<Message, WsError>> + Unpin,
{
    while let Some(message) = events.next().await {
        if let Message::Text(text) = message? {
            return Ok(serde_json::from_str(&text)?);
        }
    }
    Err("server closed the connection".into())
}

impl Report {
    fn new(results: Vec<StreamResult>, args: &Args, audio_s: f64, wall_s: f64) -> Self {
        let mut verdicts = BTreeMap::new();
        let (mut latencies, mut holds) = (Vec::new(), Vec::new());
        let mut errors = Vec::new();
        for result in results {
            latencies.extend(result.latencies_ms);
            holds.extend(result.holds_ms);
            errors.extend(result.error);
            for (verdict, count) in result.verdicts {
                *verdicts.entry(verdict).or_default() += count;
            }
        }
        Self {
            streams: args.streams,
            ramp_s: args.ramp_s,
            pace: args.pace,
            frame_ms: args.frame_ms,
            audio_s_per_stream: audio_s,
            wall_s,
            realtime_factor: audio_s * args.streams as f64 / wall_s,
            clips: verdicts.values().sum(),
            verdicts,
            latency_ms: Percentiles::of(latencies),
            hold_ms: Percentiles::of(holds),
            failed_streams: errors.len(),
            first_error: errors.into_iter().next(),
        }
    }
}
