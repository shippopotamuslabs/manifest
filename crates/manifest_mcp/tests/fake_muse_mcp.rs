//! Scripted muse-code-mcp stand-in for transport tests.
//!
//! Mode via `FAKE_MCP_MODE` (`ok` by default): `exit-early` exits
//! immediately, `malformed` answers `initialize` with a non-JSON line,
//! `init-error` answers it with an error, and `eof-after-init` exits
//! right after the `initialized` notification. Otherwise the fake
//! replays a fixed event burst and scripted `muse_status` answers.

use std::io::{BufRead, BufReader, Write};

fn write_line(stdout: &mut std::io::StdoutLock<'_>, value: &serde_json::Value) {
    let mut line = serde_json::to_string(value).expect("fake reply is JSON");
    line.push('\n');
    stdout
        .write_all(line.as_bytes())
        .expect("fake writes stdout");
    stdout.flush().expect("fake flushes stdout");
}

fn write_raw(stdout: &mut std::io::StdoutLock<'_>, line: &str) {
    stdout
        .write_all(line.as_bytes())
        .expect("fake writes stdout");
    stdout.write_all(b"\n").expect("fake writes stdout");
    stdout.flush().expect("fake flushes stdout");
}

fn tool_result(
    stdout: &mut std::io::StdoutLock<'_>,
    call_id: &serde_json::Value,
    text: &str,
    is_error: bool,
) {
    write_line(
        stdout,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": call_id,
            "result": {
                "content": [{"type": "text", "text": text}],
                "isError": is_error,
            },
        }),
    );
}

fn notify(stdout: &mut std::io::StdoutLock<'_>, params: serde_json::Value) {
    write_line(
        stdout,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/muse/event",
            "params": params,
        }),
    );
}

fn cursor(session: &str, seq: u64) -> String {
    format!("v4:{session}:{seq}")
}

/// Full spawn stream for `fake-session-1` / `fake-turn-1`.
///
/// The user message echoes the spawn prompt; the reasoning summary is
/// one sentence; the bash call streams its output before completing
/// with a JSON envelope whose `output` equals the delta text; the final
/// answer is markdown with a fenced code block.
fn emit_spawn_stream(stdout: &mut std::io::StdoutLock<'_>, prompt: &str) {
    let session = "fake-session-1";
    let turn = "fake-turn-1";
    let summary = "Planning the turn, listing the workspace, and quoting the note.";
    let command = "ls -la /tmp/ws";
    let description = "List workspace directory contents";
    let args = serde_json::json!({"command": command, "description": description}).to_string();
    let ls_output = "total 0";
    let envelope = serde_json::json!({
        "chunk_id": "exec-1-1",
        "command": command,
        "description": description,
        "exit_code": 0,
        "terminal_status": "completed",
        "output": ls_output,
        "original_output_bytes": 7,
        "original_output_tokens": 2,
        "truncated": false,
    })
    .to_string();
    let answer = "Done. The workspace holds:\n\n```\ntotal 0\n```\n\n- listed\n- read\n- quoted\n";
    let (first, second) = answer.split_at(28);
    let mut n = 0_u64;
    let mut at = || {
        n += 1;
        cursor(session, n)
    };
    notify(
        stdout,
        serde_json::json!({
            "type": "turn-started",
            "sessionId": session,
            "turnId": turn,
            "viewCursor": at(),
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-completed",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-user-1",
                "kind": "userMessage",
                "recordedAt": "2026-11-02T21:00:00Z",
                "revision": 1,
                "status": "completed",
                "text": prompt,
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-started",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-reasoning-1",
                "kind": "reasoning",
                "revision": 1,
                "status": "inProgress",
                "summary": [""],
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-delta",
            "sessionId": session,
            "viewCursor": at(),
            "itemId": "fake-item-reasoning-1",
            "field": "summary.0",
            "delta": summary,
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-completed",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-reasoning-1",
                "kind": "reasoning",
                "recordedAt": "2026-11-02T21:00:01Z",
                "revision": 2,
                "status": "completed",
                "summary": [summary],
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-started",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-bash-1",
                "kind": "toolCall",
                "callId": "call_fakebash1",
                "recordedAt": "2026-11-02T21:00:02Z",
                "revision": 1,
                "status": "inProgress",
                "tool": "bash",
                "args": args,
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-delta",
            "sessionId": session,
            "viewCursor": at(),
            "itemId": "fake-item-bash-1",
            "field": "output",
            "delta": ls_output,
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-completed",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-bash-1",
                "kind": "toolCall",
                "callId": "call_fakebash1",
                "recordedAt": "2026-11-02T21:00:03Z",
                "revision": 3,
                "status": "completed",
                "tool": "bash",
                "args": args,
                "visibleOutput": envelope,
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-started",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-agent-1",
                "kind": "agentMessage",
                "revision": 1,
                "status": "inProgress",
                "text": "",
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-delta",
            "sessionId": session,
            "viewCursor": at(),
            "itemId": "fake-item-agent-1",
            "field": "text",
            "delta": first,
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-delta",
            "sessionId": session,
            "viewCursor": at(),
            "itemId": "fake-item-agent-1",
            "field": "text",
            "delta": second,
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-completed",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-agent-1",
                "kind": "agentMessage",
                "messagePhase": "finalAnswer",
                "recordedAt": "2026-11-02T21:00:04Z",
                "revision": 2,
                "status": "completed",
                "text": answer,
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "turn-completed",
            "sessionId": session,
            "turnId": turn,
            "viewCursor": at(),
            "terminal": "completed",
            "answer": answer,
            "droppedDeltas": 0,
        }),
    );
}

