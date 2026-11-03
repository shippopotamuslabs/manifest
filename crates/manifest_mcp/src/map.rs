//! Crate-private mapping from wire values into domain values.
//!
//! The wire carries no project objects: each session row keeps its
//! workspace and a title of the session id's first 8 chars, and
//! grouping onto projects happens in the shell via domain
//! [`assign_projects`](manifest_domain::assign_projects). Anything the
//! wire cannot explain becomes a [`Diagnostic`] value rather than a
//! silent drop, except `closed` sessions, which are expected filtering.

use crate::error::Error;
use manifest_domain::{
    EngineEvent, ItemStatus, PendingKind, PendingPhase, SessionId, SessionStatus, SessionSummary,
    StreamEvent, StreamItem, TurnId, UsageKind,
};
use serde::Deserialize;
use serde_json::Value;

/// One row of a `muse_status` result. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct StatusDto {
    session_id: String,
    status: String,
    workspace: String,
    #[serde(default)]
    pending_approvals: u64,
    #[serde(default)]
    pending_user_inputs: u64,
    #[serde(default)]
    stream_dropped_deltas: Option<u64>,
}

/// Map raw `muse_status` rows into sessions and diagnostics.
///
/// Server row order is preserved; project grouping happens in the
/// shell via domain
/// [`assign_projects`](manifest_domain::assign_projects).
pub(crate) fn map_status_rows(rows: &[Value]) -> (Vec<SessionSummary>, Vec<String>) {
    let mut sessions = Vec::new();
    let mut diagnostics = Vec::new();

    for row in rows {
        let dto: StatusDto = match serde_json::from_value(row.clone()) {
            Ok(dto) => dto,
            Err(_) => {
                diagnostics.push(String::from(
                    "A session from the server couldn't be read and was skipped.",
                ));
                continue;
            }
        };
        if dto.status == "closed" {
            continue;
        }
        let status = match dto.status.as_str() {
            "idle" => SessionStatus::Idle,
            "orphaned" => SessionStatus::Orphaned,
            "running" => SessionStatus::Running,
            other => {
                diagnostics.push(format!(
                    "A session had an unrecognized status (`{other}`) and was skipped."
                ));
                continue;
            }
        };
        let status = if dto.pending_approvals > 0 || dto.pending_user_inputs > 0 {
            SessionStatus::AwaitingApproval
        } else {
            status
        };
        // First 8 chars (not bytes) so multi-byte ids truncate safely.
        let short: String = dto.session_id.chars().take(8).collect();
        sessions.push(SessionSummary {
            id: SessionId::new(&dto.session_id),
            title: short,
            status,
            project: None,
            workspace: dto.workspace.clone(),
            relative_time: String::new(),
            stream_dropped_deltas: dto.stream_dropped_deltas.unwrap_or(0),
        });
    }

    (sessions, diagnostics)
}

/// Optional string field: a present non-string value counts as absent.
fn opt_string(params: &Value, key: &str) -> Option<String> {
    params.get(key)?.as_str().map(String::from)
}

/// Required non-empty string field.
fn req_string(params: &Value, key: &str) -> Result<String, String> {
    match params.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!("stream event is missing {key}")),
    }
}

/// Session, turn, and view cursor shared by the known stream event types.
///
/// Parsing them here keeps unknown event types free of any `sessionId`
/// requirement: the wildcard arm returns `Ok(None)` before calling this.
fn parts(params: &Value) -> Result<(SessionId, Option<TurnId>, Option<String>), String> {
    let session = SessionId::new(req_string(params, "sessionId")?);
    let turn = opt_string(params, "turnId").map(TurnId::new);
    let view_cursor = opt_string(params, "viewCursor");
    Ok((session, turn, view_cursor))
}

/// Headline arg keys in priority order for tool calls.
const HEADLINE_KEYS: [&str; 8] = [
    "description",
    "command",
    "path",
    "file",
    "query",
    "pattern",
    "url",
    "prompt",
];

