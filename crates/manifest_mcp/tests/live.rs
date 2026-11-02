//! Live round trip against a real muse-code-mcp server.
//!
//! Ignored by default: run with `-- --ignored` and
//! `MANIFEST_MUSE_MCP_BIN` set to a real server binary.

use manifest_domain::{
    Connection, EngineEvent, ItemStatus, SessionEngine, SessionId, SpawnChoices, StreamEvent,
    StreamItem, TurnInput,
};
use manifest_mcp::{ServerConfig, connect};
use std::time::Duration;

#[test]
#[ignore = "live: run with --ignored and MANIFEST_MUSE_MCP_BIN set to a real muse-code-mcp"]
fn live_status_round_trip() {
    let command = match std::env::var("MANIFEST_MUSE_MCP_BIN") {
        Ok(command) => command,
        Err(_) => return,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(ServerConfig::new(command), tx);
    engine.list_sessions();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut saw_connected = false;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let event: EngineEvent = rx
            .recv_timeout(remaining)
            .expect("an event arrives within 30s");
        match &event {
            EngineEvent::ConnectionChanged(Connection::Connected) => saw_connected = true,
            EngineEvent::SessionsLoaded { .. } | EngineEvent::SessionsLoadFailed(_) => {
                assert!(saw_connected, "Connected precedes results, saw {event:?}");
                break;
            }
            EngineEvent::ConnectionChanged(Connection::Failed(text)) => {
                panic!("live connection failed: {text}");
            }
            _ => {}
        }
    }
    drop(engine);
}

