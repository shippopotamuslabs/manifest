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

use manifest_domain::{
    ApprovalDecision, Connection, EngineEvent, FailedCommand, PendingAnswer, PendingResolution,
    SessionId, SpawnChoices, TurnId, TurnInput,
};
use serde_json::Value;

use crate::error::Error;
use crate::map::{
    history_truncation_diagnostic, map_ack_result, map_history_result, map_models_result,
    map_pending_result, map_resume_result, map_skills_result, map_spawn_result, map_status_result,
    map_stream_event, map_turn_result,
};
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
    SpawnSession {
        workspace: String,
        input: TurnInput,
        choices: SpawnChoices,
    },
    /// Send a follow-up turn: `muse_turn` with `wait: false`.
    SendTurn {
        session: SessionId,
        input: TurnInput,
    },
    /// Resume a session: `muse_models` by session id (catalog discarded).
    ResumeSession { session: SessionId },
    /// Inject a prompt into the running turn: `muse_steer`.
    SteerTurn {
        session: SessionId,
        prompt: String,
        turn: TurnId,
    },
    /// Cancel the running turn: `muse_cancel`, optionally naming the turn.
    CancelTurn {
        session: SessionId,
        turn: Option<TurnId>,
    },
    /// Compact the session: `muse_raw` with `session/compact`.
    CompactSession { session: SessionId },
    /// Set the session's goal: `muse_raw` with `goal/set`.
    SetGoal {
        session: SessionId,
        objective: String,
    },
    /// List the host model catalog: `muse_models` (read-only query).
    /// Carries the host context the server needs: a workspace, a
    /// session id, or both.
    ListModels {
        workspace: Option<String>,
        session: Option<SessionId>,
    },
    /// List the host skill catalog: `muse_skills` for one session.
    ListSkills { session: SessionId },
    /// Fetch the session's stored history: `muse_raw` with `session/read`.
    LoadHistory { session: SessionId },
    /// Fetch pending approvals/questions: `muse_pending`.
    FetchPending,
    /// Decide a pending approval: `muse_decide` as `actor: "user"`.
    DecidePending {
        session: SessionId,
        request_id: String,
        decision: ApprovalDecision,
        reason: Option<String>,
    },
    /// Answer a pending question set: `muse_answer` as `actor: "user"`.
    AnswerPending {
        session: SessionId,
        request_id: String,
        answer: PendingAnswer,
    },
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
    Resume(SessionId),
    Steer(SessionId),
    Cancel(SessionId),
    Compact(SessionId),
    Goal(SessionId),
    Models,
    Skills(SessionId),
    History(SessionId),
    Fetch,
    Decide {
        session: SessionId,
        request_id: String,
        decision: ApprovalDecision,
        reason: Option<String>,
    },
    Answer {
        session: SessionId,
        request_id: String,
        answer: PendingAnswer,
    },
}

