//! Crate-private mapping from wire values into domain values.
//!
//! The wire has no timestamps and no project objects, so project
//! grouping is derived from each session row's workspace, and row
//! titles pair the derived project name with the session id. Anything
//! the wire cannot explain becomes a [`Diagnostic`] value rather than
//! a silent drop, except `closed` sessions, which are expected
//! filtering.

use crate::error::Error;
use manifest_domain::{
    EngineEvent, PendingKind, PendingPhase, ProjectId, ProjectSummary, SessionId, SessionStatus,
    SessionSummary, StreamEvent, TurnId, UsageKind,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

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

/// Short human name for a workspace: its last non-empty `/`-segment.
fn project_name(workspace: &str) -> &str {
    workspace
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(workspace)
}

/// Map raw `muse_status` rows into projects, sessions, and diagnostics.
///
/// Server row order is preserved in the flat sessions vec; projects
/// appear in first-seen order.
pub(crate) fn map_status_rows(
    rows: &[Value],
) -> (Vec<ProjectSummary>, Vec<SessionSummary>, Vec<String>) {
    let mut projects = Vec::new();
    let mut project_ids: HashMap<String, ProjectId> = HashMap::new();
    let mut project_names: HashMap<String, String> = HashMap::new();
    let mut sessions = Vec::new();
    let mut diagnostics = Vec::new();

    for row in rows {
        let dto: StatusDto = match serde_json::from_value(row.clone()) {
            Ok(dto) => dto,
            Err(err) => {
                diagnostics.push(format!("Skipped a session row from muse_status: {err}"));
                continue;
            }
        };
        if dto.status == "closed" {
            continue;
        }
        let status = match dto.status.as_str() {
            "idle" | "orphaned" => SessionStatus::Idle,
            "running" => SessionStatus::Running,
            other => {
                diagnostics.push(format!(
                    "Skipped session `{}` from muse_status: unknown status `{other}`",
                    dto.session_id,
                ));
                continue;
            }
        };
        let status = if dto.pending_approvals + dto.pending_user_inputs > 0 {
            SessionStatus::AwaitingApproval
        } else {
            status
        };
        let project_id = project_ids.entry(dto.workspace.clone()).or_insert_with(|| {
            let name = project_name(&dto.workspace).to_string();
            project_names.insert(dto.workspace.clone(), name.clone());
            projects.push(ProjectSummary {
                id: ProjectId::new(format!("workspace:{}", dto.workspace)),
                name: name.clone(),
            });
            ProjectId::new(format!("workspace:{}", dto.workspace))
        });
        let name = project_names
            .get(&dto.workspace)
            .cloned()
            .unwrap_or_else(|| dto.workspace.clone());
        // First 8 chars (not bytes) so multi-byte ids truncate safely.
        let short: String = dto.session_id.chars().take(8).collect();
        sessions.push(SessionSummary {
            id: SessionId::new(&dto.session_id),
            title: format!("{name} · {short}"),
            status,
            project: Some(project_id.clone()),
            relative_time: String::new(),
            stream_dropped_deltas: dto.stream_dropped_deltas.unwrap_or(0),
        });
    }

    (projects, sessions, diagnostics)
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
    if !matches!(
        kind,
        "turn-started"
            | "item-started"
            | "item-delta"
            | "item-completed"
            | "turn-completed"
            | "usage"
            | "pending"
    ) {
        return Ok(None);
    }
    let session_id = req_string(params, "sessionId")?;
    let session = SessionId::new(session_id);
    let turn = opt_string(params, "turnId").map(TurnId::new);
    let view_cursor = opt_string(params, "viewCursor");

    match kind {
        "turn-started" => Ok(Some(StreamEvent::TurnStarted { session, turn })),
        "item-started" | "item-completed" => {
            let item = params
                .get("item")
                .ok_or_else(|| String::from("stream event is missing its item"))?;
            let item_json = item.to_string();
            if kind == "item-started" {
                Ok(Some(StreamEvent::ItemStarted {
                    session,
                    turn,
                    view_cursor,
                    item_json,
                }))
            } else {
                Ok(Some(StreamEvent::ItemCompleted {
                    session,
                    turn,
                    view_cursor,
                    item_json,
                }))
            }
        }
        "item-delta" => {
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
        "turn-completed" => Ok(Some(StreamEvent::TurnCompleted {
            session,
            turn,
            view_cursor,
            terminal: opt_string(params, "terminal"),
            answer: opt_string(params, "answer"),
            dropped_deltas: params
                .get("droppedDeltas")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        })),
        "usage" => {
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
        // Covered by the known-type gate above; unknown types return early.
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
                    "Could not parse the muse_status result: no text content item",
                ))
                .to_string(),
            )];
        }
    };
    let rows: Vec<Value> = match serde_json::from_str(&text) {
        Ok(rows) => rows,
        Err(err) => {
            return vec![EngineEvent::SessionsLoadFailed(
                Error::Tool(format!("Could not parse the muse_status result: {err}")).to_string(),
            )];
        }
    };
    let (projects, sessions, diagnostics) = map_status_rows(&rows);
    let mut events: Vec<EngineEvent> = diagnostics
        .into_iter()
        .map(EngineEvent::Diagnostic)
        .collect();
    events.push(EngineEvent::SessionsLoaded { projects, sessions });
    events
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
    first_text_opt(result).unwrap_or_else(|| String::from("muse_status failed"))
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
    fn orphaned_maps_to_idle_and_closed_is_excluded() {
        let rows = vec![
            row("a", "orphaned", "/w/p", 0, None),
            row("b", "closed", "/w/p", 0, None),
        ];
        let (projects, sessions, diagnostics) = map_status_rows(&rows);
        assert!(diagnostics.is_empty());
        assert_eq!(projects.len(), 1);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Idle);
    }

    #[test]
    fn pending_overrides_running_and_unknown_is_a_diagnostic() {
        let rows = vec![
            row("p1", "running", "/w/p", 2, None),
            row("weird", "exploded", "/w/p", 0, None),
        ];
        let (_, sessions, diagnostics) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::AwaitingApproval);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("weird"));
        assert!(diagnostics[0].contains("exploded"));
    }

    #[test]
    fn dto_errors_skip_with_diagnostic() {
        let rows = vec![json!({"status": "running"})];
        let (_, sessions, diagnostics) = map_status_rows(&rows);
        assert!(sessions.is_empty());
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn projects_group_in_first_seen_order_with_short_names() {
        let rows = vec![
            row("s1", "idle", "/home/dev/beta", 0, None),
            row("s2", "idle", "/home/dev/alpha", 0, None),
            row("s3", "idle", "/home/dev/beta", 0, None),
        ];
        let (projects, sessions, _) = map_status_rows(&rows);
        assert_eq!(
            projects.iter().map(|p| &p.name).collect::<Vec<_>>(),
            [&String::from("beta"), &String::from("alpha")]
        );
        assert_eq!(
            sessions[0].project,
            Some(ProjectId::new("workspace:/home/dev/beta"))
        );
        assert_eq!(
            sessions[2].project,
            Some(ProjectId::new("workspace:/home/dev/beta"))
        );
    }

    #[test]
    fn title_truncates_by_chars_not_bytes() {
        let rows = vec![row("ééééééééXYZ", "idle", "/w/long-name", 0, Some(4))];
        let (_, sessions, _) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "long-name · éééééééé");
        assert_eq!(sessions[0].stream_dropped_deltas, 4);
        assert_eq!(sessions[0].relative_time, String::new());
    }

    #[test]
    fn stream_unknown_type_is_none_and_missing_session_is_err() {
        let unknown = json!({"type": "from-the-future", "sessionId": "s"});
        assert_eq!(map_stream_event(&unknown).expect("ok"), None);
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
                "muse_status failed"
            ))]
        );
    }
}
