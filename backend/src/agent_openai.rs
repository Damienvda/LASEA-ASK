//! The tool-use loop for OpenAI-compatible chat-completions APIs (OpenAI and Mistral). Same idea as
//! agent.rs: stream a turn, and whenever the model asks for tool calls, run them against the right
//! MCP server, feed the results back as `role: "tool"` messages, and continue — until the model
//! answers without requesting a tool, or `MAX_TURNS` is hit as a safety valve.
//!
//! MCP itself is model-agnostic; only the way tools are described to the model and the way the
//! model asks to call them differs per provider, which is all this file handles.

use crate::agent::{
    call_mcp_tool, request_plan, should_check, Reply, ToolSpec, COMPLETION_CHECK, PLAN_GO,
};
use crate::config::{AgentConfig, ProviderConfig};
use crate::mcp::McpClient;
use crate::providers::openai::{MISTRAL_API_URL, OPENAI_API_URL};
use crate::providers::sse::sse_events;
use crate::providers::{ChatMessage, StreamEvent};
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

/// Where to send requests for one OpenAI-compatible provider, and its provider-specific quirks.
pub struct Endpoint {
    pub label: &'static str,
    pub api_url: &'static str,
    /// The `tool_choice` value that forces the model to call some tool: OpenAI calls it
    /// "required", Mistral calls it "any".
    pub forced_tool_choice: &'static str,
}

pub fn endpoint_for(provider: &str) -> Option<Endpoint> {
    match provider {
        "openai" => Some(Endpoint {
            label: "openai",
            api_url: OPENAI_API_URL,
            forced_tool_choice: "required",
        }),
        "mistral" => Some(Endpoint {
            label: "mistral",
            api_url: MISTRAL_API_URL,
            forced_tool_choice: "any",
        }),
        _ => None,
    }
}

