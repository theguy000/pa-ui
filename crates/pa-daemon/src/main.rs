//! `pa-daemon`: supervisor and session-worker binary.

use std::path::PathBuf;

use anyhow::{anyhow, Result};

fn resolve_socket_from_env() -> Option<PathBuf> {
    std::env::var_os("PRIME_AGENT_DAEMON_SOCKET")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn main() -> Result<()> {
    // Cap per-thread allocator arenas before any threads spawn.
    pa_types::memory_release::cap_thread_arenas();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("worker") {
        pa_daemon::worker::strip_inherited_worker_env();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main(args))
}

async fn async_main(args: Vec<String>) -> Result<()> {
    let mut iter = args.into_iter().peekable();
    let first = iter.next().unwrap_or_else(|| "supervisor".to_string());
    match first.as_str() {
        "supervisor" => run_supervisor_args(iter).await,
        "--mode" => match iter.next().as_deref() {
            Some("daemon") => run_supervisor_args(iter).await,
            other => Err(anyhow!("unsupported --mode value: {other:?}")),
        },
        "worker" => pa_daemon::worker::run_worker().await,
        other => Err(anyhow!(
            "unknown command: {other} (expected supervisor|worker)"
        )),
    }
}

async fn run_supervisor_args(
    mut args: std::iter::Peekable<std::vec::IntoIter<String>>,
) -> Result<()> {
    let mut socket_path = None;
    let mut agent_dir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" | "--daemon-socket" => socket_path = args.next().map(PathBuf::from),
            "--agent-dir" => agent_dir = args.next().map(PathBuf::from),
            other => return Err(anyhow!("unknown supervisor option: {other}")),
        }
    }
    let agent_dir = match agent_dir {
        Some(dir) => dir,
        None => pa_daemon::paths::agent_dir()?,
    };
    let options = pa_daemon::supervisor::SupervisorOptions {
        socket_path: socket_path
            .or_else(resolve_socket_from_env)
            .unwrap_or_else(pa_daemon::socket::default_daemon_socket_path),
        agent_dir,
    };
    pa_daemon::supervisor::run_supervisor(options).await
}
