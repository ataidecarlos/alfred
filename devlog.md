# Alfred Development Log — Lessons Learned

## Issues, Bugs & Difficulties Encountered

---

### 1. axum Route Conflicts

**Problem:** Routes like `/api/todos/{user_id}` and `/api/todos/{id}` conflicted in axum because it treats path parameters with different names but the same position as the same route pattern.

**Error:**
```
Invalid route "/api/todos/{id}": Insertion failed due to conflict with previously registered route: /api/todos/{user_id}
```

**Fix:** Use distinct route patterns:
- `GET /api/todos` (no path parameter)
- `POST /api/todos` (create)
- `DELETE /api/todos/{id}` (delete by id)

**Lesson:** Always check axum route uniqueness rules. Different parameter names in the same position = same route.

---

### 2. OpenAI-Compatible API Field Names

**Problem:** The streaming response uses `delta.content`, not `delta.text`. Our initial OpenAI provider looked for `delta.text`, which caused empty responses even though the API returned 200 OK.

**Evidence:**
```
Response status: 200 OK
Assistant message with no text content: AssistantMessage { content: [], usage: Usage { input: 0, output: 0, total: 0 }, stop_reason: Stop }
```

**Fix in `src/llm/openai.rs`:**
```rust
// BEFORE (broken):
if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {

// AFTER (fixed):
if let Some(text) = delta.get("content").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
```

**Lesson:** Always verify the actual API response field names. "OpenAI-compatible" doesn't mean identical.

---

### 3. DeepSeek Thinking Mode

**Problem:** DeepSeek V4 enables thinking/reasoning by default, which can cause unexpected behavior and unnecessary token usage.

**Fix:** Send `"thinking": {"type": "disabled"}` in the request body when the base URL contains "deepseek":
```rust
if self.base_url.contains("deepseek") {
    body["thinking"] = serde_json::json!({"type": "disabled"});
}
```

**Lesson:** Check provider-specific default behaviors. DeepSeek's thinking mode is on by default; OpenAI's is off.

---

### 4. Tool Call Argument Accumulation Bug

**Problem:** The agent loop was trying to parse partial JSON on every streaming delta, which always failed because the JSON was incomplete mid-stream.

**Evidence:**
```
Executing tool call: todo with args: {}
Tool result: invalid arguments: missing field `action`
```

**Root Cause:** The delta handler tried to parse `{"action"` (incomplete) as JSON, which failed, keeping the args as `{}`.

**Fix:** Use a parallel `Vec<String>` to accumulate raw JSON strings, then parse only after the stream completes:
```rust
let mut tool_call_args: Vec<String> = Vec::new();

// In ToolCallDelta handler:
if !args_delta.is_empty() {
    tool_call_args[index].push_str(&args_delta);
}

// In Done handler:
for (i, tc) in tool_calls_out.iter_mut().enumerate() {
    if i < tool_call_args.len() && !tool_call_args[i].is_empty() {
        if let Ok(v) = serde_json::from_str(&tool_call_args[i]) {
            tc.arguments = v;
        }
    }
}
```

**Lesson:** Never parse partial JSON. Accumulate as raw strings, parse at the end.

---

### 5. rusqlite Connection is not Sync

**Problem:** `rusqlite::Connection` uses internal `RefCell` which is not `Sync`, causing "future cannot be sent between threads safely" errors when used in async contexts.

**Error:**
```
error: future cannot be sent between threads safely
  --> src/tools/todo.rs:48:5
   |
48 |     async fn execute(&self, args: serde_json::Value) -> ToolOutput {
   |     ^^^^^ future created by async block is not `Send`
   |
   = help: within `Store`, the trait `Sync` is not implemented for `RefCell<rusqlite::inner_connection::InnerConnection>`
```

**Fix:** Wrap the connection in `std::sync::Mutex<Connection>`:
```rust
pub struct Store {
    conn: Mutex<Connection>,
}

// Access:
let conn = self.conn.lock().map_err(|e| ...)?;
```

**Lesson:** `Mutex` is needed for non-Sync types shared across threads. Use `std::sync::Mutex` (not `tokio::sync::Mutex`) for database connections.

---

### 6. SQL Parameter Count Mismatch

**Problem:** The `add_todo` function had 7 placeholders (`?1` through `?7`) but only 6 values in the `params![]` array. The `id` field was missing.

**Error:**
```
Wrong number of parameters passed to query. Got 6, needed 7
```

**Fix:**
```rust
// BEFORE (broken):
params![title, description, priority, ..., now, now],

// AFTER (fixed):
params![id, title, description, priority, ..., now, now],
```

**Lesson:** Always count placeholders and verify they match the params array length.

---

### 7. AppState Must Be Clone

