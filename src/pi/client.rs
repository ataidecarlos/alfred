//! JSONL transport for `pi --mode rpc`.
//!
//! Commands are JSON objects written to stdin, one per line. Responses and
//! streamed events arrive on stdout, also one JSON object per line. A record
//! that fails to parse is logged and skipped; it is never fatal.
//!
//! [`JsonlReader`] implements the framing rules from Pi's RPC contract:
//! split on LF only, strip a trailing CR, and treat U+2028/U+2029 as ordinary
//! characters inside JSON strings.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tracing::warn;

use crate::error::AlfredError;

/// Reads LF-delimited records from an async byte stream.
#[derive(Debug)]
pub struct JsonlReader<R> {
    inner: R,
    buffer: Vec<u8>,
    eof: bool,
}

impl<R: AsyncRead + Unpin> JsonlReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            eof: false,
        }
    }

    /// Read the next raw record without its terminator, or `None` at end of
    /// stream. A final record without a trailing LF is returned as-is.
    pub async fn next_line(&mut self) -> Result<Option<String>, AlfredError> {
        loop {
            if let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=position).collect();
                line.pop();
                strip_carriage_return(&mut line);
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            if self.eof {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                let mut line = std::mem::take(&mut self.buffer);
                strip_carriage_return(&mut line);
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            let mut chunk = [0_u8; 8192];
            let read = self.inner.read(&mut chunk).await?;
            if read == 0 {
                self.eof = true;
            } else {
                self.buffer.extend_from_slice(&chunk[..read]);
            }
        }
    }

    /// Read the next record parsed as JSON. Unparseable records are logged at
    /// warn level and skipped; `None` means the stream ended.
    pub async fn next_message(&mut self) -> Result<Option<Value>, AlfredError> {
        while let Some(line) = self.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(message) => return Ok(Some(message)),
                Err(error) => {
                    warn!(error = %error, line = %line, "skipping unparseable Pi RPC line");
                }
            }
        }
        Ok(None)
    }
}

