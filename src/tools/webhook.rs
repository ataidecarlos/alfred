use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::tool::{Tool, ToolOutput};
use crate::llm::ToolDefinition;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WebhookArgs {
    /// The URL to send the request to
    pub url: String,
    /// HTTP method: GET, POST, PUT, DELETE (default: POST)
    #[serde(default = "default_method")]
    pub method: String,
    /// Optional headers as key-value pairs
    pub headers: Option<HashMap<String, String>>,
    /// Optional request body (JSON string)
    pub body: Option<String>,
}

fn default_method() -> String { "POST".into() }

pub struct WebhookTool {
    client: Client,
}

impl WebhookTool {
    pub fn new() -> Self {
        Self { client: Client::builder().timeout(std::time::Duration::from_secs(30)).build().unwrap() }
    }
}

#[async_trait]
impl Tool for WebhookTool {
    fn definition(&self) -> ToolDefinition {
        let schema = schemars::schema_for!(WebhookArgs);
        ToolDefinition {
            name: "webhook".into(),
            description: "Send an HTTP request to a URL. Use this to call webhooks, APIs, or any HTTP endpoint.".into(),
            parameters: schema,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let args: WebhookArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutput::error(format!("invalid arguments: {}", e)),
        };

        let method = args.method.to_uppercase();
        let mut req = match method.as_str() {
            "GET" => self.client.get(&args.url),
            "POST" => self.client.post(&args.url),
            "PUT" => self.client.put(&args.url),
            "DELETE" => self.client.delete(&args.url),
            "PATCH" => self.client.patch(&args.url),
            _ => return ToolOutput::error(format!("unsupported method: {}", method)),
        };

        if let Some(headers) = &args.headers {
            for (k, v) in headers {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        if let Some(body) = &args.body {
            req = req.body(body.clone()).header("Content-Type", "application/json");
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let truncated = safe_truncate(&body, 2000);
                ToolOutput::success(format!("Status: {}\nBody: {}", status, truncated))
            }
            Err(e) => ToolOutput::error(format!("request failed: {}", e)),
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
