//! Crate-private constructors for the JSON-RPC 2.0 lines we send.
//!
//! Every message is compact JSON on one line; the caller appends the
//! newline and flushes. Shapes follow the server contract: `initialize`
//! declares the `muse.stream` experimental capability, `tools/call`
//! invokes `muse_status`, `muse_spawn`, `muse_turn`, `muse_steer`,
//! `muse_cancel`, or `muse_models` (catalog listing, or session resume
//! with the catalog discarded), and server requests we never
//! serve get `-32601` so none is left hanging.

use manifest_domain::{PermissionMode, SpawnChoices, WorktreeChoice};
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
/// `workspace`, `prompt`, `wait`, plus optional `model`,
/// `reasoning_effort`, and `approval_mode`. Spawns return immediately.
/// `model` and `reasoning_effort` ride only when the choice names one;
/// `approval_mode` rides only when the posture differs from the server
/// default (Muse yolo), so a default [`SpawnChoices`] sends exactly
/// what the shell sent before these choices existed.
/// [`WorktreeChoice::Off`] adds no worktree keys;
/// [`WorktreeChoice::Create`] adds `worktree: 'create'` plus
/// `worktree_base` only when the base is `Some` and non-empty after
/// trimming (sent exactly as held); [`WorktreeChoice::Existing`] adds
/// `worktree: 'existing'` with `worktree_path`.
pub(crate) fn spawn_call(id: u64, workspace: &str, prompt: &str, choices: &SpawnChoices) -> String {
    let mut arguments = serde_json::Map::from_iter([
        (String::from("workspace"), Value::from(workspace)),
        (String::from("prompt"), Value::from(prompt)),
        (String::from("wait"), Value::from(false)),
    ]);
    match &choices.worktree {
        WorktreeChoice::Off => {}
        WorktreeChoice::Create { base } => {
            arguments.insert(String::from("worktree"), Value::from("create"));
            if let Some(base) = base
                && !base.trim().is_empty()
            {
                arguments.insert(String::from("worktree_base"), Value::from(base.clone()));
            }
        }
        WorktreeChoice::Existing { path } => {
            arguments.insert(String::from("worktree"), Value::from("existing"));
            arguments.insert(String::from("worktree_path"), Value::from(path.clone()));
        }
    }
    if let Some(model) = choices.model.id() {
        arguments.insert(String::from("model"), Value::from(model));
    }
    if let Some(effort) = choices.effort.wire_value() {
        arguments.insert(String::from("reasoning_effort"), Value::from(effort));
    }
    if choices.permission != PermissionMode::default() {
        let PermissionMode::Muse(posture) = choices.permission;
        arguments.insert(
            String::from("approval_mode"),
            Value::from(posture.wire_value()),
        );
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_spawn",
            "arguments": Value::Object(arguments),
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

/// A `tools/call` request invoking `muse_steer` with the given numeric id.
///
/// Argument names follow the server's `muse_steer` input schema:
/// `session_id`, `prompt`, `turn_id` (the running turn; idle sessions
/// error and send no frame).
pub(crate) fn steer_call(id: u64, session_id: &str, prompt: &str, turn_id: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_steer",
            "arguments": {"session_id": session_id, "prompt": prompt, "turn_id": turn_id},
        },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_cancel` with the given numeric id.
///
/// Argument names follow the server's `muse_cancel` input schema:
/// `session_id`, plus `turn_id` only when `Some`.
pub(crate) fn cancel_call(id: u64, session_id: &str, turn_id: Option<&str>) -> String {
    let mut arguments =
        serde_json::Map::from_iter([(String::from("session_id"), Value::from(session_id))]);
    if let Some(turn_id) = turn_id {
        arguments.insert(String::from("turn_id"), Value::from(turn_id));
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_cancel",
            "arguments": Value::Object(arguments),
        },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_models` for a catalog listing.
///
/// Argument names follow the server's `muse_models` input schema:
/// optional `workspace` and `session_id`. Each rides only when `Some`;
/// `session_id` flags that session's model as active in the catalog.
/// This is the read-only query; [`resume_call`] also invokes
/// `muse_models` but runs the ensure path and discards the catalog.
pub(crate) fn models_call(id: u64, workspace: Option<&str>, session_id: Option<&str>) -> String {
    let mut arguments = serde_json::Map::new();
    if let Some(workspace) = workspace {
        arguments.insert(String::from("workspace"), Value::from(workspace));
    }
    if let Some(session_id) = session_id {
        arguments.insert(String::from("session_id"), Value::from(session_id));
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_models",
            "arguments": Value::Object(arguments),
        },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_models` for a session resume.
///
/// The server has no resume tool: calling `muse_models` by
/// `session_id` runs its ensure() path, which performs
/// session/resume, and the returned catalog is discarded. Swap this
/// wire call if the server gains a resume tool.
pub(crate) fn resume_call(id: u64, session_id: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_models",
            "arguments": {"session_id": session_id},
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
            serde_json::from_str(&spawn_call(3, "/tmp/ws", "hello", &SpawnChoices::default()))
                .expect("valid JSON");
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
    fn spawn_off_adds_no_worktree_keys() {
        let value: Value =
            serde_json::from_str(&spawn_call(3, "/tmp/ws", "hello", &SpawnChoices::default()))
                .expect("valid JSON");
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert_eq!(arguments.len(), 3);
        assert!(!arguments.contains_key("worktree"));
        assert!(!arguments.contains_key("worktree_base"));
        assert!(!arguments.contains_key("worktree_path"));
    }

    #[test]
    fn spawn_create_without_base_adds_create_only() {
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            "hello",
            &SpawnChoices {
                worktree: WorktreeChoice::Create { base: None },
                ..Default::default()
            },
        ))
        .expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/worktree"),
            Some(&json!("create"))
        );
        assert_eq!(value.pointer("/params/arguments/worktree_base"), None);
    }

    #[test]
    fn spawn_create_with_base_passes_base_through() {
        let base = String::from("main ");
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            "hello",
            &SpawnChoices {
                worktree: WorktreeChoice::Create { base: Some(base) },
                ..Default::default()
            },
        ))
        .expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/worktree"),
            Some(&json!("create"))
        );
        // Sent exactly as held: not trimmed.
        assert_eq!(
            value.pointer("/params/arguments/worktree_base"),
            Some(&json!("main "))
        );
    }

    #[test]
    fn spawn_existing_adds_worktree_path() {
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            "hello",
            &SpawnChoices {
                worktree: WorktreeChoice::Existing {
                    path: String::from("/tmp/wt-1"),
                },
                ..Default::default()
            },
        ))
        .expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/worktree"),
            Some(&json!("existing"))
        );
        assert_eq!(
            value.pointer("/params/arguments/worktree_path"),
            Some(&json!("/tmp/wt-1"))
        );
    }

    #[test]
    fn resume_call_names_muse_models_with_session_id() {
        let value: Value =
            serde_json::from_str(&resume_call(9, "sess-orphan")).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(9)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_models")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-orphan"))
        );
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

    #[test]
    fn spawn_default_omits_model_effort_and_approval() {
        let value: Value =
            serde_json::from_str(&spawn_call(3, "/tmp/ws", "hello", &SpawnChoices::default()))
                .expect("valid JSON");
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert_eq!(arguments.len(), 3);
        assert!(!arguments.contains_key("model"));
        assert!(!arguments.contains_key("reasoning_effort"));
        assert!(!arguments.contains_key("approval_mode"));
    }

    #[test]
    fn spawn_named_choices_add_model_effort_and_approval() {
        use manifest_domain::MusePermission;
        use manifest_domain::{EffortChoice, ModelChoice, PermissionMode};
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            "hello",
            &SpawnChoices {
                model: ModelChoice::Named(String::from("muse-spark")),
                effort: EffortChoice::High,
                permission: PermissionMode::Muse(MusePermission::OnRequest),
                ..Default::default()
            },
        ))
        .expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/model"),
            Some(&json!("muse-spark"))
        );
        assert_eq!(
            value.pointer("/params/arguments/reasoning_effort"),
            Some(&json!("high"))
        );
        assert_eq!(
            value.pointer("/params/arguments/approval_mode"),
            Some(&json!("on-request"))
        );
    }

    #[test]
    fn spawn_blank_model_and_default_effort_stay_omitted() {
        use manifest_domain::ModelChoice;
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            "hello",
            &SpawnChoices {
                model: ModelChoice::Named(String::from("  ")),
                ..Default::default()
            },
        ))
        .expect("valid JSON");
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert!(!arguments.contains_key("model"));
        assert!(!arguments.contains_key("reasoning_effort"));
        assert!(!arguments.contains_key("approval_mode"));
    }

    #[test]
    fn steer_call_carries_session_prompt_and_turn() {
        let value: Value = serde_json::from_str(&steer_call(5, "sess-1", "wait, stop", "turn-9"))
            .expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(5)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_steer")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
        assert_eq!(
            value.pointer("/params/arguments/prompt"),
            Some(&json!("wait, stop"))
        );
        assert_eq!(
            value.pointer("/params/arguments/turn_id"),
            Some(&json!("turn-9"))
        );
    }

    #[test]
    fn cancel_call_omits_turn_when_none() {
        let value: Value =
            serde_json::from_str(&cancel_call(6, "sess-1", None)).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_cancel")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
        assert_eq!(value.pointer("/params/arguments/turn_id"), None);
    }

    #[test]
    fn cancel_call_carries_turn_when_some() {
        let value: Value =
            serde_json::from_str(&cancel_call(6, "sess-1", Some("turn-9"))).expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/turn_id"),
            Some(&json!("turn-9"))
        );
    }

    #[test]
    fn models_call_omits_absent_keys() {
        let value: Value = serde_json::from_str(&models_call(8, None, None)).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(8)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_models")));
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert!(arguments.is_empty());
    }

    #[test]
    fn models_call_carries_workspace_and_session() {
        let value: Value = serde_json::from_str(&models_call(8, Some("/tmp/ws"), Some("sess-1")))
            .expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/workspace"),
            Some(&json!("/tmp/ws"))
        );
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
    }
}