#[test]
#[ignore = "live: run with --ignored and MANIFEST_MUSE_MCP_BIN set to a real muse-code-mcp"]
fn live_spawn_streams_a_real_turn() {
    let command = match std::env::var("MANIFEST_MUSE_MCP_BIN") {
        Ok(command) => command,
        Err(_) => return,
    };
    let marker = "The emerald lighthouse keeper counted seven brass gulls at dawn.";
    let workspace = std::env::temp_dir().join(format!(
        "manifest-live-turn-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&workspace).expect("create temp live workspace");
    std::fs::write(workspace.join("note.txt"), format!("{marker}\n"))
        .expect("write note.txt with the marker sentence");
    let workspace_text = workspace.to_string_lossy().into_owned();
    // Ask for a tool read plus a fenced quote so the assertions below can
    // tell a real tool-using turn from a bare echo.
    let prompt = String::from(
        "Read the file note.txt in the workspace with your file-reading tool \
         (you must call the tool, do not guess at the contents). Then reply \
         with exactly one short introductory line followed by the full \
         sentence from the file quoted inside a fenced code block.",
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let engine = connect(ServerConfig::new(command), tx);

    // Wait for the connection the same loud way the status test does.
    // Reaching past this loop means Connected arrived; Failed panics.
    let connect_deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = connect_deadline.saturating_duration_since(std::time::Instant::now());
        let event: EngineEvent = rx
            .recv_timeout(remaining)
            .expect("Connected or Failed arrives within 30s");
        match &event {
            EngineEvent::ConnectionChanged(Connection::Connected) => break,
            EngineEvent::ConnectionChanged(Connection::Failed(text)) => {
                panic!("live connection failed: {text}");
            }
            _ => {}
        }
    }

    engine.spawn_session(
        workspace_text,
        TurnInput::Text(prompt),
        SpawnChoices::default(),
    );

    // Overall deadline for the whole turn; the model turn can take minutes.
    let deadline = std::time::Instant::now() + Duration::from_secs(240);
    let mut seen: Vec<EngineEvent> = Vec::new();
    let mut spawned: Option<SessionId> = None;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            let tail: Vec<_> = seen.iter().rev().take(8).collect();
            panic!("timed out waiting for TurnCompleted; last events: {tail:#?}");
        }
        let event: EngineEvent = rx.recv_timeout(remaining).unwrap_or_else(|_| {
            let tail: Vec<_> = seen.iter().rev().take(8).collect();
            panic!("no event before the 240s turn deadline; last events: {tail:#?}");
        });
        match &event {
            EngineEvent::ConnectionChanged(Connection::Failed(text)) => {
                let tail: Vec<_> = seen.iter().rev().take(8).collect();
                panic!("live connection failed mid-turn: {text}; last events: {tail:#?}");
            }
            EngineEvent::CommandFailed { message, .. } => {
                let tail: Vec<_> = seen.iter().rev().take(8).collect();
                panic!("live spawn failed: {message}; last events: {tail:#?}");
            }
            EngineEvent::SessionSpawned { session, .. } => {
                spawned = Some(session.clone());
            }
            EngineEvent::Stream(StreamEvent::TurnCompleted { session, .. })
                if spawned.as_ref().is_some_and(|s| s == session) =>
            {
                seen.push(event);
                break;
            }
            _ => {}
        }
        seen.push(event);
    }

    let session = spawned.unwrap_or_else(|| {
        let tail: Vec<_> = seen.iter().rev().take(8).collect();
        panic!("no SessionSpawned before TurnCompleted; last events: {tail:#?}");
    });
    // The pattern binds dropped_deltas, so reaching here proves the field
    // exists on the wire; any value is acceptable.
    let dropped = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::TurnCompleted {
            session: s,
            dropped_deltas,
            ..
        }) if s == &session => Some(*dropped_deltas),
        _ => None,
    });
    let dropped = dropped.unwrap_or_else(|| {
        let tail: Vec<_> = seen.iter().rev().take(8).collect();
        panic!("TurnCompleted carried no dropped_deltas field; last events: {tail:#?}");
    });

    // Every stream event for this turn must carry the spawned session id.
    let foreign: Vec<_> = seen
        .iter()
        .filter_map(|event| match event {
            EngineEvent::Stream(stream) => {
                let owner = match stream {
                    StreamEvent::TurnStarted { session, .. }
                    | StreamEvent::ItemStarted { session, .. }
                    | StreamEvent::ItemDelta { session, .. }
                    | StreamEvent::ItemCompleted { session, .. }
                    | StreamEvent::TurnCompleted { session, .. }
                    | StreamEvent::Usage { session, .. }
                    | StreamEvent::Pending { session, .. }
                    | StreamEvent::CatalogChanged { session } => session,
                };
                (owner != &session).then(|| event.clone())
            }
            _ => None,
        })
        .collect();
    assert!(
        foreign.is_empty(),
        "all stream events carry session {session:?} (dropped_deltas: {dropped}); foreign: {foreign:#?}"
    );

    let saw_delta = seen.iter().any(|event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ItemDelta { session: s, .. }) if s == &session
        )
    });
    assert!(
        saw_delta,
        "at least one ItemDelta for session {session:?} (dropped_deltas: {dropped}); last events: {seen:#?}"
    );

    let saw_tool = seen.iter().any(|event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ItemCompleted {
                item: StreamItem::ToolCall { status: ItemStatus::Completed, .. },
                session: s,
                ..
            }) if s == &session
        )
    });
    assert!(
        saw_tool,
        "a ToolCall StreamItem completed at least once for session {session:?} \
         (dropped_deltas: {dropped}); last events: {seen:#?}"
    );

    let answer = seen.iter().find_map(|event| match event {
        EngineEvent::Stream(StreamEvent::ItemCompleted {
            item: StreamItem::AgentMessage { text, .. },
            session: s,
            ..
        }) if s == &session => Some(text.clone()),
        _ => None,
    });
    let answer = answer.unwrap_or_else(|| {
        panic!(
            "no completed AgentMessage for session {session:?} \
             (dropped_deltas: {dropped}); last events: {seen:#?}"
        );
    });
    assert!(
        answer.contains(marker),
        "completed AgentMessage quotes the marker sentence for session {session:?} \
         (dropped_deltas: {dropped}); answer was {answer:?}"
    );
    assert!(
        answer.contains("```"),
        "completed AgentMessage wraps the quote in a fenced code block for session {session:?} \
         (dropped_deltas: {dropped}); answer was {answer:?}"
    );

    // Cleanup: dropping the engine kills the server child. There is no
    // close-session API, so the spawned Muse session is left to the server;
    // only the temp workspace is removed here.
    drop(engine);
    let _ = std::fs::remove_dir_all(&workspace);
}
