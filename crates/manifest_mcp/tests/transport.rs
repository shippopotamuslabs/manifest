//! Transport tests against the scripted fake server.

use manifest_domain::{
    AnswerValue, ApprovalDecision, Connection, EngineEvent, FailedCommand, ItemStatus,
    PendingAnswer, PendingKind, PendingPhase, PendingRequest, PendingResolution, SessionEngine,
    SessionId, SessionStatus, SpawnChoices, StreamEvent, StreamItem, TurnId, WorktreeChoice,
};
use manifest_mcp::{MuseEngine, ServerConfig, connect};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(15);

fn fake_config(mode: &str) -> ServerConfig {
    ServerConfig {
        command: env!("CARGO_BIN_EXE_fake_muse_mcp").to_string(),
        args: Vec::new(),
        envs: vec![(String::from("FAKE_MCP_MODE"), mode.to_string())],
    }
}

/// Receive events until `wants` matches, panicking with everything seen
/// so far when the wait times out.
fn recv_until(
    rx: &Receiver<EngineEvent>,
    label: &str,
    wants: impl Fn(&EngineEvent) -> bool,
) -> Vec<EngineEvent> {
    let mut seen = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(event) => {
                if wants(&event) {
                    seen.push(event);
                    return seen;
                }
                seen.push(event);
            }
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                panic!("timed out waiting for {label}; saw {seen:#?}");
            }
        }
    }
}

fn is_failed(event: &EngineEvent) -> bool {
    matches!(event, EngineEvent::ConnectionChanged(Connection::Failed(_)))
}

fn failed_text(events: &[EngineEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        EngineEvent::ConnectionChanged(Connection::Failed(text)) => Some(text.clone()),
        _ => None,
    })
}

/// The tagged command failure among `events`, when one arrived: the
/// command it names plus its message. The tag is the durable proof
/// the worker attributes the failure to the command that issued it.
fn command_failure(events: &[EngineEvent]) -> Option<(FailedCommand, String)> {
    events.iter().find_map(|event| match event {
        EngineEvent::CommandFailed { command, message } => Some((command.clone(), message.clone())),
        _ => None,
    })
}

/// Connect in `ok` mode and drive the worker through its first
/// `muse_status` round trip, returning everything seen plus the engine.
fn connected_with_sessions(rx: &Receiver<EngineEvent>) -> Vec<EngineEvent> {
    let seen = recv_until(rx, "Connected", |event| {
        matches!(event, EngineEvent::ConnectionChanged(Connection::Connected))
    });
    assert_eq!(
        seen.first(),
        Some(&EngineEvent::ConnectionChanged(Connection::Connecting)),
        "Connecting comes first"
    );
    seen
}

