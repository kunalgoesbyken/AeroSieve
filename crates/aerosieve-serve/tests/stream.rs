use std::path::Path;

use aerosieve::{DecodeConfig, DecodedAudio, FilterPolicy, Inspector, SegmentConfig};
use aerosieve_fixtures::{
    SAMPLE_RATE, glottal_speech, harmonic_speech, overdrive, samples, silence, tone,
};
use aerosieve_serve::{ServeConfig, decode_pcm, encode_pcm, router};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tower::ServiceExt;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const FRAME: usize = 320;

struct Server {
    url: String,
    routes: Router,
}

async fn start(config: ServeConfig) -> Server {
    let routes = router(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/stream", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, routes.clone()).into_future());
    Server { url, routes }
}

async fn connect(server: &Server) -> Socket {
    let (mut socket, _) = connect_async(&server.url).await.unwrap();
    let ready = next_event(&mut socket).await;
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["sample_rate"], SAMPLE_RATE);
    socket
}

async fn next_event(socket: &mut Socket) -> Value {
    loop {
        match socket
            .next()
            .await
            .expect("server closed without an event")
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Close(_) => panic!("server closed without an event"),
            _ => {}
        }
    }
}

async fn send_audio(socket: &mut Socket, audio: &[f32]) {
    for frame in audio.chunks(FRAME) {
        socket
            .send(Message::binary(encode_pcm(frame)))
            .await
            .unwrap();
    }
}

async fn end(socket: &mut Socket) -> (Vec<Value>, Value) {
    socket
        .send(Message::text(r#"{"type":"end"}"#))
        .await
        .unwrap();
    let mut clips = Vec::new();
    loop {
        let event = next_event(socket).await;
        match event["type"].as_str() {
            Some("clip") => clips.push(event),
            Some("summary") => return (clips, event),
            _ => panic!("unexpected {event}"),
        }
    }
}

fn session_audio() -> Vec<f32> {
    let speech = glottal_speech(samples(4.0));
    let audio = [
        silence(samples(0.5)),
        speech.clone(),
        silence(samples(0.8)),
        overdrive(&speech, 8.0),
        silence(samples(0.8)),
        harmonic_speech(samples(6.0), 7),
        silence(samples(0.2)),
    ]
    .concat();
    decode_pcm(&encode_pcm(&audio)).unwrap()
}

#[tokio::test]
async fn streamed_verdicts_match_batch_curation() {
    let audio = session_audio();
    let batch = Inspector::new(
        DecodeConfig::default(),
        SegmentConfig::default(),
        FilterPolicy::default(),
    )
    .unwrap()
    .inspect_audio(
        Path::new("session"),
        &DecodedAudio::from_samples(audio.clone(), SAMPLE_RATE),
    )
    .unwrap();
    let server = start(ServeConfig::default()).await;
    let mut socket = connect(&server).await;

    send_audio(&mut socket, &audio).await;
    let (clips, summary) = end(&mut socket).await;

    let streamed: Vec<_> = clips
        .iter()
        .map(|clip| {
            (
                clip["verdict"].as_str().unwrap().to_owned(),
                clip["start_s"].as_f64().unwrap() as f32,
            )
        })
        .collect();
    let expected: Vec<_> = batch
        .segments
        .iter()
        .map(|outcome| {
            let clip = outcome.summary(SAMPLE_RATE);
            (clip.verdict.to_owned(), clip.start_s)
        })
        .collect();
    assert_eq!(streamed, expected);
    assert!(
        expected
            .iter()
            .any(|(verdict, _)| verdict == aerosieve::ACCEPTED)
    );
    assert!(
        expected
            .iter()
            .any(|(verdict, _)| verdict != aerosieve::ACCEPTED)
    );
    for clip in &clips {
        let ends_at = clip["start_s"].as_f64().unwrap() + clip["duration_s"].as_f64().unwrap();
        assert!(clip["decided_at_s"].as_f64().unwrap() >= ends_at);
    }
    assert_eq!(summary["clips"], batch.segments.len());
    assert_eq!(summary["accepted"], batch.accepted_count());
}

#[tokio::test]
async fn a_verdict_arrives_before_the_stream_ends() {
    let server = start(ServeConfig::default()).await;
    let mut socket = connect(&server).await;

    send_audio(
        &mut socket,
        &[
            silence(samples(0.3)),
            glottal_speech(samples(4.0)),
            silence(samples(1.0)),
        ]
        .concat(),
    )
    .await;

    let event = next_event(&mut socket).await;
    assert_eq!(event["type"], "clip");
    assert_eq!(event["verdict"], aerosieve::ACCEPTED);
}

#[tokio::test]
async fn a_frame_holding_half_a_sample_is_refused() {
    let server = start(ServeConfig::default()).await;
    let mut socket = connect(&server).await;

    socket.send(Message::binary(vec![0u8; 3])).await.unwrap();

    let event = next_event(&mut socket).await;
    assert_eq!(event["type"], "error");
    assert!(event["message"].as_str().unwrap().contains("3 bytes"));
}

#[tokio::test]
async fn unknown_control_messages_are_refused() {
    let server = start(ServeConfig::default()).await;
    let mut socket = connect(&server).await;

    socket
        .send(Message::text(r#"{"type":"pause"}"#))
        .await
        .unwrap();

    assert_eq!(next_event(&mut socket).await["type"], "error");
}

#[tokio::test]
async fn speech_without_a_pause_past_the_limit_closes_the_session() {
    let server = start(ServeConfig {
        max_utterance_s: 2.0,
        ..ServeConfig::default()
    })
    .await;
    let mut socket = connect(&server).await;

    send_audio(&mut socket, &tone(SAMPLE_RATE, 200.0, 0.3, samples(3.0))).await;

    let event = next_event(&mut socket).await;
    assert_eq!(event["type"], "error");
    assert!(event["message"].as_str().unwrap().contains("pause"));
}

#[tokio::test]
async fn sessions_beyond_capacity_are_refused_with_503() {
    let server = start(ServeConfig {
        max_sessions: 1,
        ..ServeConfig::default()
    })
    .await;
    let _first = connect(&server).await;

    let refused = connect_async(&server.url).await.unwrap_err();

    let WsError::Http(response) = refused else {
        panic!("expected an HTTP refusal, got {refused}");
    };
    assert_eq!(response.status(), 503);
}

#[tokio::test]
async fn metrics_count_sessions_verdicts_and_decisions() {
    let server = start(ServeConfig::default()).await;
    let mut socket = connect(&server).await;
    send_audio(&mut socket, &session_audio()).await;
    let (clips, _) = end(&mut socket).await;

    let request = axum::http::Request::get("/metrics")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = server.routes.clone().oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("aerosieve_sessions_total 1"));
    assert!(text.contains("aerosieve_clips_total{verdict=\"accept\"}"));
    let decisions = text
        .lines()
        .find_map(|line| line.strip_prefix("aerosieve_decision_seconds_count "))
        .unwrap();
    assert!(decisions.parse::<usize>().unwrap() >= 1);
    assert!(decisions.parse::<usize>().unwrap() <= clips.len());
}