/// Echo text for a spawn or turn call: the `prompt` field, or the
/// `/selector arguments` rendering of the first skill part when the
/// call sent `parts` instead of `prompt`.
fn echo_text(arguments: &serde_json::Value) -> String {
    if let Some(prompt) = arguments.get("prompt").and_then(serde_json::Value::as_str)
        && !prompt.is_empty()
    {
        return prompt.to_string();
    }
    let part = arguments
        .get("parts")
        .and_then(serde_json::Value::as_array)
        .and_then(|parts| parts.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let selector = part
        .get("selector")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let args = part
        .get("arguments")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if args.is_empty() {
        format!("/{selector}")
    } else {
        format!("/{selector} {args}")
    }
}

/// Short follow-up stream for `muse_turn`: prompt echo, one streaming
/// agent message, and the turn close-out.
fn emit_turn_stream(stdout: &mut std::io::StdoutLock<'_>, session: &str, prompt: &str) {
    let turn = "fake-turn-2";
    let answer = "Second turn done.";
    let mut n = 0_u64;
    let mut at = || {
        n += 1;
        cursor(session, n)
    };
    notify(
        stdout,
        serde_json::json!({
            "type": "turn-started",
            "sessionId": session,
            "turnId": turn,
            "viewCursor": at(),
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-completed",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-user-2",
                "kind": "userMessage",
                "recordedAt": "2026-11-02T21:01:00Z",
                "revision": 1,
                "status": "completed",
                "text": prompt,
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-started",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-agent-2",
                "kind": "agentMessage",
                "revision": 1,
                "status": "inProgress",
                "text": "",
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-delta",
            "sessionId": session,
            "viewCursor": at(),
            "itemId": "fake-item-agent-2",
            "field": "text",
            "delta": answer,
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "item-completed",
            "sessionId": session,
            "viewCursor": at(),
            "item": {
                "itemId": "fake-item-agent-2",
                "kind": "agentMessage",
                "messagePhase": "finalAnswer",
                "recordedAt": "2026-11-02T21:01:01Z",
                "revision": 2,
                "status": "completed",
                "text": answer,
                "turnId": turn,
            },
        }),
    );
    notify(
        stdout,
        serde_json::json!({
            "type": "turn-completed",
            "sessionId": session,
            "turnId": turn,
            "viewCursor": at(),
            "terminal": "completed",
            "answer": answer,
            "droppedDeltas": 0,
        }),
    );
}

/// Pending rows the fake holds until settled: one approval and one
/// 3-question user-input (options with descriptions, one multiple).
fn pending_rows(settled: &std::collections::HashSet<String>) -> serde_json::Value {
    let mut rows = Vec::new();
    if !settled.contains("req-approval-1") {
        rows.push(serde_json::json!({
            "requestId": "req-approval-1",
            "sessionId": "sess-pending",
            "kind": "approval",
            "receivedAt": 1700000000000_u64,
            "ageSec": 5,
            "payload": {
                "approvalId": "req-approval-1",
                "sessionId": "sess-pending",
                "toolName": "bash",
                "rawArgs": "{\"command\":\"ls -la /tmp/ws\"}",
                "subject": {"kind": "command", "command": "ls -la /tmp/ws", "toolName": "bash"},
                "availableChoices": [
                    {"choiceId": "allow_once", "decision": "approved", "scope": "once", "label": "Allow once", "acceptsFeedback": false},
                    {"choiceId": "abort", "decision": "abort", "scope": "once", "label": "Reject", "acceptsFeedback": true}
                ]
            }
        }));
    }
    if !settled.contains("req-question-1") {
        rows.push(serde_json::json!({
            "requestId": "req-question-1",
            "sessionId": "sess-pending",
            "kind": "userInput",
            "receivedAt": 1700000001000_u64,
            "ageSec": 4,
            "payload": {
                "userInputId": "req-question-1",
                "sessionId": "sess-pending",
                "questions": [
                    {
                        "id": "color", "header": "Color", "question": "Pick a color",
                        "selection": {"mode": "single"},
                        "options": [
                            {"label": "Red", "description": "Warm and bright"},
                            {"label": "Blue", "description": "Cool and calm"},
                            {"label": "None"}
                        ]
                    },
                    {
                        "id": "extras", "header": "Extras", "question": "Pick extras",
                        "selection": {"mode": "multiple"},
                        "options": [
                            {"label": "Cheese", "description": "Extra cheese"},
                            {"label": "Olives", "description": "Green olives"}
                        ]
                    },
                    {
                        "id": "file", "header": "File", "question": "Which file?",
                        "selection": {"mode": "single"},
                        "options": [
                            {"label": "alpha.txt", "description": "First file"},
                            {"label": "beta.txt", "description": "Second file"}
                        ]
                    }
                ]
            }
        }));
    }
    serde_json::Value::Array(rows)
}

fn main() {
    let mode = std::env::var("FAKE_MCP_MODE").unwrap_or_else(|_| String::from("ok"));
    if mode == "exit-early" {
        std::process::exit(3);
    }

    let stdin = std::io::stdin();
    let mut lines = BufReader::new(stdin.lock()).lines();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let first = match lines.next() {
        Some(Ok(line)) => line,
        // EOF or read error before initialize: nothing to answer.
        Some(Err(_)) | None => return,
    };
    let id = serde_json::from_str::<serde_json::Value>(&first)
        .ok()
        .and_then(|message| message.get("id").and_then(serde_json::Value::as_u64))
        .unwrap_or(1);

    if mode == "malformed" {
        write_raw(&mut out, "this is not json");
    } else if mode == "init-error" {
        write_line(
            &mut out,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32603, "message": "fake initialize refused" },
            }),
        );
    } else {
        write_line(
            &mut out,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": { "name": "fake-muse-mcp", "version": "0.0.0" },
                },
            }),
        );
    }

    let mut status_calls = 0_u32;
    let mut settled_pending: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Recorded decide/answer calls (actor + payload), checked by the
    // settling behaviour: a wrong actor never settles the row.
    let mut recorded_decides: Vec<serde_json::Value> = Vec::new();
    let mut recorded_answers: Vec<serde_json::Value> = Vec::new();
    for line in lines {
        let line = match line {
            Ok(line) => line,
            Err(_) => return,
        };
        if line.trim().is_empty() {
            continue;
        }
        let message: serde_json::Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(_) => continue,
        };
        let method = message.get("method").and_then(serde_json::Value::as_str);
        match method {
            Some("notifications/initialized") => {
                if mode == "eof-after-init" {
                    return;
                }
                for event in [
                    serde_json::json!({
                        "type": "item-delta",
                        "sessionId": "sess-run",
                        "turnId": "turn-1",
                        "viewCursor": "7",
                        "itemId": "item-1",
                        "field": "text",
                        "delta": "hello",
                    }),
                    serde_json::json!({
                        "type": "from-the-future",
                        "sessionId": "sess-run",
                    }),
                ] {
                    write_line(
                        &mut out,
                        &serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/muse/event",
                            "params": event,
                        }),
                    );
                }
                write_line(
                    &mut out,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/something/else",
                        "params": {},
                    }),
                );
                write_line(
                    &mut out,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/muse/event",
                        "params": {
                            "type": "pending",
                            "sessionId": "sess-pending",
                            "requestId": "req-1",
                            "kind": "approval",
                            "phase": "requested",
                        },
                    }),
                );
            }
            Some("tools/call") => {
                let call_id = message
                    .get("id")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let tool = message
                    .get("params")
                    .and_then(|params| params.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let arguments = message
                    .get("params")
                    .and_then(|params| params.get("arguments"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if tool == "muse_spawn" {
                    let workspace = arguments
                        .get("workspace")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if workspace.contains("does-not-exist") {
                        tool_result(
                            &mut out,
                            &call_id,
                            &format!("workspace does not exist: {workspace}"),
                            true,
                        );
                        continue;
                    }
                    let worktree = arguments
                        .get("worktree")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if (worktree == "create" || worktree == "existing") && workspace == "/untrusted"
                    {
                        tool_result(
                            &mut out,
                            &call_id,
                            "Repository '/untrusted' is not trusted; worktree create, attach, and list require a trusted repository.",
                            true,
                        );
                        continue;
                    }
                    let prompt = echo_text(&arguments);
                    tool_result(
                        &mut out,
                        &call_id,
                        &serde_json::json!({
                            "session_id": "fake-session-1",
                            "turn_id": "fake-turn-1",
                            "status": "running",
                        })
                        .to_string(),
                        false,
                    );
                    emit_spawn_stream(&mut out, &prompt);
                    continue;
                }
                if tool == "muse_turn" {
                    let session = arguments
                        .get("session_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("fake-session-1");
                    let prompt = echo_text(&arguments);
                    tool_result(
                        &mut out,
                        &call_id,
                        &serde_json::json!({
                            "session_id": session,
                            "turn_id": "fake-turn-2",
                            "status": "running",
                        })
                        .to_string(),
                        false,
                    );
                    emit_turn_stream(&mut out, session, &prompt);
                    continue;
                }
                if tool == "muse_pending" {
                    tool_result(
                        &mut out,
                        &call_id,
                        &pending_rows(&settled_pending).to_string(),
                        false,
                    );
                    continue;
                }
                if tool == "muse_decide" {
                    recorded_decides.push(arguments.clone());
                    let actor = arguments.get("actor").and_then(serde_json::Value::as_str);
                    let request_id = arguments
                        .get("request_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let decision = arguments
                        .get("decision")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if actor != Some("user") {
                        tool_result(&mut out, &call_id, "actor must be user", true);
                        continue;
                    }
                    if request_id != "req-approval-1"
                        || settled_pending.contains(request_id)
                        || (decision != "approve" && decision != "deny")
                    {
                        tool_result(
                            &mut out,
                            &call_id,
                            &format!("no pending approval {request_id}"),
                            true,
                        );
                        continue;
                    }
                    settled_pending.insert(String::from(request_id));
                    tool_result(
                        &mut out,
                        &call_id,
                        &serde_json::json!({"choiceId": "allow_once"}).to_string(),
                        false,
                    );
                    continue;
                }
                if tool == "muse_answer" {
                    recorded_answers.push(arguments.clone());
                    let actor = arguments.get("actor").and_then(serde_json::Value::as_str);
                    let request_id = arguments
                        .get("request_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if actor != Some("user") {
                        tool_result(&mut out, &call_id, "actor must be user", true);
                        continue;
                    }
                    if request_id != "req-question-1" || settled_pending.contains(request_id) {
                        tool_result(
                            &mut out,
                            &call_id,
                            &format!("no pending user-input {request_id}"),
                            true,
                        );
                        continue;
                    }
                    // The answer must carry one entry per question.
                    let answer_len = arguments
                        .get("answer")
                        .and_then(serde_json::Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0);
                    if answer_len != 3 {
                        tool_result(
                            &mut out,
                            &call_id,
                            "answer must carry one entry per question",
                            true,
                        );
                        continue;
                    }
                    settled_pending.insert(String::from(request_id));
                    let _ = (&recorded_decides, &recorded_answers);
                    tool_result(
                        &mut out,
                        &call_id,
                        &serde_json::json!({"outcome": "answered"}).to_string(),
                        false,
                    );
                    continue;
                }
                if tool == "muse_skills" {
                    tool_result(
                        &mut out,
                        &call_id,
                        &serde_json::json!({"skills": [
                            {
                                "selector": "review",
                                "display_name": "Review",
                                "description": "Reviews code",
                                "source": "user",
                                "plugin_id": "pr-tools",
                                "argument_hint": "<pr>",
                            },
                            {
                                "selector": "summarize",
                                "display_name": "Summarize",
                                "description": "Summarizes the thread",
                                "source": "bundled",
                            },
                        ]})
                        .to_string(),
                        false,
                    );
                    continue;
                }
                if tool == "muse_models" {
                    if mode == "resume-in-use" {
                        tool_result(
                            &mut out,
                            &call_id,
                            "session sess-orphan is in use by another client (sessionInUse: already attached). Muse MCP will not attach to it.",
                            true,
                        );
                        continue;
                    }
                    tool_result(
                        &mut out,
                        &call_id,
                        &serde_json::json!({"models": ["fake-model-1"]}).to_string(),
                        false,
                    );
                    continue;
                }
                status_calls += 1;
                if status_calls == 1 {
                    let rows = serde_json::to_string(&serde_json::json!([
                        {
                            "session_id": "sess-run",
                            "status": "running",
                            "workspace": "/home/dev/checkout-api",
                            "pending_approvals": 0,
                            "pending_user_inputs": 0,
                            "stream_dropped_deltas": 3,
                        },
                        {
                            "session_id": "sess-idle",
                            "status": "idle",
                            "workspace": "/home/dev/checkout-api",
                        },
                        {
                            "session_id": "sess-orphan",
                            "status": "orphaned",
                            "workspace": "/home/dev/storefront-web",
                        },
                        {
                            "session_id": "sess-pending",
                            "status": "running",
                            "workspace": "/home/dev/storefront-web",
                            "pending_approvals": 2,
                            "pending_user_inputs": 0,
                        },
                        {
                            "session_id": "sess-closed",
                            "status": "closed",
                            "workspace": "/home/dev/storefront-web",
                        },
                        {
                            "session_id": "sess-weird",
                            "status": "exploded",
                            "workspace": "/home/dev/storefront-web",
                        },
                    ]))
                    .expect("rows are JSON");
                    write_line(
                        &mut out,
                        &serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": call_id,
                            "result": {
                                "content": [{"type": "text", "text": rows}],
                                "isError": false,
                            },
                        }),
                    );
                } else {
                    write_line(
                        &mut out,
                        &serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": call_id,
                            "result": {
                                "content": [{"type": "text", "text": "status blew up"}],
                                "isError": true,
                            },
                        }),
                    );
                }
            }
            _ => {}
        }
    }
}
