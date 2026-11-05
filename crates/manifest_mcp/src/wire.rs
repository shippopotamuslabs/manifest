//! Crate-private constructors for the JSON-RPC 2.0 lines we send.
//!
//! Every message is compact JSON on one line; the caller appends the
//! newline and flushes. Shapes follow the server contract: `initialize`
//! declares the `muse.stream` experimental capability, `tools/call`
//! invokes `muse_status`, `muse_spawn`, `muse_turn`, `muse_steer`,
//! `muse_cancel`, `muse_models` (catalog listing, or session resume
//! with the catalog discarded), or `muse_raw` (session compaction and
//! goal setting, forwarded unchanged), and server requests we never
//! serve get `-32601` so none is left hanging.

use manifest_domain::{
    AnswerValue, ApprovalDecision, PendingAnswer, PermissionMode, SpawnChoices, TurnInput,
    WorktreeChoice, truncate_free_text,
};
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

/// Encode one [`TurnInput`] into spawn/turn arguments.
///
/// Plain text keeps the exact historical shape (`prompt`); a skill
/// invocation replaces it with `parts` carrying one `skill` part
/// (`selector`, plus `source` only when `Some` and non-empty, plus
/// `arguments` only when non-empty). `parts` is the companion-server
/// contract: `muse_turn`/`muse_spawn` accept optional `parts` while
/// `prompt` is unchanged otherwise.
fn input_arguments(session_id: Option<&str>, input: &TurnInput) -> serde_json::Map<String, Value> {
    let mut arguments = serde_json::Map::new();
    if let Some(session_id) = session_id {
        arguments.insert(String::from("session_id"), Value::from(session_id));
    }
    match input {
        TurnInput::Text(prompt) => {
            arguments.insert(String::from("prompt"), Value::from(prompt.as_str()));
        }
        TurnInput::Skill(invocation) => {
            let mut part = serde_json::Map::from_iter([
                (String::from("type"), Value::from("skill")),
                (
                    String::from("selector"),
                    Value::from(invocation.selector.as_str()),
                ),
            ]);
            if let Some(source) = invocation
                .source
                .as_deref()
                .filter(|source| !source.is_empty())
            {
                part.insert(String::from("source"), Value::from(source));
            }
            if !invocation.arguments.is_empty() {
                part.insert(
                    String::from("arguments"),
                    Value::from(invocation.arguments.as_str()),
                );
            }
            arguments.insert(
                String::from("parts"),
                Value::Array(vec![Value::Object(part)]),
            );
        }
    }
    arguments
}

