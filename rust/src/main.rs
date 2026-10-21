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
    let mut config = Config::load(&path, secret)?;
    match std::env::var("RECENCY_TRACKER_SUBSCRIBER_ALLOWED_ORIGINS") {
        Ok(origins) => config.override_allowed_origins(&origins)?,
        Err(std::env::VarError::NotPresent) => {}
        Err(_) => return Err("allowed origins override must be valid Unicode".into()),
    }
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
fn run_with_runtime<F: std::future::Future>(future: F) -> std::io::Result<F::Output> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(future);
    // The async shutdown already spends the full grace period draining work.
    // Runtime drop would otherwise wait indefinitely for blocked filesystem calls.
    runtime.shutdown_timeout(Duration::ZERO);
    Ok(result)
}
fn main() -> ExitCode {
    let result = run_with_runtime(run()).unwrap_or_else(|e| Err(e.into()));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("recency-tracker: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_does_not_wait_for_stalled_blocking_work() {
        let (release, blocked) = std::sync::mpsc::channel();
        let (finished, completion) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = run_with_runtime(async move {
                let (started, ready) = tokio::sync::oneshot::channel();
                tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    let _ = blocked.recv();
                });
                ready.await.unwrap();
                tokio::time::timeout(Duration::from_millis(10), std::future::pending::<()>()).await
            });
            finished.send(result).unwrap();
        });
        let result = completion.recv_timeout(Duration::from_secs(2));
        // Always release the worker, including if runtime teardown regresses.
        let _ = release.send(());
        worker.join().unwrap();
        assert!(
            result
                .expect("runtime teardown exceeded its deadline")
                .unwrap()
                .is_err()
        );
    }
}