#[test]
fn connects_and_lists_sessions() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine: MuseEngine = connect(fake_config("ok"), tx);

    let mut seen = connected_with_sessions(&rx);

    // The burst: an item delta, then (after two ignored events) a pending hint.
    seen.extend(recv_until(&rx, "item delta", |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ItemDelta { delta, .. })
            if delta == "hello"
        )
    }));
    let delta = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemDelta {
            session,
            turn,
            view_cursor,
            item_id,
            field,
            delta,
        }) => Some((session, turn, view_cursor, item_id, field, delta)),
        _ => None,
    });
    let (session, turn, view_cursor, item_id, field, delta) = delta.expect("delta seen above");
    assert_eq!(session, &SessionId::new("sess-run"));
    assert_eq!(turn.as_ref().map(|_| ()), Some(()));
    assert_eq!(view_cursor, &Some(String::from("7")));
    assert_eq!(item_id, "item-1");
    assert_eq!(field, "text");
    assert_eq!(delta, "hello");

    seen.extend(recv_until(&rx, "pending hint", |event| {
        matches!(event, EngineEvent::Stream(StreamEvent::Pending { .. }))
    }));
    let pending = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::Pending {
            session,
            request_id,
            kind,
            phase,
            ..
        }) => Some((session, request_id, kind, phase)),
        _ => None,
    });
    let (session, request_id, kind, phase) = pending.expect("pending seen above");
    assert_eq!(session, &SessionId::new("sess-pending"));
    assert_eq!(request_id, "req-1");
    assert_eq!(kind, &PendingKind::Approval);
    assert_eq!(phase, &PendingPhase::Requested);

    engine.list_sessions();
    seen.extend(recv_until(&rx, "SessionsLoaded", |event| {
        matches!(event, EngineEvent::SessionsLoaded { .. })
    }));

    let diagnostics: Vec<&str> = seen
        .iter()
        .filter_map(|event| match event {
            EngineEvent::Diagnostic(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        diagnostics
            .iter()
            .any(|text| text.contains("sess-weird") || text.contains("exploded")),
        "a diagnostic names the skipped row, saw {diagnostics:?}"
    );

    let loaded = seen.iter().find_map(|event| match event {
        EngineEvent::SessionsLoaded { sessions } => Some(sessions),
        _ => None,
    });
    let sessions = loaded.expect("SessionsLoaded seen above");
    let ids: Vec<SessionId> = sessions.iter().map(|s| s.id.clone()).collect();
    assert_eq!(
        ids,
        vec![
            SessionId::new("sess-run"),
            SessionId::new("sess-idle"),
            SessionId::new("sess-orphan"),
            SessionId::new("sess-pending"),
        ],
    );
    let statuses: Vec<SessionStatus> = sessions.iter().map(|s| s.status.clone()).collect();
    assert_eq!(
        statuses,
        vec![
            SessionStatus::Running,
            SessionStatus::Idle,
            SessionStatus::Orphaned,
            SessionStatus::AwaitingApproval,
        ],
    );
    // Titles are short ids now; grouping is the shell's job.
    assert_eq!(sessions[0].title, "sess-run");
    assert_eq!(sessions[0].stream_dropped_deltas, 3);
    assert_eq!(sessions[0].workspace, "/home/dev/checkout-api");
    assert_eq!(sessions[0].project, None);
    assert_eq!(sessions[2].status, SessionStatus::Orphaned);
    assert_eq!(sessions[2].workspace, "/home/dev/storefront-web");
    assert_eq!(sessions[2].title, "sess-orp");

    assert!(
        !seen.iter().any(is_failed),
        "no failure while listing, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn spawn_flows_into_typed_stream() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.spawn_session(
        String::from("/tmp/ws"),
        String::from("Read note.txt"),
        SpawnChoices::default(),
    );
    let mut seen = recv_until(&rx, "SessionSpawned", |event| {
        matches!(event, EngineEvent::SessionSpawned { .. })
    });
    let spawned = seen.iter().find_map(|event| match event {
        EngineEvent::SessionSpawned { session, turn } => Some((session, turn)),
        _ => None,
    });
    let (session, turn) = spawned.expect("SessionSpawned seen above");
    assert_eq!(session, &SessionId::new("fake-session-1"));
    assert_eq!(turn, &Some(TurnId::new("fake-turn-1")));

    seen.extend(recv_until(&rx, "spawn turn-completed", |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::TurnCompleted { turn, .. })
            if turn == &Some(TurnId::new("fake-turn-1"))
        )
    }));

    let user_text = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemCompleted {
            item: StreamItem::UserMessage { text, .. },
            ..
        }) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(user_text.as_deref(), Some("Read note.txt"));

    let reasoning = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemCompleted {
            item: StreamItem::Reasoning { summary, .. },
            ..
        }) => Some(summary.clone()),
        _ => None,
    });
    assert_eq!(
        reasoning,
        Some(vec![String::from(
            "Planning the turn, listing the workspace, and quoting the note."
        )])
    );

    let tool = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemCompleted {
            item:
                StreamItem::ToolCall {
                    tool,
                    headline,
                    output,
                    status,
                    ..
                },
            ..
        }) => Some((
            tool.clone(),
            headline.clone(),
            output.clone(),
            status.clone(),
        )),
        _ => None,
    });
    let (tool, headline, output, status) = tool.expect("completed tool call seen above");
    assert_eq!(tool, "bash");
    assert_eq!(
        headline.as_deref(),
        Some("List workspace directory contents")
    );
    assert_eq!(output.as_deref(), Some("total 0"));
    assert_eq!(status, ItemStatus::Completed);

    let answer = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemCompleted {
            item:
                StreamItem::AgentMessage {
                    text,
                    phase: Some(phase),
                    ..
                },
            ..
        }) if phase == "finalAnswer" => Some(text.clone()),
        _ => None,
    });
    let answer = answer.expect("completed agent message seen above");
    assert!(
        answer.contains("```\ntotal 0\n```"),
        "answer keeps its fenced code block, saw {answer:?}"
    );

    let close = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::TurnCompleted {
            terminal,
            answer,
            dropped_deltas,
            ..
        }) => Some((terminal.clone(), answer.clone(), *dropped_deltas)),
        _ => None,
    });
    let (terminal, closed_answer, dropped) = close.expect("turn-completed seen above");
    assert_eq!(terminal.as_deref(), Some("completed"));
    assert_eq!(closed_answer.as_deref(), Some(answer.as_str()));
    assert_eq!(dropped, 0);
    assert!(
        !seen.iter().any(is_failed),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn spawn_bad_workspace_is_command_failed() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.spawn_session(
        String::from("/tmp/does-not-exist/ws"),
        String::from("Read note.txt"),
        SpawnChoices::default(),
    );
    let seen = recv_until(&rx, "CommandFailed", |event| {
        matches!(event, EngineEvent::CommandFailed { .. })
    });
    let (command, failure) = command_failure(&seen).expect("failure seen above");
    assert_eq!(command, FailedCommand::Spawn);
    assert!(
        failure.contains("workspace does not exist: /tmp/does-not-exist/ws"),
        "failure carries the server text, saw {failure:?}"
    );
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn spawn_with_existing_worktree_succeeds_and_streams() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.spawn_session(
        String::from("/tmp/ws"),
        String::from("Read note.txt"),
        SpawnChoices {
            worktree: WorktreeChoice::Existing {
                path: String::from("/tmp/wt-1"),
            },
            ..Default::default()
        },
    );
    let seen = recv_until(&rx, "SessionSpawned", |event| {
        matches!(event, EngineEvent::SessionSpawned { .. })
    });
    let spawned = seen.iter().find_map(|event| match event {
        EngineEvent::SessionSpawned { session, turn } => Some((session, turn)),
        _ => None,
    });
    let (session, turn) = spawned.expect("SessionSpawned seen above");
    assert_eq!(session, &SessionId::new("fake-session-1"));
    assert_eq!(turn, &Some(TurnId::new("fake-turn-1")));
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn spawn_untrusted_create_is_command_failed() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.spawn_session(
        String::from("/untrusted"),
        String::from("Read note.txt"),
        SpawnChoices {
            worktree: WorktreeChoice::Create { base: None },
            ..Default::default()
        },
    );
    let seen = recv_until(&rx, "CommandFailed", |event| {
        matches!(event, EngineEvent::CommandFailed { .. })
    });
    let (command, failure) = command_failure(&seen).expect("failure seen above");
    assert_eq!(command, FailedCommand::Spawn);
    assert_eq!(
        failure,
        "Repository '/untrusted' is not trusted; worktree create, attach, and list require a trusted repository."
    );
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn resume_orphaned_session_reports_resumed() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.resume_session(SessionId::new("sess-orphan"));
    let seen = recv_until(&rx, "SessionResumed", |event| {
        matches!(event, EngineEvent::SessionResumed { .. })
    });
    let resumed = seen.iter().find_map(|event| match event {
        EngineEvent::SessionResumed { session } => Some(session),
        _ => None,
    });
    assert_eq!(resumed, Some(&SessionId::new("sess-orphan")));
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn resume_in_use_reports_server_text() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("resume-in-use"), tx);
    let _ = connected_with_sessions(&rx);

    engine.resume_session(SessionId::new("sess-orphan"));
    let seen = recv_until(&rx, "CommandFailed", |event| {
        matches!(event, EngineEvent::CommandFailed { .. })
    });
    let (command, failure) = command_failure(&seen).expect("failure seen above");
    assert_eq!(
        command,
        FailedCommand::Resume(SessionId::new("sess-orphan"))
    );
    assert_eq!(
        failure,
        "session sess-orphan is in use by another client (sessionInUse: already attached). Muse MCP will not attach to it."
    );
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn send_turn_flows_into_acceptance_and_stream() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.send_turn(SessionId::new("fake-session-1"), String::from("And again"));
    let mut seen = recv_until(&rx, "TurnAccepted", |event| {
        matches!(event, EngineEvent::TurnAccepted { .. })
    });
    let accepted = seen.iter().find_map(|event| match event {
        EngineEvent::TurnAccepted { session, turn } => Some((session, turn)),
        _ => None,
    });
    let (session, turn) = accepted.expect("TurnAccepted seen above");
    assert_eq!(session, &SessionId::new("fake-session-1"));
    assert_eq!(turn, &Some(TurnId::new("fake-turn-2")));

    seen.extend(recv_until(&rx, "turn turn-completed", |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::TurnCompleted { turn, .. })
            if turn == &Some(TurnId::new("fake-turn-2"))
        )
    }));
    let answer = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemCompleted {
            item: StreamItem::AgentMessage { text, .. },
            turn,
            ..
        }) if turn == &Some(TurnId::new("fake-turn-2")) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(answer.as_deref(), Some("Second turn done."));
    assert!(
        !seen.iter().any(is_failed),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn second_list_is_load_failed_not_disconnect() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.list_sessions();
    let mut seen = recv_until(&rx, "first SessionsLoaded", |event| {
        matches!(event, EngineEvent::SessionsLoaded { .. })
    });

    engine.list_sessions();
    seen.extend(recv_until(&rx, "SessionsLoadFailed", |event| {
        matches!(event, EngineEvent::SessionsLoadFailed(_))
    }));
    let failure = seen
        .iter()
        .find_map(|event| match event {
            EngineEvent::SessionsLoadFailed(text) => Some(text.clone()),
            _ => None,
        })
        .expect("failure seen above");
    assert!(
        failure.contains("status blew up"),
        "second call reports the server error, saw {failure:?}"
    );
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn pending_fetch_decide_answer_then_empty() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    // Fetch: one approval + one 3-question user-input.
    engine.fetch_pending();
    let mut seen = recv_until(&rx, "PendingLoaded", |event| {
        matches!(event, EngineEvent::PendingLoaded { .. })
    });
    let loaded = seen
        .iter()
        .find_map(|event| match event {
            EngineEvent::PendingLoaded { requests } => Some(requests.clone()),
            _ => None,
        })
        .expect("PendingLoaded seen above");
    assert_eq!(loaded.len(), 2, "saw {loaded:?}");
    match &loaded[0] {
        PendingRequest::Approval {
            session,
            request_id,
            approval,
        } => {
            assert_eq!(session, &SessionId::new("sess-pending"));
            assert_eq!(request_id, "req-approval-1");
            assert_eq!(approval.tool, "bash");
            assert!(approval.subject_line.contains("ls -la /tmp/ws"));
            assert_eq!(approval.choice_labels(), vec!["Allow once", "Reject"]);
        }
        other => panic!("first pending is the approval, saw {other:?}"),
    }
    match &loaded[1] {
        PendingRequest::Question {
            session,
            request_id,
            question,
        } => {
            assert_eq!(session, &SessionId::new("sess-pending"));
            assert_eq!(request_id, "req-question-1");
            assert_eq!(question.questions.len(), 3);
            assert!(!question.questions[0].multi);
            assert_eq!(
                question.questions[0].options[0].description.as_deref(),
                Some("Warm and bright")
            );
            // The real "None" option is a plain label with no description.
            assert_eq!(question.questions[0].options[2].label, "None");
            assert_eq!(question.questions[0].options[2].description, None);
            assert!(question.questions[1].multi, "second question is multiple");
            assert_eq!(question.questions[2].prompt, "Which file?");
        }
        other => panic!("second pending is the question, saw {other:?}"),
    }

    // Decide the approval as the user; it settles.
    engine.decide(
        SessionId::new("sess-pending"),
        String::from("req-approval-1"),
        ApprovalDecision::Approve,
        None,
    );
    seen.extend(recv_until(&rx, "approval PendingSettled", |event| {
        matches!(
            event,
            EngineEvent::PendingSettled { request_id, .. } if request_id == "req-approval-1"
        )
    }));
    let settled = seen.iter().find_map(|event| match event {
        EngineEvent::PendingSettled {
            session,
            request_id,
            resolution,
        } if request_id == "req-approval-1" => Some((session.clone(), resolution.clone())),
        _ => None,
    });
    assert_eq!(
        settled,
        Some((SessionId::new("sess-pending"), PendingResolution::Approved))
    );

    // Answer all three questions; it settles with a summary.
    engine.answer(
        SessionId::new("sess-pending"),
        String::from("req-question-1"),
        PendingAnswer::new(vec![
            (
                String::from("color"),
                AnswerValue::Label(String::from("Red")),
            ),
            (
                String::from("extras"),
                AnswerValue::Labels(vec![String::from("Cheese")]),
            ),
            (String::from("file"), AnswerValue::free_text("my own file")),
        ]),
    );
    seen.extend(recv_until(&rx, "question PendingSettled", |event| {
        matches!(
            event,
            EngineEvent::PendingSettled { request_id, .. } if request_id == "req-question-1"
        )
    }));
    let settled = seen.iter().find_map(|event| match event {
        EngineEvent::PendingSettled {
            request_id,
            resolution,
            ..
        } if request_id == "req-question-1" => Some(resolution.clone()),
        _ => None,
    });
    match settled {
        Some(PendingResolution::Answered(summary)) => {
            assert!(
                summary.contains("Red"),
                "summary names answers, saw {summary:?}"
            );
        }
        other => panic!("expected Answered settlement, saw {other:?}"),
    }

    // Both settled: a fresh fetch is empty (the fake drops settled rows,
    // and only settles when actor is "user").
    engine.fetch_pending();
    seen.extend(recv_until(
        &rx,
        "empty PendingLoaded",
        |event| matches!(event, EngineEvent::PendingLoaded { requests } if requests.is_empty()),
    ));
    assert!(
        !seen.iter().any(is_failed),
        "the connection stays up, saw {seen:#?}"
    );
    assert!(command_failure(&seen).is_none(), "saw {seen:#?}");
    drop(engine);
}