/// One-line headline from raw args JSON text.
///
/// When the args parse as an object, the first present non-empty string
/// among [`HEADLINE_KEYS`], else the first string value among the
/// object's entries. Anything else yields no headline.
fn headline_from_args(args_json: &str) -> Option<String> {
    let object = serde_json::from_str::<Value>(args_json)
        .ok()?
        .as_object()?
        .clone();
    for key in HEADLINE_KEYS {
        if let Some(text) = object.get(key).and_then(Value::as_str)
            && !text.is_empty()
        {
            return Some(text.to_string());
        }
    }
    object
        .values()
        .find_map(|value| value.as_str().map(String::from))
}

/// Renderable text from a `visibleOutput` value.
///
/// Shell tool results arrive as a JSON envelope string holding an
/// `output` field; anything else renders as its raw string.
fn unwrap_output(raw: &str) -> String {
    if let Ok(Value::Object(envelope)) = serde_json::from_str::<Value>(raw)
        && let Some(text) = envelope.get("output").and_then(Value::as_str)
    {
        return text.to_string();
    }
    raw.to_string()
}

/// Wire status text into an [`ItemStatus`]; unknown or absent stays a value.
fn map_item_status(item: &Value) -> ItemStatus {
    match item.get("status").and_then(Value::as_str) {
        Some("inProgress") => ItemStatus::InProgress,
        Some("completed") => ItemStatus::Completed,
        Some("failed") => ItemStatus::Failed,
        Some(other) => ItemStatus::Other(other.to_string()),
        None => ItemStatus::Other(String::new()),
    }
}

/// Text field that is empty when the wire sent none or a non-string.
fn item_text(item: &Value, key: &str) -> String {
    item.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Parse one wire item object into a [`StreamItem`].
///
/// `itemId` is required; `kind` decides the variant. `reminderChild`
/// classifies as system bookkeeping; any other unknown kind stays an
/// [`StreamItem::Unknown`] value, never a silent drop.
fn parse_stream_item(item: &Value) -> Result<StreamItem, String> {
    let item_id = match item.get("itemId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return Err(String::from("stream event is missing itemId")),
    };
    let kind = match item.get("kind").and_then(Value::as_str) {
        Some(kind) => kind,
        None => return Err(String::from("stream event is missing kind")),
    };
    match kind {
        "userMessage" => Ok(StreamItem::UserMessage {
            item_id,
            text: item_text(item, "text"),
        }),
        "agentMessage" => Ok(StreamItem::AgentMessage {
            item_id,
            text: item_text(item, "text"),
            phase: item
                .get("messagePhase")
                .and_then(Value::as_str)
                .map(String::from),
        }),
        "reasoning" => Ok(StreamItem::Reasoning {
            item_id,
            summary: item
                .get("summary")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| entry.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        }),
        "toolCall" => {
            let args_json = match item.get("args") {
                None => String::new(),
                Some(Value::String(args)) => args.clone(),
                Some(other) => other.to_string(),
            };
            Ok(StreamItem::ToolCall {
                item_id,
                tool: item_text(item, "tool"),
                headline: headline_from_args(&args_json),
                args_json,
                status: map_item_status(item),
                output: item
                    .get("visibleOutput")
                    .and_then(Value::as_str)
                    .map(unwrap_output),
                failure: item
                    .get("failureReason")
                    .and_then(Value::as_str)
                    .map(String::from),
            })
        }
        "reminderChild" => Ok(StreamItem::System {
            item_id,
            kind: kind.to_string(),
        }),
        _ => Ok(StreamItem::Unknown {
            item_id,
            kind: kind.to_string(),
        }),
    }
}

/// Map `notifications/muse/event` params into a domain stream event.
///
/// `Ok(None)` means the event is one this client ignores (an unknown
/// event type, usage kind, or pending kind/phase). `Err` means the
/// event was malformed.
pub(crate) fn map_stream_event(params: &Value) -> Result<Option<StreamEvent>, String> {
    let kind = match params.get("type").and_then(Value::as_str) {
        Some(kind) => kind,
        None => return Err(String::from("stream event is missing its type")),
    };

    match kind {
        "turn-started" => {
            let (session, turn, view_cursor) = parts(params)?;
            Ok(Some(StreamEvent::TurnStarted {
                session,
                turn,
                view_cursor,
            }))
        }
        "item-started" | "item-completed" => {
            let (session, turn, view_cursor) = parts(params)?;
            let item = params
                .get("item")
                .ok_or_else(|| String::from("stream event is missing its item"))?;
            // Item events carry no envelope turn id in captures; the
            // item object's own turn id joins the event to its turn.
            let turn = turn.or_else(|| item.get("turnId").and_then(Value::as_str).map(TurnId::new));
            let item = parse_stream_item(item)?;
            if kind == "item-started" {
                Ok(Some(StreamEvent::ItemStarted {
                    session,
                    turn,
                    view_cursor,
                    item,
                }))
            } else {
                Ok(Some(StreamEvent::ItemCompleted {
                    session,
                    turn,
                    view_cursor,
                    item,
                }))
            }
        }
        "item-delta" => {
            let (session, turn, view_cursor) = parts(params)?;
            let item_id = req_string(params, "itemId")?;
            let delta = params
                .get("delta")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from("stream event is missing its delta"))?;
            let field = opt_string(params, "field").unwrap_or_else(|| String::from("text"));
            Ok(Some(StreamEvent::ItemDelta {
                session,
                turn,
                view_cursor,
                item_id,
                field,
                delta: delta.to_string(),
            }))
        }
        "turn-completed" => {
            let (session, turn, view_cursor) = parts(params)?;
            Ok(Some(StreamEvent::TurnCompleted {
                session,
                turn,
                view_cursor,
                terminal: opt_string(params, "terminal"),
                answer: opt_string(params, "answer"),
                dropped_deltas: params
                    .get("droppedDeltas")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            }))
        }
        "usage" => {
            let (session, turn, view_cursor) = parts(params)?;
            let usage_kind = match params.get("usageKind").and_then(Value::as_str) {
                Some("tokenUsage") => UsageKind::TokenUsage,
                Some("changed") => UsageKind::Changed,
                _ => return Ok(None),
            };
            let usage = params
                .get("usage")
                .ok_or_else(|| String::from("stream event is missing its usage"))?;
            Ok(Some(StreamEvent::Usage {
                session,
                turn,
                view_cursor,
                kind: usage_kind,
                usage_json: usage.to_string(),
            }))
        }
        "pending" => {
            let (session, turn, _) = parts(params)?;
            let request_id = req_string(params, "requestId")?;
            let kind = match params.get("kind").and_then(Value::as_str) {
                Some("approval") => PendingKind::Approval,
                Some("userInput") => PendingKind::UserInput,
                _ => return Ok(None),
            };
            let phase = match params.get("phase").and_then(Value::as_str) {
                Some("requested") => PendingPhase::Requested,
                Some("settled") => PendingPhase::Settled,
                _ => return Ok(None),
            };
            Ok(Some(StreamEvent::Pending {
                session,
                turn,
                request_id,
                kind,
                phase,
            }))
        }
        _ => Ok(None),
    }
}

