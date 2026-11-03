//! Live round trip against a real muse-code-mcp server.
//!
//! Ignored by default: run with `-- --ignored` and
//! `MANIFEST_MUSE_MCP_BIN` set to a real server binary.

use manifest_domain::{Connection, EngineEvent, SessionEngine};
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
