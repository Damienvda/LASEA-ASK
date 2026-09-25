//! Minimal MCP client over the "Streamable HTTP" transport: JSON-RPC 2.0 requests posted to a
//! single URL, whose response is either a plain `application/json` body or a `text/event-stream`
//! carrying the same JSON-RPC message as its one `data:` event. Good enough for the request/response
//! pattern we need here (initialize, tools/list, tools/call) — this does not handle server-initiated
//! notifications or long-lived SSE subscriptions, which our use case doesn't need.

use crate::providers::sse::sse_events;
use futures::StreamExt;
use serde_json::{json, Value};
use std::sync::Mutex;

pub struct McpClient {
    http: reqwest::Client,
    url: String,
    bearer_token: Option<String>,
    session_id: Mutex<Option<String>>,
    next_id: Mutex<u64>,
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpClient {
    pub fn new(url: String, bearer_token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            url,
            bearer_token,
            session_id: Mutex::new(None),
            next_id: Mutex::new(1),
        }
    }

    /// Performs the MCP handshake: `initialize` followed by the required `notifications/initialized`.
    pub async fn initialize(&self) -> anyhow::Result<()> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "laseask", "version": "0.1.0" }
            }),
        )
        .await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                Some(McpTool {
                    name: t.get("name")?.as_str()?.to_string(),
                    description: t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                })
            })
            .collect())
    }

    /// Calls a tool and flattens its result content into a single text blob (joining any `text`
    /// content blocks), which is what we feed back to the model as the tool_result.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<String> {
        let result = self
            .request("tools/call", json!({ "name": name, "arguments": arguments }))
            .await?;

        if result.get("isError").and_then(|e| e.as_bool()).unwrap_or(false) {
            let text = extract_text(&result);
            anyhow::bail!("tool '{name}' returned an error: {text}");
        }

        Ok(extract_text(&result))
    }

    async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            let id = *n;
            *n += 1;
            id
        };
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let resp = self.send(&body).await?;

        // A request (unlike a notification) must get a JSON-RPC response back; an empty one means
        // the reply couldn't be parsed, which should fail loudly rather than look like success.
        if resp.is_null() {
            anyhow::bail!("MCP server sent no parseable response to '{method}'");
        }

        if let Some(err) = resp.get("error") {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown MCP error");
            anyhow::bail!("MCP error calling '{method}': {message}");
        }

        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Fire-and-forget JSON-RPC notification (no `id`, no response body expected).
    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let _ = self.send(&body).await;
        Ok(())
    }

    async fn send(&self, body: &Value) -> anyhow::Result<Value> {
        let mut req = self
            .http
            .post(&self.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");

        if let Some(token) = &self.bearer_token {
            req = req.bearer_auth(token);
        }
        if let Some(sid) = self.session_id.lock().unwrap().clone() {
            req = req.header("Mcp-Session-Id", sid);
        }

        let resp = req.json(body).send().await?;

        if let Some(sid) = resp.headers().get("mcp-session-id") {
            if let Ok(sid) = sid.to_str() {
                *self.session_id.lock().unwrap() = Some(sid.to_string());
            }
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("MCP server returned {status}: {text}");
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // The server may send notifications (logs, progress) before the actual response, so
            // skip ahead to the first message carrying a JSON-RPC result or error.
            let mut events = sse_events(resp.bytes_stream());
            while let Some(data) = events.next().await {
                let Ok(msg) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                if msg.get("result").is_some() || msg.get("error").is_some() {
                    return Ok(msg);
                }
            }
            Ok(Value::Null)
        } else {
            let text = resp.text().await?;
            if text.trim().is_empty() {
                Ok(Value::Null)
            } else {
                Ok(serde_json::from_str(&text)?)
            }
        }
    }
}

fn extract_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