**Problem:** axum's `with_state()` requires the state type to implement `Clone`. Our `AppState` didn't derive `Clone`.

**Fix:** Add `#[derive(Clone)]` to `AppState` and ensure all fields are `Clone`-able. Use `Arc<T>` for shared state:
```rust
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<crate::store::Store>,
    pub tools: Arc<ToolRegistry>,
    pub provider: Arc<dyn LlmProvider>,
    // ...
}
```

**Lesson:** axum state must be Clone. Use Arc for shared references.

---

### 8. TUI Lifetime Issues

**Problem:** `render_message()` returned `Vec<Line<'static>>` but tried to use borrowed `&str` from the message, which has a different lifetime.

**Error:**
```
error: lifetime may not live long enough
   --> src/tui/mod.rs:200:13
    |
175 |   fn render_message(msg: &ChatMessage) -> Vec<Line<'static>> {
    |                          - let's call the lifetime of this reference `'1`
...
200 | /             vec![
201 | |                 Line::from(Span::styled(text.as_str(), Style::default().fg(Color::Yellow))),
202 | |                 Line::from(""),
203 | |             ]
    | |_____________^ returning this value requires that `'1` must outlive `'static`
```

**Fix:** Clone strings into owned `String` values before creating `Span` and `Line` objects:
```rust
Span::styled(text.clone(), Style::default().fg(Color::Yellow))
```

**Lesson:** When returning `'_static` types, ensure all data is owned, not borrowed.

---

### 9. PowerShell JSON Escaping

**Problem:** PowerShell mangles JSON when passed directly to `curl.exe` via `-d`. The quotes get converted to Unicode curly quotes.

**Evidence:**
```
Failed to parse the request body as JSON: key must be a string at line 1 column 2
```

**Fix:** Write JSON to a temp file and use `curl --data-binary @file.json`:
```powershell
[System.IO.File]::WriteAllText("C:\temp\body.json", '{"key":"value"}')
curl.exe -s -X POST http://localhost:8080/api/messages -H "Content-Type: application/json" --data-binary @C:\temp\body.json
```

**Lesson:** PowerShell and curl don't play well together with JSON. Use temp files.

---

### 10. Connection Tracking Middleware

**Problem:** The connection tracking middleware closure needed explicit type annotations, and the `active_connections` variable was cloned but not used properly.

**Error:**
```
error[E0282]: type annotations needed
  --> src/server/mod.rs:60:47
   |
60 |         .layer(middleware::from_fn(move |req, next| {
   |                                               ^^^^
   |
help: consider giving this closure parameter an explicit type
   |
60 |         .layer(middleware::from_fn(move |req, next: /* Type */| {
```

**Fix:** Use a named function instead of an inline closure:
```rust
async fn connection_tracker(req: Request, next: Next) -> Response {
    // ...
}
.layer(middleware::from_fn(connection_tracker))
```

**Lesson:** Named functions avoid type inference issues with closures.

---

### 11. Provider Registration

**Problem:** The `create_provider()` function didn't handle "deepseek" as a provider name, even though DeepSeek uses an OpenAI-compatible API.

**Error:**
```
unknown provider: deepseek
```

**Fix:** Add "deepseek" to the match statement, using `OpenAiProvider` with the DeepSeek base URL:
```rust
"deepseek" => Ok(Arc::new(openai::OpenAiProvider::new(api_key, config.base_url.as_deref().unwrap_or("https://api.deepseek.com")))),
```

**Lesson:** Adding a new provider requires updating `create_provider()` even if it uses an existing implementation.

---

### 12. Server Already Running Detection

**Problem:** When trying to start a second server instance, it would fail with a port binding error instead of detecting the existing server.

**Fix:** Try to bind to the port first. If it fails, query the existing server's `/api/info` endpoint:
```rust
if tui::check_server(&server_url).await {
    println!("Alfred server is already running.");
    if let Some(info) = tui::get_server_info(&server_url).await {
        println!("  PID: {}", info.pid);
        // ...
    }
    return;
}
```

**Lesson:** Graceful handling of "already running" scenarios improves UX.

---

## Key Takeaways

1. **Always verify API field names** — "OpenAI-compatible" doesn't mean identical
2. **Count SQL parameters** — placeholder count must match params array length
3. **Wrap non-Sync types in Mutex** — rusqlite Connection needs `Mutex` for async contexts
4. **Accumulate streaming data as strings** — parse JSON only after the stream completes
5. **Derive Clone for axum state** — all state types must be Clone-able
6. **Use temp files for JSON in PowerShell** — avoids escaping issues
7. **Named functions over closures** — avoids type inference issues with middleware
8. **Check provider-specific defaults** — DeepSeek's thinking mode is on by default
