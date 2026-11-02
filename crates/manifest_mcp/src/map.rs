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
    ApprovalChoiceSummary, ApprovalRequest, EngineEvent, GoalInfo, GoalStatus, ItemStatus,
    ModelInfo, PendingKind, PendingPhase, PendingRequest, Question, QuestionOption,
    QuestionRequest, SessionId, SessionStatus, SessionSummary, SkillInfo, StreamEvent, StreamItem,
    TodoItem, TodoStatus, TurnId, UsageKind, UsageSnapshot,
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
    /// Cumulative token usage folded by the server from the session's
    /// live `session/tokenUsage`; absent or unparseable stays out of
    /// the carried vec, never a row failure.
    #[serde(default)]
    token_usage: Option<Value>,
}

/// Map raw `muse_status` rows into sessions, diagnostics, and the
/// usage snapshots that parsed.
///
/// Server row order is preserved; project grouping happens in the
/// shell via domain
/// [`assign_projects`](manifest_domain::assign_projects).
pub(crate) fn map_status_rows(
    rows: &[Value],
) -> (
    Vec<SessionSummary>,
    Vec<String>,
    Vec<(SessionId, UsageSnapshot)>,
) {
    let mut sessions = Vec::new();
    let mut diagnostics = Vec::new();
    let mut usage = Vec::new();

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
        // The stream carries the same value as `usage_json` text, so
        // the value's string form parses the same way. Rows without
        // usable token usage carry no entry; the shell keeps its
        // stored snapshot for those.
        if let Some(raw) = dto.token_usage.as_ref()
            && let Some(snapshot) = UsageSnapshot::parse(&raw.to_string())
        {
            usage.push((SessionId::new(&dto.session_id), snapshot));
        }
    }

    (sessions, diagnostics, usage)
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

/// Whether a tool name is the todo-list tool.
///
/// Matches the captured name verbatim (`write_todos`); a
/// case-insensitive `todo` containment guard accepts renames, in
/// addition to the verbatim match and never instead of it.
fn is_todo_tool(tool: &str) -> bool {
    tool == "write_todos" || tool.to_lowercase().contains("todo")
}

