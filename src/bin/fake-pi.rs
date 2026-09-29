//! `fake-pi` — a deterministic stand-in for `pi --mode rpc`.
//!
//! Alfred's Pi transport tests point `[pi] binary` at this executable so the
//! whole RPC boundary can be exercised without a network call, an API key, or a
//! real model. It is a compiled artifact on purpose: on Windows an npm-installed
//! `pi` resolves to a `.ps1` shim, which Rust's `CreateProcess` cannot execute,
//! so the test double has to be a real executable just like the real client's
//! target. The same binary is produced on Linux and macOS.
//!
//! It ignores its command line and speaks the protocol on stdin/stdout: one
//! JSON command per line in, one JSON response or event per line out.
//!
//! Supported commands: get_state, get_last_assistant_text, get_session_stats,
//! prompt, abort, new_session, set_model, set_thinking_level, set_session_name,
//! compact, clear_queue.
//!
//! While streaming a prompt it emits one malformed line on purpose: clients
//! must log and skip unparseable records, never die on them.
//!
//! The protocol is intentionally identical to the former
//! `tests/fixtures/fake-pi.sh`; there is exactly one implementation of the
//! double so the two can never drift.

use std::io::{self, BufRead, Write};

use serde_json::{json, Map, Value};

/// A record to write to stdout: JSON for the protocol, or a raw line used for
/// the deliberately malformed record.
enum Record {
    Json(Value),
    Raw(&'static str),
}

fn main() {
    // Mirror the real CLI's `--version` so `/api/info` can report a version
    // when `[pi].binary` points at this double.
    if std::env::args().any(|arg| arg == "--version") {
        println!("fake-pi {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut last_text: Option<String> = None;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                // A read failure on stdin means there is nothing more to serve.
                eprintln!("fake-pi: failed to read stdin: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                // The shell double answered an unparseable command as an
                // unknown command; the client never sends one, so this is
                // best-effort parity rather than a modelled path.
                let _ = write_record(&mut out, &Record::Json(failed(None, "", "unknown command: ")));
                let _ = out.flush();
                continue;
            }
        };

        let command = value.get("type").and_then(Value::as_str).unwrap_or("").to_string();
        let id = value.get("id").cloned();

        let records = match dispatch(&command, &value, id.as_ref(), &mut last_text) {
            Ok(records) => records,
            Err(message) => vec![Record::Json(failed(id.as_ref(), &command, &message))],
        };

        for record in records {
            let _ = write_record(&mut out, &record);
        }
        // Flush per command so a client blocked on the response is never left
        // waiting on a buffer.
        let _ = out.flush();
    }
}

fn dispatch(
    command: &str,
    value: &Value,
    id: Option<&Value>,
    last_text: &mut Option<String>,
) -> Result<Vec<Record>, String> {
    match command {
        "get_state" => Ok(vec![Record::Json(with_data(response(id, command, true), state_data()))]),
        "get_last_assistant_text" => {
            let text = match last_text {
                Some(text) => Value::String(text.clone()),
                None => Value::Null,
            };
            Ok(vec![Record::Json(with_data(response(id, command, true), json!({ "text": text })))])
        }
        "get_session_stats" => {
            Ok(vec![Record::Json(with_data(response(id, command, true), session_stats_data()))])
        }
        "prompt" => {
            let message = value.get("message").and_then(Value::as_str).unwrap_or("");
            let reply = format!("reply to: {message}");
            *last_text = Some(reply.clone());

            Ok(vec![
                Record::Json(response(id, command, true)),
                Record::Json(json!({ "type": "agent_start" })),
                Record::Raw("this line is malformed on purpose; clients must warn and skip it"),
                Record::Json(json!({ "type": "message_start", "message": { "role": "assistant" } })),
                Record::Json(json!({
                    "type": "message_update",
                    "assistantMessageEvent": { "type": "text_delta", "delta": reply },
                })),
                Record::Json(json!({
                    "type": "message_end",
                    "message": { "role": "assistant", "content": [{ "type": "text", "text": reply }] },
                })),
                Record::Json(json!({ "type": "turn_end" })),
                Record::Json(json!({ "type": "agent_end", "messages": [], "willRetry": false })),
                Record::Json(json!({ "type": "agent_settled" })),
            ])
        }
        "abort" | "new_session" | "set_model" | "set_thinking_level" | "set_session_name"
        | "compact" | "clear_queue" => Ok(vec![Record::Json(response(id, command, true))]),
        other => Err(format!("unknown command: {other}")),
    }
}

fn response(id: Option<&Value>, command: &str, success: bool) -> Value {
    let mut object = Map::new();
    object.insert("type".to_string(), json!("response"));
    if let Some(id) = id {
        object.insert("id".to_string(), id.clone());
    }
    object.insert("command".to_string(), json!(command));
    object.insert("success".to_string(), json!(success));
    Value::Object(object)
}

fn with_data(mut response: Value, data: Value) -> Value {
    if let Some(object) = response.as_object_mut() {
        object.insert("data".to_string(), data);
    }
    response
}

fn failed(id: Option<&Value>, command: &str, message: &str) -> Value {
    let mut value = response(id, command, false);
    if let Some(object) = value.as_object_mut() {
        object.insert("error".to_string(), json!(message));
    }
    value
}

fn state_data() -> Value {
    json!({
        "model": { "id": "fake-model", "name": "Fake Model", "provider": "fixture", "api": "fixture" },
        "thinkingLevel": "off",
        "isStreaming": false,
        "isCompacting": false,
        "steeringMode": "one-at-a-time",
        "followUpMode": "one-at-a-time",
        "sessionId": "fake-session",
        "autoCompactionEnabled": true,
        "messageCount": 0,
        "pendingMessageCount": 0,
    })
}

fn session_stats_data() -> Value {
    json!({
        "sessionId": "fake-session",
        "userMessages": 1,
        "assistantMessages": 1,
        "toolCalls": 0,
        "toolResults": 0,
        "totalMessages": 2,
        "tokens": { "input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0, "total": 15 },
        "cost": 0,
    })
}

fn write_record(out: &mut impl Write, record: &Record) -> io::Result<()> {
    match record {
        Record::Json(value) => writeln!(out, "{value}"),
        Record::Raw(text) => writeln!(out, "{text}"),
    }
}