#[derive(Default)]
struct ToolCallAcc {
    id: String,
    name: String,
    arguments: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    endpoint: Endpoint,
    provider: &ProviderConfig,
    model: &str,
    initial_messages: &[ChatMessage],
    tools: &[ToolSpec],
    mcp_clients: &HashMap<String, Arc<McpClient>>,
    force_first_tool_use: bool,
    cfg: &AgentConfig,
    tx: Sender<StreamEvent>,
) {
    if let Err(e) = run_inner(
        &endpoint,
        provider,
        model,
        initial_messages,
        tools,
        mcp_clients,
        force_first_tool_use,
        cfg,
        &tx,
    )
    .await
    {
        let _ = tx.send(StreamEvent::Error { message: e.to_string() }).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(
    endpoint: &Endpoint,
    provider: &ProviderConfig,
    model: &str,
    initial_messages: &[ChatMessage],
    tools: &[ToolSpec],
    mcp_clients: &HashMap<String, Arc<McpClient>>,
    force_first_tool_use: bool,
    cfg: &AgentConfig,
    tx: &Sender<StreamEvent>,
) -> anyhow::Result<()> {
    let label = endpoint.label;

    // System messages stay inline here — both OpenAI and Mistral accept role "system" in the list.
    let mut messages: Vec<Value> = initial_messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();

    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.qualified_name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect();

    let client = reqwest::Client::new();
    let mut reply = Reply::new(tx);
    let mut tools_used = 0;
    let mut checked = false;
    // Planning turn first (see agent.rs), with tools disabled.
    let planning = cfg.plan_first;
    if planning {
        request_plan(&mut messages);
    }
    let first_tool_turn = usize::from(planning);

    for turn in 0..cfg.max_turns {
        // The browser went away (Stop button, tab closed): don't keep paying for API calls and
        // tool runs nobody will see.
        if tx.is_closed() {
            return Ok(());
        }
        let mut body = json!({
            "model": model,
            "stream": true,
            "messages": messages,
            "tools": tools_json,
        });
        if let Some(t) = provider.temperature {
            body["temperature"] = json!(t);
        }
        if planning && turn == 0 {
            body["tool_choice"] = json!("none");
        }
        // "MCP only" mode: force the first tool turn to call a tool (see agent.rs for why only
        // that one).
        if force_first_tool_use && turn == first_tool_turn {
            body["tool_choice"] = json!(endpoint.forced_tool_choice);
        }
        // Last turn: forbid tools so the model has to answer with what it has gathered (see
        // agent.rs). Both OpenAI and Mistral spell this "none".
        if turn + 1 == cfg.max_turns {
            body["tool_choice"] = json!("none");
        }

        let resp = client
            .post(endpoint.api_url)
            .bearer_auth(&provider.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("{label} error {status}: {text}");
        }

        let mut events = sse_events(resp.bytes_stream());

        let mut text = String::new();
        // Keyed by the tool call's `index`; OpenAI streams a call's arguments across many chunks,
        // Mistral usually sends each call whole in one chunk. Accumulating handles both.
        let mut calls: BTreeMap<u64, ToolCallAcc> = BTreeMap::new();
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut finish_reason = String::new();
        reply.start_turn();

        while let Some(data) = events.next().await {
            let v: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(message) = stream_error(&v) {
                anyhow::bail!("{label} error: {message}");
            }

            // Mistral sends usage in the last chunk; OpenAI only if asked, so it may stay 0.
            if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
                input_tokens = usage.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0);
                output_tokens = usage.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0);
            }
            if let Some(reason) = v.pointer("/choices/0/finish_reason").and_then(Value::as_str) {
                finish_reason = reason.to_string();
            }

            let Some(delta) = v.pointer("/choices/0/delta") else {
                continue;
            };

            let chunk = content_text(delta.get("content"));
            if !chunk.is_empty() {
                text.push_str(&chunk);
                reply.text(&chunk).await;
            }

            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for (pos, tc) in tool_calls.iter().enumerate() {
                    let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let mut key = tc
                        .get("index")
                        .and_then(|i| i.as_u64())
                        .unwrap_or(pos as u64);
                    // A different id at an already-used index is a new call, not a continuation.
                    if calls.get(&key).is_some_and(|c| !c.id.is_empty() && !id.is_empty() && c.id != id) {
                        key = calls.keys().next_back().map_or(0, |k| k + 1);
                    }
                    let acc = calls.entry(key).or_default();
                    if !id.is_empty() {
                        acc.id = id.to_string();
                    }
                    if let Some(name) = tc.pointer("/function/name").and_then(|n| n.as_str()) {
                        acc.name.push_str(name);
                    }
                    match tc.pointer("/function/arguments") {
                        Some(Value::String(s)) => acc.arguments.push_str(s),
                        Some(other) if !other.is_null() => acc.arguments.push_str(&other.to_string()),
                        _ => {}
                    }
                }
            }
        }

        tracing::info!(
            "{label} turn {}: {input_tokens} input tokens, {output_tokens} output tokens, finish: {finish_reason}",
            turn + 1
        );

        if planning && turn == 0 {
            messages.push(json!({ "role": "assistant", "content": text }));
            messages.push(json!({ "role": "user", "content": PLAN_GO }));
            continue;
        }

        if calls.is_empty() {
            if reply.checking() {
                reply.end_check().await;
            } else {
                if finish_reason == "length" {
                    reply
                        .text("\n\n*[Answer cut off: the model's output limit was reached.]*")
                        .await;
                }
                if should_check(cfg, checked, tools_used, turn) {
                    checked = true;
                    reply.start_check();
                    messages.push(json!({ "role": "assistant", "content": text }));
                    messages.push(json!({ "role": "user", "content": COMPLETION_CHECK }));
                    continue;
                }
            }
            let _ = tx.send(StreamEvent::Done).await;
            return Ok(());
        }

        if reply.checking() {
            reply.end_check().await;
        }
        tools_used += calls.len();

        let tool_calls_json: Vec<Value> = calls
            .values()
            .map(|c| {
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": c.arguments },
                })
            })
            .collect();
        messages.push(json!({
            "role": "assistant",
            "content": text,
            "tool_calls": tool_calls_json,
        }));

        for call in calls.into_values() {
            let _ = tx.send(StreamEvent::ToolCall { name: call.name.clone() }).await;

            let input: Value = if call.arguments.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}))
            };
            let result_text = call_mcp_tool(&call.name, input, mcp_clients, cfg).await;

            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": result_text,
            }));
        }
    }

    anyhow::bail!("tool-use loop exceeded {} turns without a final answer", cfg.max_turns)
}

/// Error payloads mid-stream: OpenAI sends `{"error": {"message": ...}}`, Mistral may send
/// `{"object": "error", "message": ...}`.
fn stream_error(v: &Value) -> Option<String> {
    if let Some(err) = v.get("error") {
        return Some(
            err.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string(),
        );
    }
    if v.get("object").and_then(|o| o.as_str()) == Some("error") {
        return Some(
            v.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string(),
        );
    }
    None
}

/// `delta.content` is normally a string, but some Mistral models (e.g. Magistral) stream it as an
/// array of typed chunks; keep only the plain `text` ones.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect(),
        _ => String::new(),
    }
}