/// A `tools/call` request invoking `muse_spawn` with the given numeric id.
///
/// Argument names follow the server's `muse_spawn` input schema:
/// `workspace`, `prompt` (or `parts` for a skill input), `wait`,
/// plus optional `model`, `reasoning_effort`, and
/// `approval_mode`. Spawns return immediately.
/// `model` and `reasoning_effort` ride only when the choice names one;
/// `approval_mode` rides only when the posture differs from the server
/// default (Muse yolo), so a default [`SpawnChoices`] sends exactly
/// what the shell sent before these choices existed.
/// [`WorktreeChoice::Off`] adds no worktree keys;
/// [`WorktreeChoice::Create`] adds `worktree: 'create'` plus
/// `worktree_base` only when the base is `Some` and non-empty after
/// trimming (sent exactly as held); [`WorktreeChoice::Existing`] adds
/// `worktree: 'existing'` with `worktree_path`.
pub(crate) fn spawn_call(
    id: u64,
    workspace: &str,
    input: &TurnInput,
    choices: &SpawnChoices,
) -> String {
    let mut arguments = serde_json::Map::from_iter([
        (String::from("workspace"), Value::from(workspace)),
        (String::from("wait"), Value::from(false)),
    ]);
    for (key, value) in input_arguments(None, input) {
        arguments.insert(key, value);
    }
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
/// `session_id`, `prompt` (or `parts` for a skill input), `wait`.
/// Turns return immediately; their events stream next. A skill input
/// emits `parts` instead of `prompt`; see [`input_arguments`].
pub(crate) fn turn_call(id: u64, session_id: &str, input: &TurnInput) -> String {
    let mut arguments = input_arguments(Some(session_id), input);
    arguments.insert(String::from("wait"), Value::from(false));
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_turn",
            "arguments": Value::Object(arguments),
        },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_skills` for a catalog listing.
///
/// Argument names follow the server's `muse_skills` input schema:
/// `session_id` names the session whose catalog is listed.
pub(crate) fn skills_call(id: u64, session_id: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_skills",
            "arguments": {"session_id": session_id},
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

/// A process-local UUIDv7-shaped idempotency handle for `muse_raw`
/// host methods (`session/compact`, `goal/set`), which require a
/// `commandId`.
///
/// The crate takes no UUID dependency, so this mints the shape from
/// the wall clock (48-bit Unix millis) plus a process counter for
/// the random bits: unique per command in this process, version 7,
/// RFC 4122 variant. Randomness quality is irrelevant here; the host
/// only needs a fresh handle per command.
fn command_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0) as u64
        & 0xFFFF_FFFF_FFFF;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Twelve versioned bits plus the low variant bits; the counter
    // keeps same-millisecond commands distinct.
    let rand_a = (n ^ (millis >> 16)) & 0x0FFF;
    let rand_b = (n.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ millis) & 0x3FFF_FFFF_FFFF_FFFF;
    format!(
        "{:08x}-{:04x}-7{:03x}-{:04x}-{:012x}",
        (millis >> 16) & 0xFFFF_FFFF,
        millis & 0xFFFF,
        rand_a,
        0x8000 | ((rand_b >> 48) & 0x3FFF),
        rand_b & 0xFFFF_FFFF_FFFF,
    )
}

/// A `tools/call` request invoking `muse_raw` with the given numeric id.
///
/// Argument names follow the server's `muse_raw` input schema:
/// `session_id`, `method`, `params` (forwarded unchanged to the
/// session's host). Credential methods stay refused server-side
/// (`account/*`, `oauth`, `login`, `logout`); this client never sends
/// them.
pub(crate) fn raw_call(id: u64, session_id: &str, method: &str, params: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_raw",
            "arguments": {"session_id": session_id, "method": method, "params": params},
        },
    })
    .to_string()
}

/// A `tools/call` request compacting `session_id` through `muse_raw`.
///
/// The host method is `session/compact` with `commandId` (see
/// [`command_id`]) and `sessionId`; `turnId` is omitted so the server
/// resolves the session's current run. The ack admits the compaction;
/// its outcome arrives as a terminal view event.
pub(crate) fn compact_call(id: u64, session_id: &str) -> String {
    raw_call(
        id,
        session_id,
        "session/compact",
        json!({"commandId": command_id(), "sessionId": session_id}),
    )
}

/// A `tools/call` request setting `session_id`'s goal through `muse_raw`.
///
/// The host method is `goal/set` with `commandId` (see
/// [`command_id`]), `sessionId`, and the trimmed `objective` (the
/// host rejects empty-after-trim, so the shell never sends one).
pub(crate) fn goal_call(id: u64, session_id: &str, objective: &str) -> String {
    raw_call(
        id,
        session_id,
        "goal/set",
        json!({"commandId": command_id(), "sessionId": session_id, "objective": objective}),
    )
}

/// A `tools/call` request pausing `session_id`'s goal through `muse_raw`.
///
/// The host method is `goal/pause` with exactly `commandId` (see
/// [`command_id`]) and `sessionId`: an `objective` on pause is an
/// unknown field the host rejects.
pub(crate) fn goal_pause_call(id: u64, session_id: &str) -> String {
    raw_call(
        id,
        session_id,
        "goal/pause",
        json!({"commandId": command_id(), "sessionId": session_id}),
    )
}

/// A `tools/call` request resuming `session_id`'s goal through `muse_raw`.
///
/// The host method is `goal/resume` with exactly `commandId` (see
/// [`command_id`]) and `sessionId`, mirroring [`goal_pause_call`].
pub(crate) fn goal_resume_call(id: u64, session_id: &str) -> String {
    raw_call(
        id,
        session_id,
        "goal/resume",
        json!({"commandId": command_id(), "sessionId": session_id}),
    )
}

/// A `tools/call` request editing `session_id`'s goal through `muse_raw`.
///
/// The host method is `goal/edit` with `commandId` (see
/// [`command_id`]), `sessionId`, and the trimmed `objective` (the
/// host rejects empty-after-trim, so the shell never sends one).
/// Set and edit differ in their missing-goal precondition, not in
/// payload shape.
pub(crate) fn goal_edit_call(id: u64, session_id: &str, objective: &str) -> String {
    raw_call(
        id,
        session_id,
        "goal/edit",
        json!({"commandId": command_id(), "sessionId": session_id, "objective": objective}),
    )
}

/// A `tools/call` request clearing `session_id`'s goal through `muse_raw`.
///
/// The host method is `goal/clear` with exactly `commandId` (see
/// [`command_id`]) and `sessionId`: an `objective` on clear is an
/// unknown field the host rejects.
pub(crate) fn goal_clear_call(id: u64, session_id: &str) -> String {
    raw_call(
        id,
        session_id,
        "goal/clear",
        json!({"commandId": command_id(), "sessionId": session_id}),
    )
}

/// A `tools/call` request reading `session_id`'s stored history
/// through `muse_raw`.
///
/// The host method is `session/read` with `sessionId` and
/// `excludeItems: false` (a read, so no `commandId`). The result
/// carries the `history` object (`mode`, `items` or `snapshot`, and
/// an optional `noneReason`) the mapper folds into seed items.
pub(crate) fn history_call(id: u64, session_id: &str) -> String {
    raw_call(
        id,
        session_id,
        "session/read",
        json!({"sessionId": session_id, "excludeItems": false}),
    )
}

/// A `tools/call` request reading one stored patch body through `muse_raw`.
///
/// The host method is `item/readOutput` with `sessionId`, `itemId`,
/// `outputRef` (the item's `patchRef.id`), and `offsetBytes` (a
/// read, so no `commandId`). The worker pages `offsetBytes` forward
/// by each reply's `byteLen` until `eof`, then parses the
/// concatenated body as the patch document.
pub(crate) fn patch_call(
    id: u64,
    session_id: &str,
    item_id: &str,
    patch_ref_id: &str,
    offset: u64,
) -> String {
    raw_call(
        id,
        session_id,
        "item/readOutput",
        json!({"sessionId": session_id, "itemId": item_id, "outputRef": patch_ref_id, "offsetBytes": offset}),
    )
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

/// A `tools/call` request invoking `muse_pending` with the given numeric id.
///
/// The tool takes no arguments; its result is the source of truth for
/// approval/question cards.
pub(crate) fn pending_call(id: u64) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": "muse_pending", "arguments": {} },
    })
    .to_string()
}

/// A `tools/call` request invoking `muse_decide` with the given numeric id.
///
/// `actor` is always `"user"`: the server otherwise defaults to
/// `orchestrator`. `reason` rides only on a deny with a non-empty
/// reason; the server forwards it as feedback only when the chosen
/// choice accepts feedback.
pub(crate) fn decide_call(
    id: u64,
    request_id: &str,
    decision: ApprovalDecision,
    reason: Option<&str>,
) -> String {
    let mut arguments = serde_json::Map::from_iter([
        (String::from("request_id"), Value::from(request_id)),
        (String::from("decision"), Value::from(decision.wire_value())),
        (String::from("actor"), Value::from("user")),
    ]);
    if decision == ApprovalDecision::Deny
        && let Some(reason) = reason
        && !reason.trim().is_empty()
    {
        arguments.insert(String::from("reason"), Value::from(reason));
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_decide",
            "arguments": Value::Object(arguments),
        },
    })
    .to_string()
}

/// Encode one [`AnswerValue`] as a server `WireAnswer` object.
///
/// Mirrors `buildUserInputAnswers`' array path in the server's
/// `answers.ts`: a label becomes `selectedLabel`, labels become
/// `selectedLabels`, and free text becomes `freeText` capped at 500
/// chars. A skipped question carries only its `questionId` (the
/// `asWireAnswer` path accepts that shape); no skipped wire value
/// exists, and `freeText: ""` would read as an empty answer.
fn wire_answer(question_id: &str, value: &AnswerValue) -> Value {
    match value {
        AnswerValue::Label(label) => json!({
            "questionId": question_id,
            "selectedLabel": label,
        }),
        AnswerValue::Labels(labels) => json!({
            "questionId": question_id,
            "selectedLabels": labels,
        }),
        AnswerValue::FreeText(text) => json!({
            "questionId": question_id,
            "freeText": truncate_free_text(text),
        }),
        AnswerValue::Skipped => json!({ "questionId": question_id }),
    }
}

/// A `tools/call` request invoking `muse_answer` with the given numeric id.
///
/// `actor` is always `"user"`. The answer rides as the array shape
/// the server's `asWireAnswer` passes through, one entry per question.
pub(crate) fn answer_call(id: u64, request_id: &str, answer: &PendingAnswer) -> String {
    let answers: Vec<Value> = answer
        .answers
        .iter()
        .map(|(question_id, value)| wire_answer(question_id, value))
        .collect();
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "muse_answer",
            "arguments": {
                "request_id": request_id,
                "actor": "user",
                "answer": answers,
            },
        },
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
    use manifest_domain::SkillInvocation;

    /// Plain-text input for the pre-skill call shapes.
    fn text(prompt: &str) -> TurnInput {
        TurnInput::Text(String::from(prompt))
    }

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
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            &text("hello"),
            &SpawnChoices::default(),
        ))
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
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            &text("hello"),
            &SpawnChoices::default(),
        ))
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
            &text("hello"),
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
            &text("hello"),
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
            &text("hello"),
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
            serde_json::from_str(&turn_call(4, "sess-1", &text("again"))).expect("valid JSON");
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
        let value: Value = serde_json::from_str(&spawn_call(
            3,
            "/tmp/ws",
            &text("hello"),
            &SpawnChoices::default(),
        ))
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
            &text("hello"),
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
            &text("hello"),
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
    fn compact_call_sends_session_compact_without_turn() {
        let value: Value = serde_json::from_str(&compact_call(6, "sess-1")).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_raw")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("session/compact"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert!(params.get("turnId").is_none());
        let command_id = params
            .get("commandId")
            .and_then(Value::as_str)
            .expect("commandId present");
        assert!(is_uuid_v7(command_id), "commandId is UUIDv7: {command_id}");
    }

    #[test]
    fn goal_call_sends_goal_set_with_objective() {
        let value: Value =
            serde_json::from_str(&goal_call(7, "sess-1", "ship it")).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_raw")));
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("goal/set"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert_eq!(params.get("objective"), Some(&json!("ship it")));
        let command_id = params
            .get("commandId")
            .and_then(Value::as_str)
            .expect("commandId present");
        assert!(is_uuid_v7(command_id), "commandId is UUIDv7: {command_id}");
    }

    #[test]
    fn goal_pause_call_sends_goal_pause_with_id_pair_only() {
        let value: Value = serde_json::from_str(&goal_pause_call(7, "sess-1")).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_raw")));
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("goal/pause"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert!(params.get("objective").is_none());
        assert!(params.get("turnId").is_none());
        assert_eq!(
            params.as_object().expect("params is an object").len(),
            2,
            "pause carries exactly commandId + sessionId"
        );
        let command_id = params
            .get("commandId")
            .and_then(Value::as_str)
            .expect("commandId present");
        assert!(is_uuid_v7(command_id), "commandId is UUIDv7: {command_id}");
    }

    #[test]
    fn goal_resume_call_sends_goal_resume_with_id_pair_only() {
        let value: Value =
            serde_json::from_str(&goal_resume_call(7, "sess-1")).expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("goal/resume"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert!(params.get("objective").is_none());
        assert_eq!(
            params.as_object().expect("params is an object").len(),
            2,
            "resume carries exactly commandId + sessionId"
        );
        let command_id = params
            .get("commandId")
            .and_then(Value::as_str)
            .expect("commandId present");
        assert!(is_uuid_v7(command_id), "commandId is UUIDv7: {command_id}");
    }

    #[test]
    fn goal_edit_call_sends_goal_edit_with_objective() {
        let value: Value =
            serde_json::from_str(&goal_edit_call(7, "sess-1", "ship it")).expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("goal/edit"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert_eq!(params.get("objective"), Some(&json!("ship it")));
        let command_id = params
            .get("commandId")
            .and_then(Value::as_str)
            .expect("commandId present");
        assert!(is_uuid_v7(command_id), "commandId is UUIDv7: {command_id}");
    }

    #[test]
    fn goal_clear_call_sends_goal_clear_with_id_pair_only() {
        let value: Value = serde_json::from_str(&goal_clear_call(7, "sess-1")).expect("valid JSON");
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("goal/clear"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert!(params.get("objective").is_none());
        assert_eq!(
            params.as_object().expect("params is an object").len(),
            2,
            "clear carries exactly commandId + sessionId"
        );
        let command_id = params
            .get("commandId")
            .and_then(Value::as_str)
            .expect("commandId present");
        assert!(is_uuid_v7(command_id), "commandId is UUIDv7: {command_id}");
    }

    #[test]
    fn history_call_sends_session_read_without_command_id() {
        let value: Value = serde_json::from_str(&history_call(8, "sess-1")).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_raw")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("session/read"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert_eq!(params.get("excludeItems"), Some(&json!(false)));
        assert!(params.get("commandId").is_none());
    }

    #[test]
    fn patch_call_sends_item_read_output_without_command_id() {
        let value: Value = serde_json::from_str(&patch_call(9, "sess-1", "item-7", "ref-3", 41))
            .expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_raw")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
        assert_eq!(
            value.pointer("/params/arguments/method"),
            Some(&json!("item/readOutput"))
        );
        let params = value
            .pointer("/params/arguments/params")
            .expect("params present");
        assert_eq!(params.get("sessionId"), Some(&json!("sess-1")));
        assert_eq!(params.get("itemId"), Some(&json!("item-7")));
        assert_eq!(params.get("outputRef"), Some(&json!("ref-3")));
        assert_eq!(params.get("offsetBytes"), Some(&json!(41)));
        assert!(params.get("commandId").is_none());
    }

    #[test]
    fn command_ids_are_unique_per_call() {
        let first: Value = serde_json::from_str(&compact_call(6, "sess-1")).expect("valid JSON");
        let second: Value = serde_json::from_str(&compact_call(6, "sess-1")).expect("valid JSON");
        let id = |value: &Value| value.pointer("/params/arguments/params/commandId").cloned();
        assert_ne!(id(&first), id(&second));
    }

    /// Whether `text` parses as a UUIDv7 string (version nibble `7`,
    /// RFC 4122 variant): the shape [`command_id`] mints.
    fn is_uuid_v7(text: &str) -> bool {
        let parts: Vec<&str> = text.split('-').collect();
        if parts.len() != 5 {
            return false;
        }
        let hex = |part: &str| part.chars().all(|c| c.is_ascii_hexdigit());
        parts.iter().all(|part| hex(part))
            && parts[2].starts_with('7')
            && matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b'))
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
    fn pending_call_names_muse_pending_with_no_args() {
        let value: Value = serde_json::from_str(&pending_call(11)).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(11)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_pending")));
        assert_eq!(
            value.pointer("/params/arguments"),
            Some(&json!({})),
            "muse_pending takes no arguments"
        );
    }

    #[test]
    fn decide_call_always_actors_user() {
        let value: Value =
            serde_json::from_str(&decide_call(12, "req-1", ApprovalDecision::Approve, None))
                .expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_decide")));
        assert_eq!(
            value.pointer("/params/arguments/request_id"),
            Some(&json!("req-1"))
        );
        assert_eq!(
            value.pointer("/params/arguments/decision"),
            Some(&json!("approve"))
        );
        assert_eq!(
            value.pointer("/params/arguments/actor"),
            Some(&json!("user"))
        );
        assert_eq!(value.pointer("/params/arguments/reason"), None);
    }

    #[test]
    fn decide_deny_reason_only_when_some_non_empty() {
        let with = serde_json::from_str::<Value>(&decide_call(
            12,
            "req-1",
            ApprovalDecision::Deny,
            Some("not safe"),
        ))
        .expect("valid JSON");
        assert_eq!(
            with.pointer("/params/arguments/reason"),
            Some(&json!("not safe"))
        );
        for reason in [None, Some(""), Some("   ")] {
            let value = serde_json::from_str::<Value>(&decide_call(
                12,
                "req-1",
                ApprovalDecision::Deny,
                reason,
            ))
            .expect("valid JSON");
            assert_eq!(
                value.pointer("/params/arguments/reason"),
                None,
                "reason {reason:?} must not ride"
            );
        }
        // A reason on approve never rides either.
        let approve = serde_json::from_str::<Value>(&decide_call(
            12,
            "req-1",
            ApprovalDecision::Approve,
            Some("looks fine"),
        ))
        .expect("valid JSON");
        assert_eq!(approve.pointer("/params/arguments/reason"), None);
    }

    #[test]
    fn answer_call_encodes_each_answer_kind() {
        let answer = PendingAnswer::new(vec![
            (String::from("q1"), AnswerValue::Label(String::from("Red"))),
            (
                String::from("q2"),
                AnswerValue::Labels(vec![String::from("A"), String::from("B")]),
            ),
            (
                String::from("q3"),
                AnswerValue::FreeText(String::from("my own words")),
            ),
            (String::from("q4"), AnswerValue::Skipped),
        ]);
        let value: Value =
            serde_json::from_str(&answer_call(13, "req-9", &answer)).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_answer")));
        assert_eq!(
            value.pointer("/params/arguments/request_id"),
            Some(&json!("req-9"))
        );
        assert_eq!(
            value.pointer("/params/arguments/actor"),
            Some(&json!("user"))
        );
        assert_eq!(
            value.pointer("/params/arguments/answer"),
            Some(&json!([
                {"questionId": "q1", "selectedLabel": "Red"},
                {"questionId": "q2", "selectedLabels": ["A", "B"]},
                {"questionId": "q3", "freeText": "my own words"},
                {"questionId": "q4"},
            ])),
            "skip carries only questionId, never freeText"
        );
    }

    #[test]
    fn answer_call_caps_free_text_at_500_chars() {
        let answer = PendingAnswer::single("q1", AnswerValue::FreeText("x".repeat(600)));
        let value: Value =
            serde_json::from_str(&answer_call(13, "req-9", &answer)).expect("valid JSON");
        let text = value
            .pointer("/params/arguments/answer/0/freeText")
            .and_then(Value::as_str)
            .expect("freeText present");
        assert_eq!(text.chars().count(), 500);
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

    #[test]
    fn text_turn_call_json_unchanged() {
        let value: Value =
            serde_json::from_str(&turn_call(4, "sess-1", &text("again"))).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_turn")));
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert_eq!(arguments.len(), 3);
        assert_eq!(arguments.get("session_id"), Some(&json!("sess-1")));
        assert_eq!(arguments.get("prompt"), Some(&json!("again")));
        assert_eq!(arguments.get("wait"), Some(&json!(false)));
        assert!(!arguments.contains_key("parts"));
    }

    /// A skill turn replaces `prompt` with one `skill` part carrying
    /// every populated field.
    #[test]
    fn skill_turn_call_emits_parts_without_prompt() {
        let input = TurnInput::Skill(SkillInvocation {
            selector: String::from("review"),
            source: Some(String::from("user")),
            arguments: String::from("pr 42"),
        });
        let value: Value =
            serde_json::from_str(&turn_call(4, "sess-1", &input)).expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_turn")));
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert_eq!(arguments.len(), 3);
        assert_eq!(arguments.get("session_id"), Some(&json!("sess-1")));
        assert_eq!(arguments.get("wait"), Some(&json!(false)));
        assert!(!arguments.contains_key("prompt"));
        assert_eq!(
            arguments.get("parts"),
            Some(&json!([{
                "type": "skill",
                "selector": "review",
                "source": "user",
                "arguments": "pr 42",
            }])),
        );
    }

    /// `source` rides only when `Some` and non-empty; `arguments`
    /// rides only when non-empty.
    #[test]
    fn skill_turn_call_omits_empty_source_and_arguments() {
        for (source, expect_source) in [
            (None, false),
            (Some(String::new()), false),
            (Some(String::from("user")), true),
        ] {
            let input = TurnInput::Skill(SkillInvocation {
                selector: String::from("review"),
                source,
                arguments: String::new(),
            });
            let value: Value =
                serde_json::from_str(&turn_call(4, "sess-1", &input)).expect("valid JSON");
            let part = value
                .pointer("/params/arguments/parts/0")
                .expect("one skill part")
                .as_object()
                .expect("part is an object");
            assert_eq!(part.len(), if expect_source { 3 } else { 2 });
            assert_eq!(part.get("type"), Some(&json!("skill")));
            assert_eq!(part.get("selector"), Some(&json!("review")));
            assert!(!part.contains_key("arguments"));
            assert_eq!(
                part.get("source"),
                expect_source.then_some(&json!("user")),
                "source rides only when Some and non-empty"
            );
        }
    }

    /// A skill spawn keeps the workspace/choices shape and swaps
    /// `prompt` for the same `parts` idea, without `session_id`.
    #[test]
    fn skill_spawn_call_emits_parts_without_prompt_or_session() {
        let input = TurnInput::Skill(SkillInvocation {
            selector: String::from("review"),
            source: None,
            arguments: String::from("pr 42"),
        });
        let value: Value =
            serde_json::from_str(&spawn_call(3, "/tmp/ws", &input, &SpawnChoices::default()))
                .expect("valid JSON");
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_spawn")));
        let arguments = value
            .pointer("/params/arguments")
            .expect("arguments present")
            .as_object()
            .expect("arguments is an object");
        assert_eq!(arguments.len(), 3);
        assert_eq!(arguments.get("workspace"), Some(&json!("/tmp/ws")));
        assert_eq!(arguments.get("wait"), Some(&json!(false)));
        assert!(!arguments.contains_key("prompt"));
        assert!(!arguments.contains_key("session_id"));
        assert_eq!(
            arguments.get("parts"),
            Some(&json!([{
                "type": "skill",
                "selector": "review",
                "arguments": "pr 42",
            }])),
        );
    }

    #[test]
    fn skills_call_names_muse_skills_with_session_id() {
        let value: Value = serde_json::from_str(&skills_call(9, "sess-1")).expect("valid JSON");
        assert_eq!(value.get("id"), Some(&json!(9)));
        assert_eq!(value.pointer("/params/name"), Some(&json!("muse_skills")));
        assert_eq!(
            value.pointer("/params/arguments/session_id"),
            Some(&json!("sess-1"))
        );
    }
}
