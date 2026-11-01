//! Data-layer adapter for the muse-code-mcp server over MCP stdio.
//!
//! This crate spawns the `muse-code-mcp` server as a child process and
//! speaks JSON-RPC 2.0 with it over newline-delimited JSON on its stdio:
//! `initialize` declaring the `muse.stream` experimental capability,
//! `tools/call` for `muse_status`, and `notifications/muse/event`
//! mapped into domain [`StreamEvent`](manifest_domain::StreamEvent)
//! values. The client is blocking and thread-based; there is no async
//! runtime in this crate.
//!
//! Wire types stay private to this crate. The public surface is
//! [`ServerConfig`], [`MuseEngine`], and [`connect`].

mod error;
mod map;
mod wire;
mod worker;

use std::sync::mpsc::Sender;

use manifest_domain::{EngineEvent, SessionEngine};

use worker::{Command, WorkerMsg};

/// How to launch the muse-code-mcp server process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Server binary to spawn (an absolute path or something on `PATH`).
    pub command: String,
    /// Extra argv entries passed to the server.
    pub args: Vec<String>,
    /// Extra environment variables set on the server process.
    pub envs: Vec<(String, String)>,
}

impl ServerConfig {
    /// Build a config that runs `command` with no extra args or env.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            envs: Vec::new(),
        }
    }

    /// Build a config from the environment.
    ///
    /// The command comes from `MANIFEST_MUSE_MCP_BIN` when set, and
    /// defaults to `muse-mcp` otherwise. Args and envs are empty.
    pub fn from_env() -> Self {
        let command =
            std::env::var("MANIFEST_MUSE_MCP_BIN").unwrap_or_else(|_| String::from("muse-mcp"));
        Self::new(command)
    }
}

/// Handle to one muse-code-mcp connection.
///
/// Created by [`connect`]. Command submission through
/// [`SessionEngine::list_sessions`] is non-blocking; results arrive as
/// [`EngineEvent`] values on the channel passed to [`connect`]. Dropping
/// the `MuseEngine` shuts the connection down and kills the child server
/// process.
pub struct MuseEngine {
    worker_tx: Sender<WorkerMsg>,
}

impl SessionEngine for MuseEngine {
    fn list_sessions(&self) {
        // A failed send means the worker is gone; the shell has moved on.
        let _ = self
            .worker_tx
            .send(WorkerMsg::Command(Command::ListSessions));
    }
}

impl Drop for MuseEngine {
    fn drop(&mut self) {
        // Ask the worker to kill the child and exit. A failed send means
        // the worker is already gone, so there is nothing left to stop.
        let _ = self.worker_tx.send(WorkerMsg::Command(Command::Shutdown));
    }
}

/// Connect to the muse-code-mcp server described by `config`.
///
/// Spawns the worker thread (which spawns the child server process)
/// immediately and returns the [`MuseEngine`] handle. Connection
/// progress and failure arrive as [`EngineEvent`] values on `events`;
/// the worker stops trying to emit once the receiver is dropped.
/// Dropping the returned [`MuseEngine`] (and any internal clones, which
/// only the worker holds) shuts the connection down and kills the child
/// server process.
pub fn connect(config: ServerConfig, events: Sender<EngineEvent>) -> MuseEngine {
    let (worker_tx, worker_rx) = std::sync::mpsc::channel::<WorkerMsg>();
    let engine_tx = worker_tx.clone();
    std::thread::spawn(move || worker::run(config, events, worker_rx, worker_tx));
    MuseEngine {
        worker_tx: engine_tx,
    }
}
