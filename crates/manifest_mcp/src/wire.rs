//! Crate-private constructors for the JSON-RPC 2.0 lines we send.
//!
//! Every message is compact JSON on one line; the caller appends the
//! newline and flushes. Shapes follow the server contract: `initialize`
//! declares the `muse.stream` experimental capability, `tools/call`
//! invokes `muse_status`, `muse_spawn`, or `muse_turn`, and server
//! requests we never serve get `-32601` so none is left hanging.

use serde_json::{Value, json};

/// Client version stamped into `clientInfo`; the crate version.
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `initialize` request for the given numeric id.
pub(crate) fn initialize_request(id: u64) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": { "experimental": { "muse.stream": {} } },
            "clientInfo": { "name": "manifest", "version": CLIENT_VERSION },
        },
    })
    .to_string()
}

/// The `notifications/initialized` notification sent after a good `initialize` result.
pub(crate) fn initialized_notification() -> String {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {},
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_spawn` with the given numeric id.
///
/// Argument names follow the server's `muse_spawn` input schema:
/// `workspace`, `prompt`, `wait`. Spawns return immediately.
pub(crate) fn spawn_call(id: u64, workspace: &str, prompt: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_spawn",
            "arguments": {"workspace": workspace, "prompt": prompt, "wait": false},
        },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_turn` with the given numeric id.
///
/// Argument names follow the server's `muse_turn` input schema:
/// `session_id`, `prompt`, `wait`. Turns return immediately; their
/// events stream next.
pub(crate) fn turn_call(id: u64, session_id: &str, prompt: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_turn",
            "arguments": {"session_id": session_id, "prompt": prompt, "wait": false},
        },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_status` with the given numeric id.
pub(crate) fn status_call(id: u64) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": "muse_status", "arguments": {} },
    })
    .to_string()
}

/// A `-32601` reply to a server request, echoing its id verbatim.
///
/// We do not advertise elicitation in M2, so no server request is served.
pub(crate) fn method_not_supported(id: &Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": "Method not supported by this client" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_declares_stream_capability() {
        let value: Value = serde_json::from_str(&initialize_request(1)).expect("valid JSON");
        assert_eq!(
            value
                .pointer("/params/capabilities/experimental/muse.stream")
                .expect("stream capability present"),
            &json!({}),
        );
        assert_eq!(
            value.pointer("/params/protocolVersion"),
            Some(&json!("2025-06-18"))
        );
        assert_eq!(value.get("id"), Some(&json!(1)));
    }

    #[test]
    fn status_call_names_muse_status() {
        let value: Value = serde_json::from_str(&status_call(7)).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(7)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_status")),);
    }

    #[test]
    fn spawn_call_carries_workspace_prompt_and_no_wait() {
        let value: Value =
            serde_json::from_str(&spawn_call(3, "/tmp/ws", "hello")).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(3)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_spawn")));
        assert_eq!(
            value.pointer("/params/arguments/workspace"),
            Some(&json!("/tmp/ws"))
        );
        assert_eq!(
            value.pointer("/params/arguments/prompt"),
            Some(&json!("hello"))
        );
        assert_eq!(value.pointer("/params/arguments/wait"), Some(&json!(false)));
    }

    #[test]
    fn turn_call_carries_session_prompt_and_no_wait() {
        let value: Value =
            serde_json::from_str(&turn_call(4, "sess-1", "again")).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(4)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_turn")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
        assert_eq!(
            value.pointer("/params/arguments/prompt"),
            Some(&json!("again"))
        );
        assert_eq!(value.pointer("/params/arguments/wait"), Some(&json!(false)));
    }
}
