//! Crate-private constructors for the JSON-RPC 2.0 lines we send.
//!
//! Every message is compact JSON on one line; the caller appends the
//! newline and flushes. Shapes follow the server contract: `initialize`
//! declares the `muse.stream` experimental capability, `tools/call`
//! invokes `muse_status`, and server requests we never serve get
//! `-32601` so none is left hanging.

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
}
