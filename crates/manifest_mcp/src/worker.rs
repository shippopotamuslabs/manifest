//! The blocking worker thread that owns the child server process.
//!
//! One worker thread owns the child: it is the only writer to the
//! child's stdin, and a single reader thread forwards stdout lines into
//! the same [`WorkerMsg`] channel the worker consumes commands from.
//! Request ids are `u64` starting at 1 (`initialize`) and incrementing;
//! a map tracks whether each id awaits the initialize reply or a
//! `muse_status` reply.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command as ProcCommand, Stdio};
use std::sync::mpsc::{Receiver, Sender};

use manifest_domain::{Connection, EngineEvent, SessionId};
use serde_json::Value;

use crate::error::Error;
use crate::map::{map_spawn_result, map_status_result, map_stream_event, map_turn_result};
use crate::{ServerConfig, wire};

/// Message when a spawn or turn command arrives before the connection.
fn not_connected_message() -> String {
    String::from("The Muse MCP server isn't connected yet. Try again once it connects.")
}

/// Commands the [`crate::MuseEngine`] handle can send the worker.
#[derive(Debug)]
pub(crate) enum Command {
    /// Request a `muse_status` round trip (or queue one until connected).
    ListSessions,
    /// Spawn a session: `muse_spawn` with `wait: false`.
    SpawnSession { workspace: String, prompt: String },
    /// Send a follow-up turn: `muse_turn` with `wait: false`.
    SendTurn { session: SessionId, prompt: String },
    /// Shut down: kill the child and exit without emitting.
    Shutdown,
}

/// Everything the worker's main loop consumes.
#[derive(Debug)]
pub(crate) enum WorkerMsg {
    Command(Command),
    Line(String),
    ReaderEof,
    ReaderFailed(String),
}

/// What one in-flight request id is waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Initialize,
    ListSessions,
    Spawn,
    Turn(SessionId),
}

/// Owns the child process; killing and reaping it on drop so every exit
/// path terminates the server.
struct ChildGuard {
    inner: Option<Child>,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.inner.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Worker state for one connection attempt.
struct Worker {
    events: Sender<EngineEvent>,
    stdin: ChildStdin,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    connected: bool,
    list_on_connect: bool,
}

impl Worker {
    /// Emit one event. False when the receiver is gone: the caller shuts down.
    fn emit(&self, event: EngineEvent) -> bool {
        self.events.send(event).is_ok()
    }

    /// Emit a connection failure composed from a typed boundary error.
    fn fail(&self, err: Error) -> bool {
        self.emit(EngineEvent::ConnectionChanged(Connection::Failed(
            err.to_string(),
        )))
    }

    /// Write one compact-JSON line to the server's stdin.
    fn send_line(&mut self, line: &str) -> Result<(), Error> {
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(Error::Write)
    }

    /// Issue a `muse_status` round trip with a fresh id.
    fn send_status_call(&mut self) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::ListSessions);
        self.send_line(&wire::status_call(id))
    }

    /// Issue a `muse_spawn` round trip with a fresh id.
    fn send_spawn_call(&mut self, workspace: &str, prompt: &str) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Spawn);
        self.send_line(&wire::spawn_call(id, workspace, prompt))
    }

    /// Issue a `muse_turn` round trip with a fresh id.
    fn send_turn_call(&mut self, session: &SessionId, prompt: &str) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Turn(session.clone()));
        self.send_line(&wire::turn_call(id, session.as_str(), prompt))
    }

    /// Handle one stdout line. `Ok(true)` keeps looping, `Ok(false)`
    /// shuts the worker down.
    fn handle_line(&mut self, line: &str) -> bool {
        if line.trim().is_empty() {
            return true;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(err) => {
                let _ = self.fail(Error::Malformed(format!(
                    "could not parse a line from the MCP server: {err}"
                )));
                return false;
            }
        };
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id");
        match (method, id) {
            (Some(_), Some(id)) => self.handle_server_request(id),
            (Some(method), None) => self.handle_notification(method, &message),
            (None, Some(_)) => self.handle_response(&message),
            // No method and no id: not a request, response, or
            // notification we recognize; ignore it.
            (None, None) => true,
        }
    }

    /// Reply `-32601` to a server request so none is left hanging.
    fn handle_server_request(&mut self, id: &Value) -> bool {
        if let Err(err) = self.send_line(&wire::method_not_supported(id)) {
            let _ = self.fail(err);
            return false;
        }
        true
    }

    /// Handle a server notification (no id).
    fn handle_notification(&mut self, method: &str, message: &Value) -> bool {
        if method != "notifications/muse/event" {
            return true;
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match map_stream_event(&params) {
            Ok(Some(event)) => self.emit(EngineEvent::Stream(event)),
            Ok(None) => true,
            Err(detail) => self.emit(EngineEvent::Diagnostic(detail)),
        }
    }

    /// Handle a response (id, result or error, no method).
    fn handle_response(&mut self, message: &Value) -> bool {
        if message.get("result").is_none() && message.get("error").is_none() {
            return true;
        }
        let id = match message.get("id").and_then(Value::as_u64) {
            Some(id) => id,
            // Non-numeric or unknown ids are ignored.
            None => return true,
        };
        let pending = match self.pending.remove(&id) {
            Some(pending) => pending,
            None => return true,
        };
        match pending {
            Pending::Initialize => self.handle_initialize_reply(message),
            Pending::ListSessions => self.handle_status_reply(message),
            Pending::Spawn | Pending::Turn(_) => self.handle_command_reply(&pending, message),
        }
    }

    /// Handle the reply to `initialize`.
    fn handle_initialize_reply(&mut self, message: &Value) -> bool {
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            let _ = self.fail(Error::Rpc {
                message: text.to_string(),
            });
            return false;
        }
        let is_object = message
            .get("result")
            .is_some_and(|result| result.is_object());
        if !is_object {
            let _ = self.fail(Error::Malformed(String::from(
                "the initialize result was not an object",
            )));
            return false;
        }
        if let Err(err) = self.send_line(&wire::initialized_notification()) {
            let _ = self.fail(err);
            return false;
        }
        self.connected = true;
        if !self.emit(EngineEvent::ConnectionChanged(Connection::Connected)) {
            return false;
        }
        if self.list_on_connect {
            self.list_on_connect = false;
            if let Err(err) = self.send_status_call() {
                let _ = self.fail(err);
                return false;
            }
        }
        true
    }

    /// Handle the reply to a `muse_spawn` or `muse_turn` call.
    ///
    /// The connection stays up: every outcome is a value the shell
    /// renders, mirroring [`Worker::handle_status_reply`].
    fn handle_command_reply(&mut self, pending: &Pending, message: &Value) -> bool {
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Couldn't reach the Muse MCP server.");
            return self.emit(EngineEvent::CommandFailed(text.to_string()));
        }
        let result = message.get("result").cloned().unwrap_or(Value::Null);
        let event = match pending {
            Pending::Spawn => match map_spawn_result(&result) {
                Ok((session, turn)) => EngineEvent::SessionSpawned { session, turn },
                Err(message) => EngineEvent::CommandFailed(message),
            },
            Pending::Turn(commanded) => match map_turn_result(&result, commanded) {
                Ok((session, turn)) => EngineEvent::TurnAccepted { session, turn },
                Err(message) => EngineEvent::CommandFailed(message),
            },
            Pending::Initialize | Pending::ListSessions => return true,
        };
        self.emit(event)
    }

    /// Handle the reply to a `muse_status` call. The connection stays up.
    fn handle_status_reply(&mut self, message: &Value) -> bool {
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Couldn't load your sessions from the Muse MCP server.");
            return self.emit(EngineEvent::SessionsLoadFailed(text.to_string()));
        }
        let result = message.get("result").cloned().unwrap_or(Value::Null);
        let mut keep_going = true;
        // Diagnostics first, then SessionsLoaded: partial success rides as
        // values, so the shell can show what was skipped before the rows.
        for event in map_status_result(&result) {
            keep_going = self.emit(event);
            if !keep_going {
                break;
            }
        }
        keep_going
    }
}