/// Parsed todo list from raw toolCall args JSON.
///
/// Returns `Some` only for the todo tool carrying a `$.todos` array of
/// `{text, status}` entries, with [`TodoStatus`] from the snake_case
/// entry status (`completed` is done; `in_progress` is in progress;
/// `pending` waits; `cancelled` is dropped). Anything else — another
/// tool, empty or malformed JSON, a missing or non-array `todos`, an
/// entry without string `text`/`status`, or an unrecognized status —
/// is `None`: an expected shape the shell ignores, never a
/// whole-item failure.
fn parse_todo_list(tool: &str, args_json: &str) -> Option<Vec<TodoItem>> {
    if !is_todo_tool(tool) {
        return None;
    }
    let entries = serde_json::from_str::<Value>(args_json)
        .ok()?
        .get("todos")?
        .as_array()?
        .clone();
    let mut todos = Vec::with_capacity(entries.len());
    for entry in &entries {
        let label = entry.get("text")?.as_str()?;
        let status = match entry.get("status")?.as_str()? {
            "completed" => TodoStatus::Done,
            "in_progress" => TodoStatus::InProgress,
            "pending" => TodoStatus::Pending,
            "cancelled" => TodoStatus::Cancelled,
            _ => return None,
        };
        todos.push(TodoItem {
            label: label.to_string(),
            status,
        });
    }
    Some(todos)
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

/// Parse one host goal block into a [`GoalInfo`].
///
/// `objective` is required and non-empty (a malformed block, like the
/// other required wire fields, fails the event); `percentComplete`
/// must be a JSON number and rides verbatim, even past 100 (the
/// renderer clamps). `status` parses leniently with
/// [`GoalStatus::parse`], defaulting to an empty [`GoalStatus::Other`]
/// when absent; `currentWork` and `nextWork` ride when present.
fn parse_goal(block: &Value) -> Result<GoalInfo, String> {
    let objective = match block.get("objective").and_then(Value::as_str) {
        Some(objective) if !objective.is_empty() => objective.to_string(),
        _ => return Err(String::from("stream event is missing goal objective")),
    };
    let percent_complete = match block.get("percentComplete").and_then(Value::as_f64) {
        Some(percent) => percent,
        None => return Err(String::from("stream event is missing goal percent")),
    };
    Ok(GoalInfo {
        objective,
        percent_complete,
        status: block
            .get("status")
            .and_then(Value::as_str)
            .map(GoalStatus::parse)
            .unwrap_or(GoalStatus::Other(String::new())),
        current_work: opt_string(block, "currentWork"),
        next_work: opt_string(block, "nextWork"),
    })
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
/// classifies as system bookkeeping; `compaction` classifies as a
/// [`StreamItem::Compaction`] whose terminal outcome and reason ride
/// as values (failures after admission arrive as the item's terminal
/// event, never as wire errors). Any other unknown kind stays an
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
            let tool = item_text(item, "tool");
            let todos = parse_todo_list(&tool, &args_json);
            Ok(StreamItem::ToolCall {
                item_id,
                tool,
                headline: headline_from_args(&args_json),
                args_json,
                todos,
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
        "compaction" => Ok(StreamItem::Compaction {
            item_id,
            outcome: field_str(item, "outcome").map(String::from),
            reason: field_str(item, "reason").map(String::from),
            tokens_before: item.get("tokensBefore").and_then(Value::as_u64),
            tokens_after: item.get("tokensAfter").and_then(Value::as_u64),
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
        "catalog-changed" => {
            let session = SessionId::new(req_string(params, "sessionId")?);
            Ok(Some(StreamEvent::CatalogChanged { session }))
        }
        "goal-changed" => {
            let session = SessionId::new(req_string(params, "sessionId")?);
            let goal = match params.get("goal") {
                // An absent or explicit-null goal clears the session's
                // goal, mirroring the host's nullable Goal block.
                None | Some(Value::Null) => None,
                Some(block) => Some(parse_goal(block)?),
            };
            Ok(Some(StreamEvent::GoalChanged { session, goal }))
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
    let (sessions, diagnostics, usage) = map_status_rows(&rows);
    let mut events: Vec<EngineEvent> = diagnostics
        .into_iter()
        .map(EngineEvent::Diagnostic)
        .collect();
    events.push(EngineEvent::SessionsLoaded { sessions, usage });
    events
}

/// Non-empty string field of an object, when present.
fn field_str<'a>(object: &'a Value, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// Map one approval payload into an [`ApprovalRequest`].
///
/// Reads the server's `ApprovalSubjectView` fields (`toolName`,
/// `subject`, `availableChoices`); unknown fields are ignored. The
/// subject line joins the subject parts the way the server's
/// `canonicalSubject` does, so the card shows what is gated.
fn map_approval_payload(payload: &Value) -> Option<ApprovalRequest> {
    let choices_value = payload.get("availableChoices")?.as_array()?;
    let mut choices = Vec::new();
    for choice in choices_value {
        let decision = field_str(choice, "decision")?;
        choices.push(ApprovalChoiceSummary {
            label: choice
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            decision: decision.to_string(),
            scope: choice
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            accepts_feedback: choice
                .get("acceptsFeedback")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    let subject = payload.get("subject");
    let subject_part = |key: &str| {
        subject
            .and_then(|subject| field_str(subject, key))
            .map(String::from)
    };
    let tool = field_str(payload, "toolName")
        .map(String::from)
        .or_else(|| subject_part("toolName"))
        .unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    for part in [
        field_str(payload, "toolName").map(String::from),
        subject_part("kind"),
        subject_part("command"),
        subject_part("host"),
        subject_part("path"),
        subject_part("toolName"),
        field_str(payload, "rawArgs").map(String::from),
    ]
    .into_iter()
    .flatten()
    {
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    let subject_line = if parts.is_empty() {
        tool.clone()
    } else {
        parts.join(" ")
    };
    Some(ApprovalRequest {
        tool,
        subject_line,
        choices,
    })
}

/// Map one question payload into a [`QuestionRequest`].
///
/// Reads the raw `userInput/request` params the broker holds
/// verbatim: `questions[].{id,question,options[].{label,description},
/// selection.mode}`. `header` stands in when `question` is absent.
/// Questions without a usable id or prompt are skipped; `None` when
/// no question survives.
fn map_question_payload(payload: &Value) -> Option<QuestionRequest> {
    let rows = payload.get("questions")?.as_array()?;
    let mut questions = Vec::new();
    for row in rows {
        let Some(id) = field_str(row, "id") else {
            continue;
        };
        let prompt = field_str(row, "question")
            .or_else(|| field_str(row, "header"))
            .unwrap_or_default();
        if prompt.is_empty() {
            continue;
        }
        let options = row
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| {
                        let label = field_str(option, "label")?;
                        Some(QuestionOption {
                            label: label.to_string(),
                            description: option
                                .get("description")
                                .and_then(Value::as_str)
                                .map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let multi = row
            .get("selection")
            .and_then(|selection| selection.get("mode"))
            .and_then(Value::as_str)
            == Some("multiple");
        questions.push(Question {
            id: id.to_string(),
            prompt: prompt.to_string(),
            options,
            multi,
        });
    }
    if questions.is_empty() {
        None
    } else {
        Some(QuestionRequest { questions })
    }
}

/// Map raw `muse_pending` rows into pending requests and diagnostics.
///
/// Each row is `{requestId, sessionId, kind, payload, ..}`. Unknown
/// fields are ignored, and an unknown `kind` is ignored (forward
/// compatibility, mirroring the stream hint). A malformed row yields
/// one diagnostic and never drops the rest of the list.
pub(crate) fn map_pending_rows(rows: &[Value]) -> (Vec<PendingRequest>, Vec<String>) {
    let mut requests = Vec::new();
    let mut diagnostics = Vec::new();
    for row in rows {
        let kind = row.get("kind").and_then(Value::as_str).unwrap_or_default();
        if kind != "approval" && kind != "userInput" {
            continue;
        }
        let request_id = field_str(row, "requestId").map(String::from).or_else(|| {
            row.get("payload")
                .and_then(|payload| {
                    field_str(payload, "approvalId").or_else(|| field_str(payload, "userInputId"))
                })
                .map(String::from)
        });
        let session_id = field_str(row, "sessionId").map(String::from).or_else(|| {
            row.get("payload")
                .and_then(|payload| field_str(payload, "sessionId"))
                .map(String::from)
        });
        let (Some(request_id), Some(session_id)) = (request_id, session_id) else {
            diagnostics.push(String::from(
                "A pending request from the server couldn't be read and was skipped.",
            ));
            continue;
        };
        let session = SessionId::new(session_id);
        let payload = row.get("payload").cloned().unwrap_or(Value::Null);
        let mapped = if kind == "approval" {
            map_approval_payload(&payload).map(|approval| PendingRequest::Approval {
                session: session.clone(),
                request_id: request_id.clone(),
                approval,
            })
        } else {
            map_question_payload(&payload).map(|question| PendingRequest::Question {
                session: session.clone(),
                request_id: request_id.clone(),
                question,
            })
        };
        match mapped {
            Some(request) => requests.push(request),
            None => diagnostics.push(String::from(
                "A pending request from the server couldn't be read and was skipped.",
            )),
        }
    }
    (requests, diagnostics)
}

/// Fold one `muse_pending` result value into requests and diagnostics.
///
/// The content text parses as the broker's JSON array. A
/// server-authored `isError` body passes through as the failure text;
/// anything else that does not parse fails with the friendly generic
/// message, mirroring [`map_models_result`].
pub(crate) fn map_pending_result(
    result: &Value,
) -> Result<(Vec<PendingRequest>, Vec<String>), String> {
    let generic = "Couldn't load pending requests from the Muse MCP server.";
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        let text = first_text_opt(result).unwrap_or_else(|| String::from(generic));
        return Err(Error::Tool(text).to_string());
    }
    let text = match first_text_opt(result) {
        Some(text) => text,
        None => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let rows: Vec<Value> = match serde_json::from_str(&text) {
        Ok(rows) => rows,
        Err(_) => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    Ok(map_pending_rows(&rows))
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

/// Map a `muse_steer` or `muse_cancel` result into its acknowledgement.
///
/// Either call succeeds with no payload worth rendering: any
/// non-`isError` result is `Ok(())`, and only failures carry text. A
/// server-authored `isError` body passes through as the failure text;
/// anything else fails with the friendly `generic` message.
pub(crate) fn map_ack_result(result: &Value, generic: &str) -> Result<(), String> {
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        let text = first_text_opt(result).unwrap_or_else(|| String::from(generic));
        return Err(Error::Tool(text).to_string());
    }
    Ok(())
}

/// Map a `muse_models` catalog result into domain catalog values.
///
/// The content text parses as the catalog object: a `models` array
/// whose rows carry `id`, `display_label`, `is_default`, and
/// `is_active` (the server's snake_case names). Rows without a
/// non-empty string `id` cannot round-trip as `model`, so they are
/// skipped; absent labels and flags default to empty and false, and
/// [`ModelInfo::label`] falls back to the id for display. A
/// server-authored `isError` body passes through as the failure text;
/// anything else that does not parse fails with the friendly generic
/// message.
pub(crate) fn map_models_result(result: &Value) -> Result<Vec<ModelInfo>, String> {
    let generic = "Couldn't load the model catalog from the Muse MCP server.";
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        let text = first_text_opt(result).unwrap_or_else(|| String::from(generic));
        return Err(Error::Tool(text).to_string());
    }
    let text = match first_text_opt(result) {
        Some(text) => text,
        None => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let body: Value = match serde_json::from_str(&text) {
        Ok(body) => body,
        Err(_) => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let rows = match body.get("models").and_then(Value::as_array) {
        Some(rows) => rows,
        None => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let mut models = Vec::new();
    for row in rows {
        let id = match row.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };
        models.push(ModelInfo {
            display_label: row
                .get("display_label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            is_default: row
                .get("is_default")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_active: row
                .get("is_active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            id,
        });
    }
    Ok(models)
}

/// Map a `muse_skills` catalog result into domain catalog values.
///
/// The content text parses as the catalog object: a `skills` array
/// whose rows carry `selector`, `display_name`, `description`,
/// `source`, `plugin_id`, and `argument_hint` (the server's
/// snake_case names). Rows with a missing or empty `selector` cannot
/// round-trip as a skill part, so they are skipped; an absent
/// `display_name` falls back to the selector (see
/// [`SkillInfo::label`); `description` and `source` default to empty
/// and `plugin_id`/`argument_hint` default to absent. A
/// server-authored `isError` body passes through as the failure text;
/// anything else that does not parse fails with the friendly generic
/// message.
pub(crate) fn map_skills_result(result: &Value) -> Result<Vec<SkillInfo>, String> {
    let generic = "Couldn't load the skill catalog from the Muse MCP server.";
    // Each message converts to String exactly once, at this boundary.
    if result.get("isError") == Some(&Value::Bool(true)) {
        let text = first_text_opt(result).unwrap_or_else(|| String::from(generic));
        return Err(Error::Tool(text).to_string());
    }
    let text = match first_text_opt(result) {
        Some(text) => text,
        None => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let body: Value = match serde_json::from_str(&text) {
        Ok(body) => body,
        Err(_) => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let rows = match body.get("skills").and_then(Value::as_array) {
        Some(rows) => rows,
        None => return Err(Error::Tool(String::from(generic)).to_string()),
    };
    let mut skills = Vec::new();
    for row in rows {
        let selector = match row.get("selector").and_then(Value::as_str) {
            Some(selector) if !selector.is_empty() => selector.to_string(),
            _ => continue,
        };
        let display_name = row
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&selector)
            .to_string();
        skills.push(SkillInfo {
            display_name,
            description: row
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            source: row
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            plugin_id: row
                .get("plugin_id")
                .and_then(Value::as_str)
                .map(String::from),
            argument_hint: row
                .get("argument_hint")
                .and_then(Value::as_str)
                .map(String::from),
            selector,
        });
    }
    Ok(skills)
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
    fn token_usage_parses_into_carried_snapshots() {
        let mut with_usage = row("u1", "idle", "/w/p", 0, None);
        with_usage["token_usage"] = json!({"total": 42, "limit": 200});
        let mut camel = row("u2", "running", "/w/q", 0, None);
        camel["token_usage"] = json!({"promptTokens": 10, "outputTokens": 5});
        let rows = vec![with_usage, camel, row("u3", "idle", "/w/r", 0, None)];
        let (sessions, diagnostics, usage) = map_status_rows(&rows);
        assert!(diagnostics.is_empty());
        assert_eq!(sessions.len(), 3);
        assert_eq!(usage.len(), 2);
        assert_eq!(usage[0].0, SessionId::new("u1"));
        assert_eq!(usage[0].1.used, 42);
        assert_eq!(usage[0].1.window, Some(200));
        assert_eq!(usage[1].0, SessionId::new("u2"));
        assert_eq!(usage[1].1.used, 15);
    }

    #[test]
    fn token_usage_without_counts_or_garbage_stays_out() {
        let mut empty = row("e1", "idle", "/w/p", 0, None);
        empty["token_usage"] = json!({});
        let mut number = row("e2", "idle", "/w/p", 0, None);
        number["token_usage"] = json!(7);
        let mut text = row("e3", "idle", "/w/p", 0, None);
        text["token_usage"] = json!("nope");
        let mut nil = row("e4", "idle", "/w/p", 0, None);
        nil["token_usage"] = Value::Null;
        let rows = vec![empty, number, text, nil, row("e5", "idle", "/w/p", 0, None)];
        let (sessions, diagnostics, usage) = map_status_rows(&rows);
        assert!(diagnostics.is_empty());
        assert_eq!(sessions.len(), 5);
        assert!(usage.is_empty());
    }

    #[test]
    fn orphaned_stays_orphaned_and_closed_is_excluded() {
        let rows = vec![
            row("a", "orphaned", "/w/p", 0, None),
            row("b", "closed", "/w/p", 0, None),
        ];
        let (sessions, diagnostics, _) = map_status_rows(&rows);
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
        let (sessions, _, _) = map_status_rows(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::AwaitingApproval);
    }

    #[test]
    fn pending_overrides_running_and_unknown_is_a_diagnostic() {
        let rows = vec![
            row("p1", "running", "/w/p", 2, None),
            row("weird", "exploded", "/w/p", 0, None),
        ];
        let (sessions, diagnostics, _) = map_status_rows(&rows);
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
        let (sessions, diagnostics, _) = map_status_rows(&rows);
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
        let (sessions, _, _) = map_status_rows(&rows);
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
        let (sessions, _, _) = map_status_rows(&rows);
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
        let (sessions, _, _) = map_status_rows(&rows);
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

    fn approval_row() -> Value {
        json!({
            "requestId": "req-a1",
            "sessionId": "sess-1",
            "kind": "approval",
            "receivedAt": 1700000000000_u64,
            "ageSec": 3,
            "futureField": true,
            "payload": {
                "approvalId": "req-a1",
                "sessionId": "sess-1",
                "toolName": "bash",
                "rawArgs": "{\"command\":\"ls\"}",
                "subject": {"kind": "command", "command": "ls -la /tmp/ws", "toolName": "bash"},
                "availableChoices": [
                    {"choiceId": "allow_once", "decision": "approved", "scope": "once", "label": "Allow once", "acceptsFeedback": false},
                    {"choiceId": "abort", "decision": "abort", "scope": "once", "label": "Reject", "acceptsFeedback": true},
                ],
                "extra": "ignored",
            },
        })
    }

    fn question_row() -> Value {
        json!({
            "requestId": "req-q1",
            "sessionId": "sess-1",
            "kind": "userInput",
            "payload": {
                "userInputId": "req-q1",
                "sessionId": "sess-1",
                "questions": [
                    {
                        "id": "color", "header": "Color", "question": "Pick a color",
                        "selection": {"mode": "single"},
                        "options": [
                            {"label": "Red", "description": "Warm"},
                            {"label": "None"},
                        ],
                    },
                    {
                        "id": "extras", "header": "Extras", "question": "Pick extras",
                        "selection": {"mode": "multiple"},
                        "options": [
                            {"label": "A", "description": "First"},
                            {"label": "B"},
                        ],
                    },
                ],
            },
        })
    }

    #[test]
    fn pending_approval_maps_tool_subject_and_choices() {
        let (requests, diagnostics) = map_pending_rows(&[approval_row()]);
        assert!(diagnostics.is_empty(), "saw {diagnostics:?}");
        assert_eq!(requests.len(), 1);
        match &requests[0] {
            PendingRequest::Approval {
                session,
                request_id,
                approval,
            } => {
                assert_eq!(session, &SessionId::new("sess-1"));
                assert_eq!(request_id, "req-a1");
                assert_eq!(approval.tool, "bash");
                assert!(
                    approval.subject_line.contains("ls -la /tmp/ws"),
                    "subject line names the command, saw {:?}",
                    approval.subject_line
                );
                assert_eq!(approval.choice_labels(), vec!["Allow once", "Reject"]);
                assert!(!approval.choices[0].accepts_feedback);
                assert!(approval.choices[1].accepts_feedback);
            }
            other => panic!("wrong request: {other:?}"),
        }
    }

    #[test]
    fn pending_question_maps_single_multi_and_descriptions() {
        let (requests, diagnostics) = map_pending_rows(&[question_row()]);
        assert!(diagnostics.is_empty(), "saw {diagnostics:?}");
        match &requests[0] {
            PendingRequest::Question { question, .. } => {
                assert_eq!(question.questions.len(), 2);
                let (first, second) = (&question.questions[0], &question.questions[1]);
                assert!(!first.multi);
                assert_eq!(first.prompt, "Pick a color");
                assert_eq!(first.option_labels(), vec!["Red", "None"]);
                assert_eq!(first.options[0].description.as_deref(), Some("Warm"),);
                assert_eq!(first.options[1].description, None);
                assert!(second.multi, "multiple mode maps to multi");
                assert_eq!(second.options[0].description.as_deref(), Some("First"));
            }
            other => panic!("wrong request: {other:?}"),
        }
    }

    #[test]
    fn pending_malformed_row_is_diagnostic_and_rest_survives() {
        let rows = vec![
            json!({"kind": "approval", "payload": {}}),
            approval_row(),
            json!({"requestId": "bad-q", "sessionId": "s", "kind": "userInput", "payload": {"questions": []}}),
        ];
        let (requests, diagnostics) = map_pending_rows(&rows);
        assert_eq!(requests.len(), 1, "good row survives malformed neighbours");
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|text| text.contains("couldn't be read")),
            "saw {diagnostics:?}"
        );
    }

    #[test]
    fn pending_unknown_kind_is_ignored() {
        let rows = vec![
            json!({"requestId": "x", "sessionId": "s", "kind": "telepathy", "payload": {}}),
            json!({"requestId": "y", "sessionId": "s", "payload": {}}),
        ];
        let (requests, diagnostics) = map_pending_rows(&rows);
        assert!(requests.is_empty());
        assert!(
            diagnostics.is_empty(),
            "unknown kinds stay silent, saw {diagnostics:?}"
        );
    }

    #[test]
    fn pending_result_parses_array_and_carries_errors() {
        let text =
            serde_json::to_string(&vec![approval_row(), question_row()]).expect("rows are JSON");
        let result = json!({
            "content": [{"type": "text", "text": text}],
            "isError": false,
        });
        let (requests, diagnostics) = map_pending_result(&result).expect("ok");
        assert_eq!(requests.len(), 2);
        assert!(diagnostics.is_empty());

        let failed = json!({
            "content": [{"type": "text", "text": "pending blew up"}],
            "isError": true,
        });
        assert_eq!(
            map_pending_result(&failed),
            Err(String::from("pending blew up"))
        );
        let garbled = json!({
            "content": [{"type": "text", "text": "not json"}],
            "isError": false,
        });
        assert_eq!(
            map_pending_result(&garbled),
            Err(String::from(
                "Couldn't load pending requests from the Muse MCP server."
            ))
        );
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
    fn status_result_carries_parsed_token_usage() {
        let mut with_usage = row("ok-1", "idle", "/w/p", 0, None);
        with_usage["token_usage"] = json!({"total": 9});
        let result = json!({
            "content": [{"type": "text", "text": serde_json::to_string(&vec![
                with_usage,
                row("ok-2", "idle", "/w/q", 0, None),
            ]).expect("rows")}],
            "isError": false,
        });
        let events = map_status_result(&result);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EngineEvent::SessionsLoaded { sessions, usage } => {
                assert_eq!(sessions.len(), 2);
                assert_eq!(usage.len(), 1);
                assert_eq!(usage[0].0, SessionId::new("ok-1"));
                assert_eq!(usage[0].1.used, 9);
            }
            other => panic!("expected SessionsLoaded, got {other:?}"),
        }
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
                        todos: None,
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
                    todos: None,
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
                    todos: None,
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
                    todos: None,
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

    /// Verbatim live capture (`plans/mfst-8/item-catalog.md`): the first
    /// `write_todos` call, started, carries the full 3-item pending list.
    #[test]
    fn stream_write_todos_started_carries_full_pending_list() {
        let args = serde_json::to_string(&json!({"todos": [
            {"text": "create the file task-one.txt containing the word alpha", "status": "pending"},
            {"text": "create the file task-two.txt containing the word beta", "status": "pending"},
            {"text": "create the file task-three.txt containing the word gamma", "status": "pending"},
        ]}))
        .expect("args are JSON");
        let event = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {
                "itemId": "01a1a282-2cc9-7112-a173-fde23091588b",
                "kind": "toolCall",
                "status": "inProgress",
                "tool": "write_todos",
                "args": args.clone(),
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemStarted { item, .. } => assert_eq!(
                item,
                StreamItem::ToolCall {
                    item_id: String::from("01a1a282-2cc9-7112-a173-fde23091588b"),
                    tool: String::from("write_todos"),
                    headline: None,
                    args_json: args,
                    todos: Some(vec![
                        TodoItem {
                            label: String::from(
                                "create the file task-one.txt containing the word alpha"
                            ),
                            status: TodoStatus::Pending,
                        },
                        TodoItem {
                            label: String::from(
                                "create the file task-two.txt containing the word beta"
                            ),
                            status: TodoStatus::Pending,
                        },
                        TodoItem {
                            label: String::from(
                                "create the file task-three.txt containing the word gamma"
                            ),
                            status: TodoStatus::Pending,
                        },
                    ]),
                    status: ItemStatus::InProgress,
                    output: None,
                    failure: None,
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    /// Mid-turn capture shape: `completed`/`in_progress`/`pending`
    /// entry statuses map to `Done`/`InProgress`/`Pending`.
    #[test]
    fn stream_write_todos_completed_maps_snake_case_statuses() {
        let args = serde_json::to_string(&json!({"todos": [
            {"text": "create the file task-one.txt containing the word alpha", "status": "completed"},
            {"text": "create the file task-two.txt containing the word beta", "status": "in_progress"},
            {"text": "create the file task-three.txt containing the word gamma", "status": "pending"},
        ]}))
        .expect("args are JSON");
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "01a1a282-47ea-7112-a173-fde23091588b",
                "kind": "toolCall",
                "status": "completed",
                "tool": "write_todos",
                "args": args,
                "visibleOutput": "{\"items\":3,\"ok\":true,\"revision\":3}",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => match item {
                StreamItem::ToolCall { todos, output, .. } => {
                    let todos = todos.expect("todo tool carries todos");
                    assert_eq!(
                        todos.iter().map(|todo| todo.status).collect::<Vec<_>>(),
                        vec![
                            TodoStatus::Done,
                            TodoStatus::InProgress,
                            TodoStatus::Pending
                        ]
                    );
                    assert_eq!(
                        todos[0].label,
                        "create the file task-one.txt containing the word alpha"
                    );
                    assert_eq!(
                        output.as_deref(),
                        Some("{\"items\":3,\"ok\":true,\"revision\":3}")
                    );
                }
                other => panic!("wrong item: {other:?}"),
            },
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_todo_tool_name_matches_case_insensitively() {
        let args = serde_json::to_string(&json!({"todos": [
            {"text": "a step", "status": "completed"},
        ]}))
        .expect("args are JSON");
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-todo",
                "kind": "toolCall",
                "status": "completed",
                "tool": "Write_Todos",
                "args": args,
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => match item {
                StreamItem::ToolCall { todos, .. } => assert_eq!(
                    todos,
                    Some(vec![TodoItem {
                        label: String::from("a step"),
                        status: TodoStatus::Done,
                    }])
                ),
                other => panic!("wrong item: {other:?}"),
            },
            other => panic!("wrong event: {other:?}"),
        }
    }

    /// A non-todo tool carrying todos-looking args stays `None`: the
    /// tool name decides, never the args shape.
    #[test]
    fn stream_other_tool_with_todos_args_carries_no_todos() {
        let args = serde_json::to_string(&json!({"todos": [
            {"text": "a step", "status": "completed"},
        ]}))
        .expect("args are JSON");
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-other",
                "kind": "toolCall",
                "status": "completed",
                "tool": "read_file",
                "args": args,
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => match item {
                StreamItem::ToolCall { todos, .. } => assert_eq!(todos, None),
                other => panic!("wrong item: {other:?}"),
            },
            other => panic!("wrong event: {other:?}"),
        }
    }

    /// Malformed or unrecognized todo shapes degrade to `None` without
    /// failing the whole item.
    #[test]
    fn stream_malformed_todo_args_carry_no_todos() {
        for args in [
            String::from("not json{"),
            String::from("{\"path\":\"/tmp/ws/note.txt\"}"),
            String::from("{\"todos\":\"not-an-array\"}"),
            String::from("{\"todos\":[{\"status\":\"pending\"}]}"),
            String::from("{\"todos\":[{\"text\":\"a step\"}]}"),
            String::from("{\"todos\":[{\"text\":\"a step\",\"status\":\"done\"}]}"),
        ] {
            let event = map_stream_event(&json!({
                "type": "item-completed",
                "sessionId": "s",
                "item": {
                    "itemId": "c-bad",
                    "kind": "toolCall",
                    "status": "completed",
                    "tool": "write_todos",
                    "args": args,
                    "turnId": "t",
                },
            }))
            .expect("ok")
            .expect("known");
            match event {
                StreamEvent::ItemCompleted { item, .. } => match item {
                    StreamItem::ToolCall { todos, status, .. } => {
                        assert_eq!(todos, None, "args: {args}");
                        assert_eq!(status, ItemStatus::Completed);
                    }
                    other => panic!("wrong item: {other:?}"),
                },
                other => panic!("wrong event: {other:?}"),
            }
        }
    }

    /// An empty `todos` array is a cleared list, not a no-op.
    #[test]
    fn stream_empty_todos_array_is_some_empty() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-empty",
                "kind": "toolCall",
                "status": "completed",
                "tool": "write_todos",
                "args": "{\"todos\":[]}",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => match item {
                StreamItem::ToolCall { todos, .. } => assert_eq!(todos, Some(vec![])),
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
    fn stream_compaction_completed_carries_outcome_and_tokens() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-1",
                "kind": "compaction",
                "status": "completed",
                "outcome": "compacted",
                "trigger": "manual",
                "tokensBefore": 48210,
                "tokensAfter": 3912,
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => assert_eq!(
                item,
                StreamItem::Compaction {
                    item_id: String::from("c-1"),
                    outcome: Some(String::from("compacted")),
                    reason: None,
                    tokens_before: Some(48210),
                    tokens_after: Some(3912),
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_compaction_failed_carries_outcome_and_reason() {
        let event = map_stream_event(&json!({
            "type": "item-completed",
            "sessionId": "s",
            "item": {
                "itemId": "c-2",
                "kind": "compaction",
                "status": "failed",
                "outcome": "failed",
                "reason": "summarizer_failed",
                "turnId": "t",
            },
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemCompleted { item, .. } => assert_eq!(
                item,
                StreamItem::Compaction {
                    item_id: String::from("c-2"),
                    outcome: Some(String::from("failed")),
                    reason: Some(String::from("summarizer_failed")),
                    tokens_before: None,
                    tokens_after: None,
                }
            ),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn stream_compaction_started_without_outcome_is_in_progress() {
        let event = map_stream_event(&json!({
            "type": "item-started",
            "sessionId": "s",
            "item": {"itemId": "c-3", "kind": "compaction", "turnId": "t"},
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::ItemStarted { item, .. } => assert_eq!(
                item,
                StreamItem::Compaction {
                    item_id: String::from("c-3"),
                    outcome: None,
                    reason: None,
                    tokens_before: None,
                    tokens_after: None,
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

    #[test]
    fn ack_result_accepts_any_non_error_body() {
        let generic = "Couldn't steer that turn on the Muse MCP server.";
        let ok = json!({"content": [{"type": "text", "text": "{}"}]});
        assert_eq!(map_ack_result(&ok, generic), Ok(()));
        let bare = json!({"content": []});
        assert_eq!(map_ack_result(&bare, generic), Ok(()));
    }

    #[test]
    fn ack_result_failures_carry_server_or_generic_text() {
        let generic = "Couldn't cancel that turn on the Muse MCP server.";
        let server = json!({
            "content": [{"type": "text", "text": "session sess-1 is idle"}],
            "isError": true,
        });
        assert_eq!(
            map_ack_result(&server, generic),
            Err(String::from("session sess-1 is idle"))
        );
        let empty = json!({"content": [], "isError": true});
        assert_eq!(map_ack_result(&empty, generic), Err(String::from(generic)));
    }

    #[test]
    fn models_result_maps_catalog_rows_in_order() {
        let body = serde_json::to_string(&json!({
            "models": [
                {
                    "id": "muse-spark",
                    "display_label": "Muse Spark",
                    "is_default": true,
                    "is_active": false,
                },
                {"id": "other-model"},
            ],
            "provider_id": "muse",
            "source": "host",
        }))
        .expect("body is JSON");
        let result = json!({"content": [{"type": "text", "text": body}]});
        let models = map_models_result(&result).expect("catalog parses");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "muse-spark");
        assert_eq!(models[0].label(), "Muse Spark");
        assert!(models[0].is_default);
        assert!(!models[0].is_active);
        assert_eq!(models[1].id, "other-model");
        assert_eq!(models[1].label(), "other-model");
        assert!(!models[1].is_default);
        assert!(!models[1].is_active);
    }

    #[test]
    fn models_result_skips_rows_without_usable_ids() {
        let body = serde_json::to_string(&json!({
            "models": [
                {"display_label": "No id"},
                {"id": ""},
                {"id": 7},
                {"id": "kept", "display_label": "Kept", "is_active": true},
            ],
        }))
        .expect("body is JSON");
        let result = json!({"content": [{"type": "text", "text": body}]});
        let models = map_models_result(&result).expect("catalog parses");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "kept");
        assert!(models[0].is_active);
    }

    #[test]
    fn models_result_failures_carry_server_or_generic_text() {
        let generic = "Couldn't load the model catalog from the Muse MCP server.";
        let server = json!({
            "content": [{"type": "text", "text": "models requires workspace or session_id"}],
            "isError": true,
        });
        assert_eq!(
            map_models_result(&server),
            Err(String::from("models requires workspace or session_id"))
        );
        let garbage = json!({"content": [{"type": "text", "text": "not json"}]});
        assert_eq!(map_models_result(&garbage), Err(String::from(generic)));
        let no_models =
            json!({"content": [{"type": "text", "text": "{\"provider_id\":\"muse\"}"}]});
        assert_eq!(map_models_result(&no_models), Err(String::from(generic)));
        let empty = json!({"content": [], "isError": true});
        assert_eq!(map_models_result(&empty), Err(String::from(generic)));
    }

    #[test]
    fn skills_result_maps_catalog_rows_in_order() {
        let body = serde_json::to_string(&json!({
            "skills": [
                {
                    "selector": "review",
                    "display_name": "Review",
                    "description": "Reviews code",
                    "source": "user",
                    "plugin_id": "pr-tools",
                    "argument_hint": "<pr>",
                },
                {"selector": "bare"},
            ],
        }))
        .expect("body is JSON");
        let result = json!({"content": [{"type": "text", "text": body}]});
        let skills = map_skills_result(&result).expect("catalog parses");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].selector, "review");
        assert_eq!(skills[0].label(), "Review");
        assert_eq!(skills[0].description, "Reviews code");
        assert_eq!(skills[0].source, "user");
        assert_eq!(skills[0].plugin_id.as_deref(), Some("pr-tools"));
        assert_eq!(skills[0].argument_hint.as_deref(), Some("<pr>"));
        assert_eq!(skills[1].selector, "bare");
        assert_eq!(skills[1].label(), "bare");
        assert_eq!(skills[1].description, String::new());
        assert_eq!(skills[1].source, String::new());
        assert_eq!(skills[1].plugin_id, None);
        assert_eq!(skills[1].argument_hint, None);
    }

    #[test]
    fn skills_result_skips_rows_without_usable_selectors() {
        let body = serde_json::to_string(&json!({
            "skills": [
                {"display_name": "No selector"},
                {"selector": ""},
                {"selector": 7},
                {"selector": "kept", "display_name": "Kept", "source": "project"},
            ],
        }))
        .expect("body is JSON");
        let result = json!({"content": [{"type": "text", "text": body}]});
        let skills = map_skills_result(&result).expect("catalog parses");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].selector, "kept");
        assert_eq!(skills[0].source, "project");
    }

    #[test]
    fn skills_result_failures_carry_server_or_generic_text() {
        let generic = "Couldn't load the skill catalog from the Muse MCP server.";
        let server = json!({
            "content": [{"type": "text", "text": "skills requires session_id"}],
            "isError": true,
        });
        assert_eq!(
            map_skills_result(&server),
            Err(String::from("skills requires session_id"))
        );
        let garbage = json!({"content": [{"type": "text", "text": "not json"}]});
        assert_eq!(map_skills_result(&garbage), Err(String::from(generic)));
        let no_skills =
            json!({"content": [{"type": "text", "text": "{\"provider_id\":\"muse\"}"}]});
        assert_eq!(map_skills_result(&no_skills), Err(String::from(generic)));
        let empty = json!({"content": [], "isError": true});
        assert_eq!(map_skills_result(&empty), Err(String::from(generic)));
    }

    #[test]
    fn stream_goal_changed_maps_full_block() {
        let event = map_stream_event(&json!({
            "type": "goal-changed",
            "sessionId": "sess-1",
            "goal": {
                "objective": "Ship the slice",
                "percentComplete": 42,
                "status": "active",
                "currentWork": "Wiring the mapper",
                "nextWork": "Writing tests",
            },
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            event,
            StreamEvent::GoalChanged {
                session: SessionId::new("sess-1"),
                goal: Some(GoalInfo {
                    objective: String::from("Ship the slice"),
                    percent_complete: 42.0,
                    status: GoalStatus::Active,
                    current_work: Some(String::from("Wiring the mapper")),
                    next_work: Some(String::from("Writing tests")),
                }),
            }
        );
    }

    #[test]
    fn stream_goal_changed_null_clears() {
        // An explicit-null goal clears the session's goal.
        let event = map_stream_event(&json!({
            "type": "goal-changed",
            "sessionId": "sess-1",
            "goal": null,
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            event,
            StreamEvent::GoalChanged {
                session: SessionId::new("sess-1"),
                goal: None,
            }
        );
        // An absent goal clears too.
        let event = map_stream_event(&json!({"type": "goal-changed", "sessionId": "sess-1"}))
            .expect("ok")
            .expect("known");
        assert_eq!(
            event,
            StreamEvent::GoalChanged {
                session: SessionId::new("sess-1"),
                goal: None,
            }
        );
    }

    #[test]
    fn stream_goal_changed_carries_over_100_verbatim() {
        let event = map_stream_event(&json!({
            "type": "goal-changed",
            "sessionId": "sess-1",
            "goal": {"objective": "Ship it", "percentComplete": 150, "status": "weird"},
        }))
        .expect("ok")
        .expect("known");
        match event {
            StreamEvent::GoalChanged {
                goal: Some(goal), ..
            } => {
                assert_eq!(goal.percent_complete, 150.0);
                assert_eq!(goal.clamped_percent(), 100.0);
                assert_eq!(goal.status, GoalStatus::Other(String::from("weird")));
                assert_eq!(goal.current_work, None);
            }
            other => panic!("expected a goal, got {other:?}"),
        }
    }

    #[test]
    fn stream_goal_changed_malformed_block_is_err() {
        // Missing or empty objective fails, like the other required
        // wire fields.
        let missing_objective = json!({
            "type": "goal-changed",
            "sessionId": "sess-1",
            "goal": {"percentComplete": 10, "status": "active"},
        });
        assert!(map_stream_event(&missing_objective).is_err());
        let empty_objective = json!({
            "type": "goal-changed",
            "sessionId": "sess-1",
            "goal": {"objective": "", "percentComplete": 10},
        });
        assert!(map_stream_event(&empty_objective).is_err());
        let missing_percent = json!({
            "type": "goal-changed",
            "sessionId": "sess-1",
            "goal": {"objective": "Ship it"},
        });
        assert!(map_stream_event(&missing_percent).is_err());
        let missing_session = json!({
            "type": "goal-changed",
            "goal": {"objective": "Ship it", "percentComplete": 10},
        });
        assert!(map_stream_event(&missing_session).is_err());
    }

    #[test]
    fn stream_catalog_changed_maps_to_session_event() {
        let event = map_stream_event(&json!({
            "type": "catalog-changed",
            "sessionId": "sess-1",
        }))
        .expect("ok")
        .expect("known");
        assert_eq!(
            event,
            StreamEvent::CatalogChanged {
                session: SessionId::new("sess-1"),
            }
        );
        let missing = map_stream_event(&json!({"type": "catalog-changed"}));
        assert!(missing.is_err());
    }
}
