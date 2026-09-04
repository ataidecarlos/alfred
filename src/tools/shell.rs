use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::agent::tool::{Tool, ToolOutput};
use crate::llm::ToolDefinition;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShellArgs {
    /// The shell command to execute
    pub command: String,
    /// Timeout in seconds (default: 60)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 { 60 }

pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for ShellTool {
    fn definition(&self) -> ToolDefinition {
        let schema = schemars::schema_for!(ShellArgs);
        ToolDefinition {
            name: "shell".into(),
            description: "Execute a shell command. Returns stdout and stderr output.".into(),
            parameters: schema,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let args: ShellArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutput::error(format!("invalid arguments: {}", e)),
        };

        let timeout = std::time::Duration::from_secs(args.timeout_secs);

        #[cfg(target_os = "windows")]
        let cmd_result = {
            tokio::time::timeout(timeout, async {
                Command::new("cmd")
                    .args(["/C", &args.command])
                    .output()
                    .await
            }).await
        };

        #[cfg(not(target_os = "windows"))]
        let cmd_result = {
            tokio::time::timeout(timeout, async {
                Command::new("sh")
                    .args(["-c", &args.command])
                    .output()
                    .await
            }).await
        };

        match cmd_result {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut result = String::new();
                if !stdout.is_empty() {
                    result.push_str(&format!("stdout:\n{}", safe_truncate(&stdout, 2000)));
                }
                if !stderr.is_empty() {
                    if !result.is_empty() { result.push_str("\n\n"); }
                    result.push_str(&format!("stderr:\n{}", safe_truncate(&stderr, 2000)));
                }
                if result.is_empty() {
                    result = "(no output)".into();
                }
                if !out.status.success() {
                    result.push_str(&format!("\n\nexit code: {}", out.status.code().unwrap_or(-1)));
                }
                ToolOutput { output: result, is_error: !out.status.success() }
            }
            Ok(Err(e)) => ToolOutput::error(format!("failed to execute: {}", e)),
            Err(_) => ToolOutput::error(format!("command timed out after {} seconds", args.timeout_secs)),
        }
    }
}

fn safe_truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
