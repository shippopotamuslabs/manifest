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
