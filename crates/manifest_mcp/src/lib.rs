//! Data-layer adapter for the muse-code-mcp server over MCP stdio.
//!
//! This crate spawns the `muse-code-mcp` server as a child process and
//! speaks JSON-RPC 2.0 with it over newline-delimited JSON on its stdio:
//! `initialize` declaring the `muse.stream` experimental capability,
//! `tools/call` for `muse_status`, `muse_spawn` (workspace, prompt,
//! worktree choices, `wait: false`), `muse_turn`, and `muse_models`
//! (session resume only; catalog discarded), and
//! `notifications/muse/event` mapped into domain
//! [`StreamEvent`](manifest_domain::StreamEvent) values. The client is
//! blocking and thread-based; there is no async runtime in this crate.
//!
//! Wire types stay private to this crate. The public surface is
//! [`ServerConfig`], [`MuseEngine`], and [`connect`].

mod error;
mod map;
mod wire;
mod worker;

use std::sync::mpsc::Sender;

use manifest_domain::{EngineEvent, SessionEngine, SessionId, SpawnChoices};

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
    /// The command resolves in this order (first hit wins):
    ///
    /// 1. `MANIFEST_MUSE_MCP_BIN` when set, returned verbatim even if
    ///    the file does not exist (an explicit user choice is never
    ///    second-guessed; the spawn error path explains a bad path).
    /// 2. `muse-code-mcp/bin/muse-mcp` next to the running binary
    ///    (installed sidecar layout).
    /// 3. `../share/manifest/muse-code-mcp/bin/muse-mcp` relative to
    ///    the running binary (prefix layout: `$PREFIX/bin/manifest`
    ///    with the payload under `$PREFIX/share/manifest/`).
    /// 4. `bundled/muse-code-mcp/bin/muse-mcp` in a dev checkout
    ///    (relative to `CARGO_MANIFEST_DIR`; absent on machines
    ///    without the checkout, so the candidate is simply skipped).
    /// 5. `muse-mcp` resolved from `PATH`.
    ///
    /// Args and envs are empty.
    pub fn from_env() -> Self {
        Self::new(Self::resolve_command(
            std::env::var("MANIFEST_MUSE_MCP_BIN").ok(),
            std::env::current_exe()
                .ok()
                .as_deref()
                .and_then(|p| p.parent()),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            std::path::Path::is_file,
        ))
    }

    /// Resolve the server command from its candidates.
    ///
    /// Pure over its inputs so the resolution order is unit-testable;
    /// [`ServerConfig::from_env`] feeds it the real values.
    fn resolve_command(
        env_bin: Option<String>,
        exe_dir: Option<&std::path::Path>,
        manifest_dir: &std::path::Path,
        is_file: impl Fn(&std::path::Path) -> bool,
    ) -> String {
        if let Some(bin) = env_bin {
            return bin;
        }
        if let Some(dir) = exe_dir {
            let sidecar = dir.join("muse-code-mcp/bin/muse-mcp");
            if is_file(&sidecar) {
                return sidecar.to_string_lossy().into_owned();
            }
            let shared = dir.join("../share/manifest/muse-code-mcp/bin/muse-mcp");
            if is_file(&shared) {
                return shared.to_string_lossy().into_owned();
            }
        }
        let dev = manifest_dir.join("../../bundled/muse-code-mcp/bin/muse-mcp");
        if is_file(&dev) {
            return dev.to_string_lossy().into_owned();
        }
        String::from("muse-mcp")
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

    fn spawn_session(&self, workspace: String, prompt: String, choices: SpawnChoices) {
        // A failed send means the worker is gone; the shell has moved on.
        let _ = self
            .worker_tx
            .send(WorkerMsg::Command(Command::SpawnSession {
                workspace,
                prompt,
                choices,
            }));
    }

    fn send_turn(&self, session: SessionId, prompt: String) {
        // A failed send means the worker is gone; the shell has moved on.
        let _ = self
            .worker_tx
            .send(WorkerMsg::Command(Command::SendTurn { session, prompt }));
    }

    fn resume_session(&self, session: SessionId) {
        // A failed send means the worker is gone; the shell has moved on.
        let _ = self
            .worker_tx
            .send(WorkerMsg::Command(Command::ResumeSession { session }));
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

#[cfg(test)]
mod resolve_command_tests {
    use super::ServerConfig;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn resolve(
        env_bin: Option<&str>,
        exe_dir: Option<&str>,
        manifest_dir: &str,
        existing: &[&str],
    ) -> String {
        let present: HashSet<PathBuf> = existing.iter().copied().map(PathBuf::from).collect();
        ServerConfig::resolve_command(
            env_bin.map(String::from),
            exe_dir.map(Path::new),
            Path::new(manifest_dir),
            |p| present.contains(p),
        )
    }

    fn exe_paths(exe_dir: &str, manifest_dir: &str) -> (String, String, String) {
        let dir = Path::new(exe_dir);
        (
            dir.join("muse-code-mcp/bin/muse-mcp")
                .to_string_lossy()
                .into_owned(),
            dir.join("../share/manifest/muse-code-mcp/bin/muse-mcp")
                .to_string_lossy()
                .into_owned(),
            Path::new(manifest_dir)
                .join("../../bundled/muse-code-mcp/bin/muse-mcp")
                .to_string_lossy()
                .into_owned(),
        )
    }

    #[test]
    fn env_wins_over_everything() {
        let (sidecar, shared, dev) = exe_paths("/prefix/bin", "/repo/crates/manifest_mcp");
        let got = resolve(
            Some("/custom/muse-mcp"),
            Some("/prefix/bin"),
            "/repo/crates/manifest_mcp",
            &[&sidecar, &shared, &dev],
        );
        assert_eq!(got, "/custom/muse-mcp");
    }

    #[test]
    fn env_returned_verbatim_even_when_missing() {
        let got = resolve(Some("/does/not/exist"), Some("/prefix/bin"), "/repo", &[]);
        assert_eq!(got, "/does/not/exist");
    }

    #[test]
    fn sidecar_beats_share_beats_dev() {
        let (sidecar, shared, dev) = exe_paths("/prefix/bin", "/repo/crates/manifest_mcp");
        let got = resolve(
            None,
            Some("/prefix/bin"),
            "/repo/crates/manifest_mcp",
            &[&sidecar, &shared, &dev],
        );
        assert_eq!(got, sidecar);

        let got = resolve(
            None,
            Some("/prefix/bin"),
            "/repo/crates/manifest_mcp",
            &[&shared, &dev],
        );
        assert_eq!(got, shared);

        let got = resolve(
            None,
            Some("/prefix/bin"),
            "/repo/crates/manifest_mcp",
            &[&dev],
        );
        assert_eq!(got, dev);
    }

    #[test]
    fn dev_used_when_installed_candidates_absent() {
        let (_, _, dev) = exe_paths("/prefix/bin", "/repo/crates/manifest_mcp");
        let got = resolve(
            None,
            Some("/prefix/bin"),
            "/repo/crates/manifest_mcp",
            &[&dev],
        );
        assert_eq!(got, dev);
    }

    #[test]
    fn falls_back_to_path_lookup_when_nothing_exists() {
        let got = resolve(None, Some("/prefix/bin"), "/repo/crates/manifest_mcp", &[]);
        assert_eq!(got, "muse-mcp");
    }

    #[test]
    fn exe_dir_none_skips_installed_candidates() {
        let (_, _, dev) = exe_paths("/prefix/bin", "/repo/crates/manifest_mcp");
        let got = resolve(None, None, "/repo/crates/manifest_mcp", &[&dev]);
        assert_eq!(got, dev);

        let got = resolve(None, None, "/repo/crates/manifest_mcp", &[]);
        assert_eq!(got, "muse-mcp");
    }
}