/// Fold one `muse_status` result value into engine events.
///
/// Returns the events to emit: diagnostics first, then either
/// `SessionsLoaded` or a single `SessionsLoadFailed`.
pub(crate) fn map_status_result(result: &Value) -> Vec<EngineEvent> {
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        return vec![EngineEvent::SessionsLoadFailed(
            Error::Tool(first_text(result)).to_string(),
        )];
    }
    let text = match first_text_opt(result) {
        Some(text) => text,
        None => {
            return vec![EngineEvent::SessionsLoadFailed(
                Error::Tool(String::from(
                    "Couldn't load your sessions from the Muse MCP server.",
                ))
                .to_string(),
            )];
        }
    };
    let rows: Vec<Value> = match serde_json::from_str(&text) {
        Ok(rows) => rows,
        Err(_) => {
            return vec![EngineEvent::SessionsLoadFailed(
                Error::Tool(String::from(
                    "Couldn't load your sessions from the Muse MCP server.",
                ))
                .to_string(),
            )];
        }
    };
    let (sessions, diagnostics) = map_status_rows(&rows);
    let mut events: Vec<EngineEvent> = diagnostics
        .into_iter()
        .map(EngineEvent::Diagnostic)
        .collect();
    events.push(EngineEvent::SessionsLoaded { sessions });
    events
}

/// Session and turn ids from a `muse_spawn` or `muse_turn` result body.
#[derive(Debug, Deserialize)]
struct CommandIds {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
}

