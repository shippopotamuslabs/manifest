//! Transport tests against the scripted fake server.

use manifest_domain::{
    Connection, EngineEvent, PendingKind, PendingPhase, ProjectId, SessionEngine, SessionId,
    SessionStatus, StreamEvent,
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
        EngineEvent::SessionsLoaded { projects, sessions } => Some((projects, sessions)),
        _ => None,
    });
    let (projects, sessions) = loaded.expect("SessionsLoaded seen above");
    assert_eq!(
        projects.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        vec![String::from("checkout-api"), String::from("storefront-web")],
    );
    assert_eq!(
        projects.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        vec![
            ProjectId::new("workspace:/home/dev/checkout-api"),
            ProjectId::new("workspace:/home/dev/storefront-web"),
        ],
    );
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
            SessionStatus::Idle,
            SessionStatus::AwaitingApproval,
        ],
    );
    assert_eq!(sessions[0].title, "checkout-api · sess-run");
    assert_eq!(sessions[0].stream_dropped_deltas, 3);
    assert_eq!(
        sessions[0].project,
        Some(ProjectId::new("workspace:/home/dev/checkout-api"))
    );

    assert!(
        !seen.iter().any(is_failed),
        "no failure while listing, saw {seen:#?}"
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