fn strip_carriage_return(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

/// A parsed `{"type":"response"}` message.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PiResponse {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A running `pi --mode rpc` subprocess.
///
/// Requests are correlated by `id`; commands without one get a generated id.
/// Messages that are not the awaited response are buffered and replayed in
/// order by [`PiClient::next_message`].
#[derive(Debug)]
pub struct PiClient {
    binary: String,
    child: Child,
    stdin: ChildStdin,
    stdout: JsonlReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
    stderr_task: Option<JoinHandle<()>>,
    next_id: u64,
    pending: VecDeque<Value>,
}

impl PiClient {
    /// Spawn a Pi subprocess with piped stdio. `binary` is used to name the
    /// process in errors; a failed spawn returns [`AlfredError::PiSpawn`]
    /// naming that path.
    pub async fn spawn(binary: &str, mut command: Command) -> Result<Self, AlfredError> {
        command.kill_on_drop(true);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| AlfredError::PiSpawn {
            binary: binary.to_string(),
            message: error.to_string(),
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AlfredError::Pi(format!("Pi process '{binary}' has no stdin pipe")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AlfredError::Pi(format!("Pi process '{binary}' has no stdout pipe")))?;
        let mut stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| AlfredError::Pi(format!("Pi process '{binary}' has no stderr pipe")))?;

        let stderr = Arc::new(Mutex::new(String::new()));
        let stderr_sink = Arc::clone(&stderr);
        let stderr_task = tokio::spawn(async move {
            let mut captured = Vec::new();
            let _ = stderr_pipe.read_to_end(&mut captured).await;
            let text = String::from_utf8_lossy(&captured);
            let mut sink = stderr_sink
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            sink.push_str(&text);
        });

        Ok(Self {
            binary: binary.to_string(),
            child,
            stdin,
            stdout: JsonlReader::new(stdout),
            stderr,
            stderr_task: Some(stderr_task),
            next_id: 0,
            pending: VecDeque::new(),
        })
    }

    /// The binary label this client was spawned with.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// The operating-system process id of the child, or `None` once it has been
    /// reaped. Used by shutdown tests to prove no child is orphaned.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// If the child has already exited, return its status without blocking.
    ///
    /// `None` means the child is still running (or its status could not be
    /// read). Mirrors [`std::process::Child::try_wait`].
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// Kill and reap the child if it is still running.
    ///
    /// A call on an already-exited child succeeds and returns its status. This
    /// is the explicit-reap counterpart to the client's `kill_on_drop`: the drop
    /// path guarantees the process does not outlive the client, while this lets
    /// a caller observe the exit before reporting it.
    pub async fn kill(&mut self) -> Option<std::process::ExitStatus> {
        let _ = self.child.start_kill();
        match self.child.wait().await {
            Ok(status) => Some(status),
            Err(_) => None,
        }
    }

    /// Stderr captured from the subprocess so far.
    pub fn captured_stderr(&self) -> String {
        self.stderr
            .lock()
            .map(|captured| captured.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// Write a command without waiting for its response. An `id` is generated
    /// when the command does not carry one.
    pub async fn send(&mut self, command: &mut Value) -> Result<(), AlfredError> {
        self.write(command).await.map(|_| ())
    }

    /// Send a command and wait for its response. Messages received in the
    /// meantime (events, UI requests) are buffered and replayed by
    /// [`PiClient::next_message`].
    pub async fn request(&mut self, mut command: Value) -> Result<PiResponse, AlfredError> {
        let id = self.write(&mut command).await?;
        let mut deferred = Vec::new();
        loop {
            let message = match self.pending.pop_front() {
                Some(message) => message,
                None => match self.stdout.next_message().await? {
                    Some(message) => message,
                    None => return Err(self.process_exited_error().await),
                },
            };
            if is_response_for(&message, &id) {
                self.pending.extend(deferred);
                return serde_json::from_value(message)
                    .map_err(|error| AlfredError::Pi(format!("malformed response: {error}")));
            }
            deferred.push(message);
        }
    }

    /// Read the next streamed message. Ends the wait with
    /// [`AlfredError::PiProcessExited`] when the stream closes mid-run.
    pub async fn next_message(&mut self) -> Result<Value, AlfredError> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(message);
        }
        match self.stdout.next_message().await? {
            Some(message) => Ok(message),
            None => Err(self.process_exited_error().await),
        }
    }

    async fn write(&mut self, command: &mut Value) -> Result<Value, AlfredError> {
        let object = command
            .as_object_mut()
            .ok_or_else(|| AlfredError::Pi("command must be a JSON object".into()))?;
        let id = match object.get("id") {
            Some(id) => id.clone(),
            None => {
                self.next_id += 1;
                let id = Value::String(format!("alfred-{}", self.next_id));
                object.insert("id".to_string(), id.clone());
                id
            }
        };
        let mut line = serde_json::to_string(&*command)
            .map_err(|error| AlfredError::Pi(format!("failed to encode command: {error}")))?;
        line.push('\n');
        if let Err(error) = self.stdin.write_all(line.as_bytes()).await {
            return Err(self.process_error("write command", error).await);
        }
        if let Err(error) = self.stdin.flush().await {
            return Err(self.process_error("flush command", error).await);
        }
        Ok(id)
    }

    /// Turn an I/O failure into a process-exited error when the child is gone,
    /// naming the binary and any stderr it produced.
    async fn process_error(&mut self, operation: &str, error: std::io::Error) -> AlfredError {
        match self.child.try_wait() {
            Ok(Some(status)) => self.exited_error(status.to_string()).await,
            _ => AlfredError::Pi(format!(
                "failed to {operation} for Pi process '{}': {error}",
                self.binary
            )),
        }
    }

    /// The stdout stream ended while the process was still expected to talk.
    async fn process_exited_error(&mut self) -> AlfredError {
        let _ = self.child.start_kill();
        let status = match self.child.wait().await {
            Ok(status) => status.to_string(),
            Err(error) => format!("unavailable: {error}"),
        };
        self.exited_error(status).await
    }

    async fn exited_error(&mut self, status: String) -> AlfredError {
        if let Some(task) = self.stderr_task.take() {
            let _ = task.await;
        }
        AlfredError::PiProcessExited {
            binary: self.binary.clone(),
            status,
            stderr: self.captured_stderr(),
        }
    }
}

fn is_response_for(message: &Value, id: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some("response") && message.get("id") == Some(id)
}