#[test]
fn decide_unknown_request_is_tagged_command_failed() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("ok"), tx);
    let _ = connected_with_sessions(&rx);

    engine.decide(
        SessionId::new("sess-pending"),
        String::from("req-nope"),
        ApprovalDecision::Deny,
        Some(String::from("no thanks")),
    );
    let seen = recv_until(&rx, "CommandFailed", |event| {
        matches!(event, EngineEvent::CommandFailed { .. })
    });
    let (command, failure) = command_failure(&seen).expect("failure seen above");
    assert_eq!(
        command,
        FailedCommand::Decide(SessionId::new("sess-pending"), String::from("req-nope"))
    );
    assert!(
        failure.contains("no pending approval"),
        "failure carries the server text, saw {failure:?}"
    );
    assert!(
        failed_text(&seen).is_none(),
        "the connection stays up, saw {seen:#?}"
    );
    drop(engine);
}

#[test]
fn exit_early_fails_named() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("exit-early"), tx);
    let seen = recv_until(&rx, "Failed", is_failed);
    assert!(failed_text(&seen).is_some());
    drop(engine);
}

#[test]
fn malformed_line_fails_connection() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("malformed"), tx);
    let seen = recv_until(&rx, "Failed", is_failed);
    let text = failed_text(&seen).expect("Failed seen above");
    assert!(
        text.contains("couldn't understand"),
        "failure is the friendly protocol sentence, saw {text:?}"
    );
    drop(engine);
}

