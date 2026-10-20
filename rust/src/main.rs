#![forbid(unsafe_code)]
use recency_tracker::{config::Config, server::Server};
use std::{path::PathBuf, process::ExitCode, time::Duration};

async fn signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {_=term.recv()=>Ok(()),result=tokio::signal::ctrl_c()=>result}
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var_os("RECENCY_TRACKER_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/config.json"));
    let secret = match std::env::var("RECENCY_TRACKER_PUBLISHER_SECRET") {
        Ok(s) => Some(s),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("publisher secret must be valid Unicode".into()),
    };
    let config = Config::load(&path, secret)?;
    let server = Server::start(config).await?;
    eprintln!(
        "recency-tracker {} listening on publisher {} and subscriber {}",
        env!("CARGO_PKG_VERSION"),
        server.publisher,
        server.subscriber
    );
    let result = signal().await;
    tokio::time::timeout(Duration::from_secs(15), server.shutdown()).await??;
    result?;
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("recency-tracker: {e}");
            ExitCode::FAILURE
        }
    }
}