/// The [`FailedCommand`] tag for a pending request: the failure of
/// this request belongs to exactly this command.
fn failed_command(pending: &Pending) -> Option<FailedCommand> {
    match pending {
        Pending::Spawn => Some(FailedCommand::Spawn),
        Pending::Turn(session) => Some(FailedCommand::Turn(session.clone())),
        Pending::Resume(session) => Some(FailedCommand::Resume(session.clone())),
        Pending::Steer(session) => Some(FailedCommand::Steer(session.clone())),
        Pending::Cancel(session) => Some(FailedCommand::Cancel(session.clone())),
        Pending::Compact(session) => Some(FailedCommand::Compact(session.clone())),
        Pending::Goal(session) => Some(FailedCommand::Goal(session.clone())),
        Pending::Models => Some(FailedCommand::Models),
        Pending::Skills(session) => Some(FailedCommand::Skills(session.clone())),
        Pending::History(session) => Some(FailedCommand::History(session.clone())),
        Pending::Fetch => Some(FailedCommand::PendingFetch),
        Pending::Decide {
            session,
            request_id,
            ..
        } => Some(FailedCommand::Decide(session.clone(), request_id.clone())),
        Pending::Answer {
            session,
            request_id,
            ..
        } => Some(FailedCommand::Answer(session.clone(), request_id.clone())),
        Pending::Initialize | Pending::ListSessions => None,
    }
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
    fn send_spawn_call(
        &mut self,
        workspace: &str,
        input: &TurnInput,
        choices: &SpawnChoices,
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Spawn);
        self.send_line(&wire::spawn_call(id, workspace, input, choices))
    }

    /// Issue a `muse_models` session-resume round trip with a fresh id.
    fn send_resume_call(&mut self, session: &SessionId) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Resume(session.clone()));
        self.send_line(&wire::resume_call(id, session.as_str()))
    }

    /// Issue a `muse_turn` round trip with a fresh id.
    fn send_turn_call(&mut self, session: &SessionId, input: &TurnInput) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Turn(session.clone()));
        self.send_line(&wire::turn_call(id, session.as_str(), input))
    }

    /// Issue a `muse_steer` round trip with a fresh id.
    fn send_steer_call(
        &mut self,
        session: &SessionId,
        prompt: &str,
        turn: &TurnId,
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Steer(session.clone()));
        self.send_line(&wire::steer_call(
            id,
            session.as_str(),
            prompt,
            turn.as_str(),
        ))
    }

    /// Issue a `muse_cancel` round trip with a fresh id.
    fn send_cancel_call(
        &mut self,
        session: &SessionId,
        turn: Option<&TurnId>,
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Cancel(session.clone()));
        self.send_line(&wire::cancel_call(
            id,
            session.as_str(),
            turn.map(TurnId::as_str),
        ))
    }

    /// Issue a `session/compact` round trip through `muse_raw` with a
    /// fresh id.
    fn send_compact_call(&mut self, session: &SessionId) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Compact(session.clone()));
        self.send_line(&wire::compact_call(id, session.as_str()))
    }

    /// Issue a `goal/set` round trip through `muse_raw` with a fresh id.
    fn send_goal_call(&mut self, session: &SessionId, objective: &str) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Goal(session.clone()));
        self.send_line(&wire::goal_call(id, session.as_str(), objective))
    }

    /// Issue a `muse_pending` round trip with a fresh id.
    fn send_pending_call(&mut self) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Fetch);
        self.send_line(&wire::pending_call(id))
    }

    /// Issue a `muse_decide` round trip with a fresh id.
    fn send_decide_call(
        &mut self,
        session: &SessionId,
        request_id: &str,
        decision: ApprovalDecision,
        reason: Option<&str>,
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(
            id,
            Pending::Decide {
                session: session.clone(),
                request_id: request_id.to_string(),
                decision,
                reason: reason.map(String::from),
            },
        );
        self.send_line(&wire::decide_call(id, request_id, decision, reason))
    }

    /// Issue a `muse_answer` round trip with a fresh id.
    fn send_answer_call(
        &mut self,
        session: &SessionId,
        request_id: &str,
        answer: &PendingAnswer,
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(
            id,
            Pending::Answer {
                session: session.clone(),
                request_id: request_id.to_string(),
                answer: answer.clone(),
            },
        );
        self.send_line(&wire::answer_call(id, request_id, answer))
    }

    /// Issue a `muse_models` catalog round trip with a fresh id.
    fn send_models_call(
        &mut self,
        workspace: Option<&str>,
        session: Option<&str>,
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Models);
        self.send_line(&wire::models_call(id, workspace, session))
    }

    /// Issue a `muse_skills` catalog round trip with a fresh id.
    fn send_skills_call(&mut self, session: &SessionId) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::Skills(session.clone()));
        self.send_line(&wire::skills_call(id, session.as_str()))
    }

    /// Issue a `session/read` round trip through `muse_raw` with a
    /// fresh id.
    fn send_history_call(&mut self, session: &SessionId) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(id, Pending::History(session.clone()));
        self.send_line(&wire::history_call(id, session.as_str()))
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
            Pending::Spawn
            | Pending::Turn(_)
            | Pending::Resume(_)
            | Pending::Steer(_)
            | Pending::Cancel(_)
            | Pending::Compact(_)
            | Pending::Goal(_)
            | Pending::Models
            | Pending::Skills(_)
            | Pending::History(_)
            | Pending::Fetch
            | Pending::Decide { .. }
            | Pending::Answer { .. } => self.handle_command_reply(&pending, message),
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

    /// Handle the reply to a `muse_spawn`, `muse_turn`, `muse_steer`,
    /// `muse_cancel`, `muse_raw` (compact, goal, history),
    /// `muse_models`, or resume call.
    ///
    /// The connection stays up: every outcome is a value the shell
    /// renders, mirroring [`Worker::handle_status_reply`]. Steer and
    /// cancel successes carry no payload worth rendering, so they emit
    /// nothing; only their failures arrive as [`EngineEvent::CommandFailed`].
    fn handle_command_reply(&mut self, pending: &Pending, message: &Value) -> bool {
        let Some(command) = failed_command(pending) else {
            return true;
        };
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Couldn't reach the Muse MCP server.");
            return self.emit(EngineEvent::CommandFailed {
                command,
                message: text.to_string(),
            });
        }
        let result = message.get("result").cloned().unwrap_or(Value::Null);
        let event = match pending {
            // History carries partial-success diagnostics alongside
            // the loaded event, so it folds in its own reply handler.
            Pending::History(session) => return self.handle_history_reply(session, message),
            Pending::Spawn => match map_spawn_result(&result) {
                Ok((session, turn)) => EngineEvent::SessionSpawned { session, turn },
                Err(message) => EngineEvent::CommandFailed { command, message },
            },
            Pending::Turn(commanded) => match map_turn_result(&result, commanded) {
                Ok((session, turn)) => EngineEvent::TurnAccepted { session, turn },
                Err(message) => EngineEvent::CommandFailed { command, message },
            },
            Pending::Resume(commanded) => match map_resume_result(&result, commanded) {
                Ok(session) => EngineEvent::SessionResumed { session },
                Err(message) => EngineEvent::CommandFailed { command, message },
            },
            Pending::Steer(_) => {
                match map_ack_result(&result, "Couldn't steer that turn on the Muse MCP server.") {
                    Ok(()) => return true,
                    Err(message) => EngineEvent::CommandFailed { command, message },
                }
            }
            Pending::Cancel(_) => {
                match map_ack_result(&result, "Couldn't cancel that turn on the Muse MCP server.") {
                    Ok(()) => return true,
                    Err(message) => EngineEvent::CommandFailed { command, message },
                }
            }
            // Compaction and goal ack like cancel: success emits
            // nothing (the outcome streams in as events), failure
            // arrives as `CommandFailed` for the shell's transcript.
            Pending::Compact(_) => {
                match map_ack_result(
                    &result,
                    "Couldn't compact that session on the Muse MCP server.",
                ) {
                    Ok(()) => return true,
                    Err(message) => EngineEvent::CommandFailed { command, message },
                }
            }
            Pending::Goal(_) => {
                match map_ack_result(
                    &result,
                    "Couldn't set that session's goal on the Muse MCP server.",
                ) {
                    Ok(()) => return true,
                    Err(message) => EngineEvent::CommandFailed { command, message },
                }
            }
            Pending::Models => match map_models_result(&result) {
                Ok(models) => EngineEvent::ModelsLoaded { models },
                Err(message) => EngineEvent::CommandFailed { command, message },
            },
            Pending::Skills(commanded) => match map_skills_result(&result) {
                Ok(skills) => EngineEvent::SkillsLoaded {
                    session: commanded.clone(),
                    skills,
                },
                Err(message) => EngineEvent::CommandFailed { command, message },
            },
            Pending::Fetch => match map_pending_result(&result) {
                Ok((requests, diagnostics)) => {
                    for diagnostic in diagnostics {
                        if !self.emit(EngineEvent::Diagnostic(diagnostic)) {
                            return false;
                        }
                    }
                    EngineEvent::PendingLoaded { requests }
                }
                Err(message) => EngineEvent::CommandFailed { command, message },
            },
            Pending::Decide {
                session,
                request_id,
                decision,
                reason,
            } => {
                match map_ack_result(
                    &result,
                    "Couldn't decide that approval on the Muse MCP server.",
                ) {
                    Ok(()) => EngineEvent::PendingSettled {
                        session: session.clone(),
                        request_id: request_id.clone(),
                        resolution: match decision {
                            ApprovalDecision::Approve => PendingResolution::Approved,
                            ApprovalDecision::Deny => PendingResolution::Denied(
                                reason.clone().filter(|text| !text.trim().is_empty()),
                            ),
                        },
                    },
                    Err(message) => EngineEvent::CommandFailed { command, message },
                }
            }
            Pending::Answer {
                session,
                request_id,
                answer,
            } => {
                match map_ack_result(
                    &result,
                    "Couldn't answer that question on the Muse MCP server.",
                ) {
                    Ok(()) => EngineEvent::PendingSettled {
                        session: session.clone(),
                        request_id: request_id.clone(),
                        resolution: PendingResolution::Answered(answer.summary()),
                    },
                    Err(message) => EngineEvent::CommandFailed { command, message },
                }
            }
            Pending::Initialize | Pending::ListSessions => return true,
        };
        self.emit(event)
    }

    /// Handle the reply to a `session/read` round trip. The
    /// connection stays up: failures arrive as
    /// `CommandFailed(History)` for the shell's transcript. Partial
    /// success rides as values first (one diagnostic per
    /// degradation, mirroring [`Worker::handle_status_reply`]), then
    /// the loaded event the shell seeds from.
    fn handle_history_reply(&mut self, session: &SessionId, message: &Value) -> bool {
        let result = message.get("result").cloned().unwrap_or(Value::Null);
        match map_history_result(&result) {
            Ok(page) => {
                for diagnostic in page.diagnostics {
                    if !self.emit(EngineEvent::Diagnostic(diagnostic)) {
                        return false;
                    }
                }
                if let Some(reason) = page.none_reason
                    && let Some(diagnostic) = history_truncation_diagnostic(&reason)
                    && !self.emit(EngineEvent::Diagnostic(diagnostic))
                {
                    return false;
                }
                self.emit(EngineEvent::HistoryLoaded {
                    session: session.clone(),
                    items: page.items,
                })
            }
            Err(message) => self.emit(EngineEvent::CommandFailed {
                command: FailedCommand::History(session.clone()),
                message,
            }),
        }
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
            WorkerMsg::Command(Command::SpawnSession {
                workspace,
                input,
                choices,
            }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Spawn,
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_spawn_call(&workspace, &input, &choices) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::SendTurn { session, input }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Turn(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_turn_call(&session, &input) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::ResumeSession { session }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Resume(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_resume_call(&session) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::SteerTurn {
                session,
                prompt,
                turn,
            }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Steer(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_steer_call(&session, &prompt, &turn) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::CancelTurn { session, turn }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Cancel(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_cancel_call(&session, turn.as_ref()) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::CompactSession { session }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Compact(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_compact_call(&session) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::SetGoal { session, objective }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Goal(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_goal_call(&session, &objective) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::FetchPending) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::PendingFetch,
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_pending_call() {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::DecidePending {
                session,
                request_id,
                decision,
                reason,
            }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Decide(session.clone(), request_id.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) =
                    worker.send_decide_call(&session, &request_id, decision, reason.as_deref())
                {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::AnswerPending {
                session,
                request_id,
                answer,
            }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Answer(session.clone(), request_id.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_answer_call(&session, &request_id, &answer) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::ListModels { workspace, session }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Models,
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_models_call(
                    workspace.as_deref(),
                    session.as_ref().map(SessionId::as_str),
                ) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::ListSkills { session }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::Skills(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_skills_call(&session) {
                    let _ = worker.fail(err);
                    false
                } else {
                    true
                }
            }
            WorkerMsg::Command(Command::LoadHistory { session }) => {
                if !worker.connected {
                    worker.emit(EngineEvent::CommandFailed {
                        command: FailedCommand::History(session.clone()),
                        message: not_connected_message(),
                    })
                } else if let Err(err) = worker.send_history_call(&session) {
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