#[test]
fn init_error_fails_with_server_message() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("init-error"), tx);
    let seen = recv_until(&rx, "Failed", is_failed);
    let text = failed_text(&seen).expect("Failed seen above");
    assert!(
        text.contains("fake initialize refused"),
        "failure carries the server message, saw {text:?}"
    );
    drop(engine);
}

#[test]
fn eof_after_connect_fails() {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(fake_config("eof-after-init"), tx);
    let mut seen = connected_with_sessions(&rx);
    seen.extend(recv_until(&rx, "Failed", is_failed));
    assert!(failed_text(&seen).is_some());
    drop(engine);
}

#[test]
fn missing_binary_fails_naming_command() {
    let (tx, rx) = std::sync::mpsc::channel();
    let missing = String::from("/definitely/not/a/real/binary");
    let engine = connect(
        ServerConfig {
            command: missing.clone(),
            args: Vec::new(),
            envs: Vec::new(),
        },
        tx,
    );
    let seen = recv_until(&rx, "Failed", is_failed);
    let text = failed_text(&seen).expect("Failed seen above");
    assert!(
        text.contains("Couldn't find"),
        "failure is the friendly missing-binary sentence, saw {text:?}"
    );
    assert!(
        text.contains(&missing),
        "failure names the command, saw {text:?}"
    );
    drop(engine);
}