/// Parse the content text of a spawn or turn result into raw ids.
///
/// A server-authored `isError` body passes through as the failure text;
/// anything else that does not parse fails with the friendly `generic`
/// message. Empty ids count as absent.
fn command_ids(result: &Value, generic: &str) -> Result<(Option<String>, Option<String>), String> {
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        let text = first_text_opt(result).unwrap_or_else(|| String::from(generic));
        return Err(Error::Tool(text).to_string());
    }
    let text = match first_text_opt(result) {
        Some(text) => text,
        None => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let ids: CommandIds = match serde_json::from_str(&text) {
        Ok(ids) => ids,
        Err(_) => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let session = ids.session_id.filter(|id| !id.is_empty());
    let turn = ids.turn_id.filter(|id| !id.is_empty());
    Ok((session, turn))
}

/// Map a `muse_spawn` result into the spawned session and first turn.
pub(crate) fn map_spawn_result(result: &Value) -> Result<(SessionId, Option<TurnId>), String> {
    let generic = "Couldn't start a session on the Muse MCP server.";
    let (session, turn) = command_ids(result, generic)?;
    match session {
        Some(session) => Ok((SessionId::new(session), turn.map(TurnId::new))),
        None => Err(Error::Tool(String::from(generic)).to_string()),
    }
}

/// Map a `muse_turn` result into the accepted session and turn.
///
/// The session comes from the result's `session_id`; when the result
/// carries none, the commanded session is kept so the acceptance still
/// joins its stream.
pub(crate) fn map_turn_result(
    result: &Value,
    commanded: &SessionId,
) -> Result<(SessionId, Option<TurnId>), String> {
    let generic = "Couldn't send that turn to the Muse MCP server.";
    let (session, turn) = command_ids(result, generic)?;
    Ok((
        session
            .map(SessionId::new)
            .unwrap_or_else(|| commanded.clone()),
        turn.map(TurnId::new),
    ))
}

/// Map a `muse_models` session-resume result into the resumed session.
///
/// The commanded session is kept: the result body is a discarded
/// catalog, and its `isError` flag is the only signal. A
/// server-authored `isError` body passes through as the failure text.
pub(crate) fn map_resume_result(
    result: &Value,
    commanded: &SessionId,
) -> Result<SessionId, String> {
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        let text = first_text_opt(result).unwrap_or_else(|| {
            String::from("Couldn't resume that session on the Muse MCP server.")
        });
        return Err(Error::Tool(text).to_string());
    }
    Ok(commanded.clone())
}

/// Text of the first `type == "text"` content item, if any.
fn first_text_opt(result: &Value) -> Option<String> {
    result
        .get("content")?
        .as_array()?
        .iter()
        .find(|item| item.get("type") == Some(&Value::String(String::from("text"))))?
        .get("text")?
        .as_str()
        .map(String::from)
}

