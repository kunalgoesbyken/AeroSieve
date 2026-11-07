use std::net::SocketAddr;
use std::time::Duration;

use aerosieve_serve::{ServeConfig, router};
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Streaming speech-quality gate over WebSocket.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    addr: SocketAddr,
    /// Concurrent sessions before new ones are refused.
    #[arg(long, default_value_t = ServeConfig::default().max_sessions)]
    max_sessions: usize,
    /// Seconds of unbroken speech held before a session is closed.
    #[arg(long, default_value_t = ServeConfig::default().max_utterance_s)]
    max_utterance_s: f32,
    /// Seconds without a message before a session is closed.
    #[arg(long, default_value_t = ServeConfig::default().idle_timeout.as_secs())]
    idle_timeout_s: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();
    let config = ServeConfig {
        max_sessions: args.max_sessions,
        max_utterance_s: args.max_utterance_s,
        idle_timeout: Duration::from_secs(args.idle_timeout_s),
        ..ServeConfig::default()
    };

    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(listener, router(config)?)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