/// Read child stdout lines and forward them into the worker's channel.
fn spawn_reader(stdout: ChildStdout, worker_tx: Sender<WorkerMsg>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if worker_tx.send(WorkerMsg::Line(line)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = worker_tx.send(WorkerMsg::ReaderFailed(err.to_string()));
                    return;
                }
            }
        }
        let _ = worker_tx.send(WorkerMsg::ReaderEof);
    });
}

/// Worker entry point: spawn the child, run initialize, serve the loop.
///
/// Every exit path drops the [`ChildGuard`], which kills and reaps the
/// server. Events stop as soon as the receiver is gone.
pub(crate) fn run(
    config: ServerConfig,
    events: Sender<EngineEvent>,
    worker_rx: Receiver<WorkerMsg>,
    worker_tx: Sender<WorkerMsg>,
) {
    if events
        .send(EngineEvent::ConnectionChanged(Connection::Connecting))
        .is_err()
    {
        return;
    }
    let mut command = ProcCommand::new(&config.command);
    command
        .args(&config.args)
        .envs(config.envs.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Dev tool: forward server stderr logging, per the MCP spec.
        .stderr(Stdio::inherit());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = events.send(EngineEvent::ConnectionChanged(Connection::Failed(
                Error::Spawn {
                    command: config.command,
                    source: err,
                }
                .to_string(),
            )));
            return;
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    // Lives until `run` returns, so every exit path kills the server.
    let _child_guard = ChildGuard { inner: Some(child) };
    let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
        let _ = events.send(EngineEvent::ConnectionChanged(Connection::Failed(
            Error::Malformed(String::from("the MCP server did not open its stdio pipes"))
                .to_string(),
        )));
        return;
    };
    spawn_reader(stdout, worker_tx);

    let mut worker = Worker {
        events,
        stdin,
        next_id: 2,
        pending: HashMap::from([(1, Pending::Initialize)]),
        connected: false,
        list_on_connect: false,
    };
    if let Err(err) = worker.send_line(&wire::initialize_request(1)) {
        let _ = worker.fail(err);
        return;
    }

    loop {
        let message = match worker_rx.recv() {
            Ok(message) => message,
            // The handle (and reader) are gone; the shell moved on.
            Err(_) => return,
        };
        let keep_going = match message {
            WorkerMsg::Command(Command::Shutdown) => false,
            WorkerMsg::Command(Command::ListSessions) => {
                if !worker.connected {
                    worker.list_on_connect = true;
                    true
                } else if let Err(err) = worker.send_status_call() {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::SpawnSession { workspace, prompt }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed(not_connected_message()))
                } else if let Err(err) = worker.send_spawn_call(&workspace, &prompt) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::SendTurn { session, prompt }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed(not_connected_message()))
                } else if let Err(err) = worker.send_turn_call(&session, &prompt) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Line(line) => worker.handle_line(&line),
            WorkerMsg::ReaderEof => {
                let _ = worker.fail(Error::Closed {
                    before_connect: !worker.connected,
                });
                false
            }
            WorkerMsg::ReaderFailed(detail) => {
                let _ = worker.fail(Error::Read(detail));
                false
            }
        };
        if !keep_going {
            return;
        }
    }
}