/// Text of the first `type == "text"` content item, or the fallback.
fn first_text(result: &Value) -> String {
    first_text_opt(result)
        .unwrap_or_else(|| String::from("Couldn't load your sessions from the Muse MCP server."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(
        session_id: &str,
        status: &str,
        workspace: &str,
        pending_approvals: u64,
        dropped: Option<u64>,
    ) -> Value {
        let mut row = json!({
            "session_id": session_id,
            "status": status,
            "workspace": workspace,
            "pending_approvals": pending_approvals,
            "pending_user_inputs": 0,
        });
        if let Some(dropped) = dropped {
            row["stream_dropped_deltas"] = json!(dropped);
        }
        row
    }

    #[test]
    fn orphaned_stays_orphaned_and_closed_is_excluded() {
        let rows = vec![
            row("a", "orphaned", "/w/p", 0, None),
            row("b", "closed", "/w/p", 0, None),
        ];
        let (sessions, diagnostics) = map_status_rows(&rows);
        assert!(diagnostics.is_empty());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Orphaned);
    }

    /// Guards against overflow when the server reports huge pending counts.
    #[test]
    fn huge_pending_counts_do_not_overflow() {
        let rows = vec![json!({
            "session_id": "max-1",
            "status": "running",
            "workspace": "/w/p",
            "pending_approvals": u64::MAX,
            "pending_user_inputs": u64::MAX,
        })];
        let (sessions, _) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::AwaitingApproval);
    }

    #[test]
    fn pending_overrides_running_and_unknown_is_a_diagnostic() {
        let rows = vec![
            row("p1", "running", "/w/p", 2, None),
            row("weird", "exploded", "/w/p", 0, None),
        ];
        let (sessions, diagnostics) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::AwaitingApproval);
        assert_eq!(
            diagnostics,
            vec![String::from(
                "A session had an unrecognized status (`exploded`) and was skipped."
            )]
        );
    }

    #[test]
    fn dto_errors_skip_with_diagnostic() {
        let rows = vec![json!({"status": "running"})];
        let (sessions, diagnostics) = map_status_rows(&rows);
        assert!(sessions.is_empty());
        assert_eq!(
            diagnostics,
            vec![String::from(
                "A session from the server couldn't be read and was skipped."
            )]
        );
    }

    #[test]
    fn rows_keep_workspace_with_short_id_titles_and_no_project() {
        let rows = vec![
            row("s1", "idle", "/home/dev/beta", 0, None),
            row("s2", "idle", "/home/dev/alpha", 0, None),
            row("s3", "idle", "/home/dev/beta", 0, None),
        ];
        let (sessions, _) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].workspace, String::from("/home/dev/beta"));
        assert_eq!(sessions[0].title, String::from("s1"));
        assert_eq!(sessions[0].project, None);
        assert_eq!(sessions[2].workspace, String::from("/home/dev/beta"));
        assert_eq!(sessions[2].title, String::from("s3"));
        assert_eq!(sessions[2].project, None);
    }

    #[test]
    fn orphaned_rows_keep_workspace() {
        let rows = vec![row(
            "sess-orphan",
            "orphaned",
            "/home/dev/storefront-web",
            0,
            None,
        )];
        let (sessions, _) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Orphaned);
        assert_eq!(
            sessions[0].workspace,
            String::from("/home/dev/storefront-web")
        );
        assert_eq!(sessions[0].project, None);
    }

    #[test]
    fn title_truncates_by_chars_not_bytes() {
        let rows = vec![row("ééééééééXYZ", "idle", "/w/long-name", 0, Some(4))];
        let (sessions, _) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "éééééééé");
        assert_eq!(sessions[0].workspace, String::from("/w/long-name"));
        assert_eq!(sessions[0].stream_dropped_deltas, 4);
        assert_eq!(sessions[0].relative_time, String::new());
    }

    #[test]
    fn resume_result_keeps_commanded_session() {
        let commanded = SessionId::new("sess-orphan");
        let ok = json!({"content": [{"type": "text", "text": "catalog"}]});
        assert_eq!(map_resume_result(&ok, &commanded), Ok(commanded.clone()));
        // No isError flag at all still resumes.
        let bare = json!({"content": []});
        assert_eq!(map_resume_result(&bare, &commanded), Ok(commanded.clone()));
    }

    #[test]
    fn resume_result_failures_carry_server_or_generic_text() {
        let commanded = SessionId::new("sess-orphan");
        let server = json!({
            "content": [{"type": "text", "text": "session sess-orphan is in use"}],
            "isError": true,
        });
        assert_eq!(
            map_resume_result(&server, &commanded),
            Err(String::from("session sess-orphan is in use"))
        );
        let empty = json!({"content": [], "isError": true});
        assert_eq!(
            map_resume_result(&empty, &commanded),
            Err(String::from(
                "Couldn't resume that session on the Muse MCP server."
            ))
        );
    }

    #[test]
    fn stream_unknown_type_is_none_and_missing_session_is_err() {
        let unknown = json!({"type": "from-the-future", "sessionId": "s"});
        assert_eq!(map_stream_event(&unknown).expect("ok"), None);
        // Unknown types ignore the event before any `sessionId` is required.
        let unknown_without_session = json!({"type": "from-the-future"});
        assert_eq!(
            map_stream_event(&unknown_without_session).expect("ok"),
            None
        );
        let missing = json!({"type": "item-delta", "itemId": "i", "delta": "d"});
        assert!(map_stream_event(&missing).is_err());
        let non_string_type = json!({"type": 7});
        assert!(map_stream_event(&non_string_type).is_err());
    }

    #[test]
    fn stream_item_delta_defaults_field_to_text() {
        let event = map_stream_event(&json!({
            "type": "item-delta",
            "sessionId": "s",
            "itemId": "i",
            "delta": "",
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            event,
            StreamEvent::ItemDelta {
                session: SessionId::new("s"),
                turn: None,
                view_cursor: None,
                item_id: String::from("i"),
                field: String::from("text"),
                delta: String::new(),
            }
        );
    }

    #[test]
    fn stream_pending_unknown_kind_or_phase_is_none() {
        let base = json!({
            "type": "pending",
            "sessionId": "s",
            "requestId": "r",
        });
        let mut kind = base.clone();
        kind["kind"] = json!("telepathy");
        kind["phase"] = json!("requested");
        assert_eq!(map_stream_event(&kind).expect("ok"), None);
        let mut phase = base.clone();
        phase["kind"] = json!("approval");
        phase["phase"] = json!("eventually");
        assert_eq!(map_stream_event(&phase).expect("ok"), None);
        let mut missing_kind = base.clone();
        missing_kind["phase"] = json!("requested");
        assert_eq!(map_stream_event(&missing_kind).expect("ok"), None);
    }

    #[test]
    fn status_result_emits_diagnostics_before_sessions_loaded() {
        let result = json!({
            "content": [{"type": "text", "text": serde_json::to_string(&vec![
                row("ok-1", "idle", "/w/p", 0, None),
                row("bad-1", "exploded", "/w/p", 0, None),
            ]).expect("rows")}],
            "isError": false,
        });
        let events = map_status_result(&result);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], EngineEvent::Diagnostic(_)));
        assert!(matches!(events[1], EngineEvent::SessionsLoaded { .. }));
    }

    #[test]
    fn stream_turn_started_maps_view_cursor() {
        let event = map_stream_event(&json!({
            "type": "turn-started",
            "sessionId": "s",
            "turnId": "t",
            "viewCursor": "v4:s:1",
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            event,
            StreamEvent::TurnStarted {
                session: SessionId::new("s"),
                turn: Some(TurnId::new("t")),
                view_cursor: Some(String::from("v4:s:1")),
            }
        );
    }

    #[test]
    fn stream_user_message_completed() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "viewCursor": "v4:s:2",
            "item": {
                "itemId": "u-1",
                "kind": "userMessage",
                "status": "completed",
                "text": "Read the file note.txt",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            event,
            StreamEvent::ItemCompleted {
                session: SessionId::new("s"),
                turn: Some(TurnId::new("t")),
                view_cursor: Some(String::from("v4:s:2")),
                item: StreamItem::UserMessage {
                    item_id: String::from("u-1"),
                    text: String::from("Read the file note.txt"),
                },
            }
        );
    }

    #[test]
    fn stream_agent_message_started_and_completed() {
        let started = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {
                "itemId": "a-1",
                "kind": "agentMessage",
                "revision": 1,
                "status": "inProgress",
                "text": "",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            started,
            StreamEvent::ItemStarted {
                session: SessionId::new("s"),
                turn: Some(TurnId::new("t")),
                view_cursor: None,
                item: StreamItem::AgentMessage {
                    item_id: String::from("a-1"),
                    text: String::new(),
                    phase: None,
                },
            }
        );
        let completed = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "a-1",
                "kind": "agentMessage",
                "messagePhase": "finalAnswer",
                "revision": 2,
                "status": "completed",
                "text": "```\nThe quick brown fox\n```",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            completed,
            StreamEvent::ItemCompleted {
                session: SessionId::new("s"),
                turn: Some(TurnId::new("t")),
                view_cursor: None,
                item: StreamItem::AgentMessage {
                    item_id: String::from("a-1"),
                    text: String::from("```\nThe quick brown fox\n```"),
                    phase: Some(String::from("finalAnswer")),
                },
            }
        );
    }

    #[test]
    fn stream_reasoning_keeps_string_summaries_only() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "r-1",
                "kind": "reasoning",
                "status": "completed",
                "summary": ["Applying step-by-step arithmetic.", 7, ""],
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => assert_eq!(
                item,
                StreamItem::Reasoning {
                    item_id: String::from("r-1"),
                    summary: vec![
                        String::from("Applying step-by-step arithmetic."),
                        String::new(),
                    ],
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_read_file_tool_call_headlines_path() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-1",
                "kind": "toolCall",
                "status": "completed",
                "tool": "read_file",
                "args": "{\"path\":\"/tmp/ws/note.txt\"}",
                "visibleOutput": "Read text file `/tmp/ws/note.txt`.",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, turn, .. } => {
                assert_eq!(turn, Some(TurnId::new("t")));
                assert_eq!(
                    item,
                    StreamItem::ToolCall {
                        item_id: String::from("c-1"),
                        tool: String::from("read_file"),
                        headline: Some(String::from("/tmp/ws/note.txt")),
                        args_json: String::from("{\"path\":\"/tmp/ws/note.txt\"}"),
                        status: ItemStatus::Completed,
                        output: Some(String::from("Read text file `/tmp/ws/note.txt`.")),
                        failure: None,
                    }
                );
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_bash_tool_call_unwraps_output_envelope() {
        let envelope = json!({
            "chunk_id": "exec-1-1",
            "command": "ls -la /tmp/ws",
            "description": "List workspace directory contents",
            "exit_code": 0,
            "terminal_status": "completed",
            "output": "total 0",
        })
        .to_string();
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-2",
                "kind": "toolCall",
                "status": "completed",
                "tool": "bash",
                "args": "{\"command\":\"ls -la /tmp/ws\",\"description\":\"List workspace directory contents\"}",
                "visibleOutput": envelope,
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => assert_eq!(
                item,
                StreamItem::ToolCall {
                    item_id: String::from("c-2"),
                    tool: String::from("bash"),
                    headline: Some(String::from("List workspace directory contents")),
                    args_json: String::from(
                        "{\"command\":\"ls -la /tmp/ws\",\"description\":\"List workspace directory contents\"}"
                    ),
                    status: ItemStatus::Completed,
                    output: Some(String::from("total 0")),
                    failure: None,
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_failed_tool_call_carries_status_and_failure() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-3",
                "kind": "toolCall",
                "status": "failed",
                "tool": "read_file",
                "args": "{\"path\":\"missing-file.txt\"}",
                "visibleOutput": "tool failed: No such file or directory (os error 2)",
                "failureReason": "No such file or directory (os error 2)",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => assert_eq!(
                item,
                StreamItem::ToolCall {
                    item_id: String::from("c-3"),
                    tool: String::from("read_file"),
                    headline: Some(String::from("missing-file.txt")),
                    args_json: String::from("{\"path\":\"missing-file.txt\"}"),
                    status: ItemStatus::Failed,
                    output: Some(String::from(
                        "tool failed: No such file or directory (os error 2)"
                    )),
                    failure: Some(String::from("No such file or directory (os error 2)")),
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_tool_call_with_object_args_keeps_compact_json() {
        let event = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {
                "itemId": "c-4",
                "kind": "toolCall",
                "status": "inProgress",
                "tool": "read_file",
                "args": {"path": "/tmp/ws/note.txt"},
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemStarted { item, .. } => assert_eq!(
                item,
                StreamItem::ToolCall {
                    item_id: String::from("c-4"),
                    tool: String::from("read_file"),
                    headline: Some(String::from("/tmp/ws/note.txt")),
                    args_json: String::from("{\"path\":\"/tmp/ws/note.txt\"}"),
                    status: ItemStatus::InProgress,
                    output: None,
                    failure: None,
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_unknown_status_stays_a_value() {
        let event = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {
                "itemId": "c-5",
                "kind": "toolCall",
                "status": "paused",
                "tool": "bash",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemStarted { item, .. } => match item {
                StreamItem::ToolCall {
                    status, args_json, ..
                } => {
                    assert_eq!(status, ItemStatus::Other(String::from("paused")));
                    assert_eq!(args_json, String::new());
                }
                other => panic!("wrong item: {other:?}"),
            },
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_reminder_child_is_system_and_other_kinds_are_unknown() {
        let system = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {"itemId": "x-1", "kind": "reminderChild", "turnId": "t"},
        }))
        .expect("ok")
        .expect("known");
        match system {
            StreamEvent::ItemStarted { item, .. } => assert_eq!(
                item,
                StreamItem::System {
                    item_id: String::from("x-1"),
                    kind: String::from("reminderChild"),
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
        let unknown = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {"itemId": "x-2", "kind": "from-the-future", "turnId": "t"},
        }))
        .expect("ok")
        .expect("known");
        match unknown {
            StreamEvent::ItemStarted { item, .. } => assert_eq!(
                item,
                StreamItem::Unknown {
                    item_id: String::from("x-2"),
                    kind: String::from("from-the-future"),
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_item_requires_id_and_kind() {
        let missing_id = json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {"kind": "userMessage", "text": "hi"},
        });
        assert!(map_stream_event(&missing_id).is_err());
        let empty_id = json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {"itemId": "", "kind": "userMessage"},
        });
        assert!(map_stream_event(&empty_id).is_err());
        let missing_kind = json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {"itemId": "i"},
        });
        assert!(map_stream_event(&missing_kind).is_err());
        let missing_item = json!({"type": "item-completed", "sessionId": "s"});
        assert!(map_stream_event(&missing_item).is_err());
    }

    #[test]
    fn stream_envelope_turn_id_wins_over_item_turn_id() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "turnId": "envelope-turn",
            "item": {"itemId": "u-1", "kind": "userMessage", "turnId": "item-turn"},
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { turn, .. } => {
                assert_eq!(turn, Some(TurnId::new("envelope-turn")));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn spawn_result_maps_ids_and_turn_absent_is_none() {
        let body = serde_json::to_string(&json!({
            "session_id": "sess-1",
            "turn_id": "turn-1",
            "status": "running",
        }))
        .expect("body is JSON");
        let result = json!({"content": [{"type": "text", "text": body}]});
        assert_eq!(
            map_spawn_result(&result),
            Ok((SessionId::new("sess-1"), Some(TurnId::new("turn-1"))))
        );
        let no_turn = json!({"content": [{"type": "text", "text": "{\"session_id\":\"sess-1\"}"}]});
        assert_eq!(
            map_spawn_result(&no_turn),
            Ok((SessionId::new("sess-1"), None))
        );
    }

    #[test]
    fn spawn_result_failures_carry_server_or_generic_text() {
        let server = json!({
            "content": [{"type": "text", "text": "workspace does not exist: /nope"}],
            "isError": true,
        });
        assert_eq!(
            map_spawn_result(&server),
            Err(String::from("workspace does not exist: /nope"))
        );
        let garbage = json!({"content": [{"type": "text", "text": "not json"}]});
        assert_eq!(
            map_spawn_result(&garbage),
            Err(String::from(
                "Couldn't start a session on the Muse MCP server."
            ))
        );
        let no_session = json!({"content": [{"type": "text", "text": "{\"turn_id\":\"t\"}"}]});
        assert_eq!(
            map_spawn_result(&no_session),
            Err(String::from(
                "Couldn't start a session on the Muse MCP server."
            ))
        );
    }

    #[test]
    fn turn_result_prefers_result_session_then_commanded() {
        let commanded = SessionId::new("sess-cmd");
        let body = serde_json::to_string(&json!({
            "session_id": "sess-echo",
            "turn_id": "turn-2",
            "status": "running",
        }))
        .expect("body is JSON");
        let result = json!({"content": [{"type": "text", "text": body}]});
        assert_eq!(
            map_turn_result(&result, &commanded),
            Ok((SessionId::new("sess-echo"), Some(TurnId::new("turn-2"))))
        );
        let bare = json!({"content": [{"type": "text", "text": "{\"status\":\"running\"}"}]});
        assert_eq!(
            map_turn_result(&bare, &commanded),
            Ok((SessionId::new("sess-cmd"), None))
        );
        let server = json!({
            "content": [{"type": "text", "text": "unknown session"}],
            "isError": true,
        });
        assert_eq!(
            map_turn_result(&server, &commanded),
            Err(String::from("unknown session"))
        );
    }

    #[test]
    fn status_is_error_uses_first_text_or_fallback() {
        let result = json!({
            "content": [{"type": "text", "text": "status blew up"}],
            "isError": true,
        });
        assert_eq!(
            map_status_result(&result),
            vec![EngineEvent::SessionsLoadFailed(String::from(
                "status blew up"
            ))]
        );
        let empty = json!({"content": [], "isError": true});
        assert_eq!(
            map_status_result(&empty),
            vec![EngineEvent::SessionsLoadFailed(String::from(
                "Couldn't load your sessions from the Muse MCP server."
            ))]
        );
    }
}
