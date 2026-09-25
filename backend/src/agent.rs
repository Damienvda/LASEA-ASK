//! The Anthropic tool-use loop: streams a chat turn, and whenever Claude asks to invoke a tool,
//! calls it against the right MCP server, feeds the result back, and continues — repeating until
//! Claude answers without requesting a tool, or `MAX_TURNS` is hit as a safety valve.
//!
//! The wire format here is Anthropic-specific (content-block streaming, `tool_use`/`tool_result`
//! blocks). OpenAI-compatible providers (OpenAI, Mistral) use a different tool-calling format and
//! have their own loop in agent_openai.rs; both share `call_mcp_tool` below. routes.rs picks the
//! right loop whenever at least one MCP tool is configured, and otherwise uses the plain
//! Provider::stream_chat path.

use crate::mcp::McpClient;
use crate::providers::sse::sse_events;
use crate::providers::{ChatMessage, StreamEvent};
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

#[derive(Clone)]
pub struct ToolSpec {
    /// "{server}__{tool}", unique across every configured MCP server, and what we tell Claude the
    /// tool is named.
    pub qualified_name: String,
    pub description: String,
    pub input_schema: Value,
}

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_TURNS: usize = 20;
/// Roughly 10k tokens. See `truncate_tool_result`.
const MAX_TOOL_RESULT_CHARS: usize = 40_000;

enum BlockAcc {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

pub async fn run(
    api_key: &str,
    model: &str,
    initial_messages: &[ChatMessage],
    tools: &[ToolSpec],
    mcp_clients: &HashMap<String, Arc<McpClient>>,
    force_first_tool_use: bool,
    tx: Sender<StreamEvent>,
) {
    if let Err(e) = run_inner(
        api_key,
        model,
        initial_messages,
        tools,
        mcp_clients,
        force_first_tool_use,
        &tx,
    )
    .await
    {
        let _ = tx.send(StreamEvent::Error { message: e.to_string() }).await;
    }
}

async fn run_inner(
    api_key: &str,
    model: &str,
    initial_messages: &[ChatMessage],
    tools: &[ToolSpec],
    mcp_clients: &HashMap<String, Arc<McpClient>>,
    force_first_tool_use: bool,
    tx: &Sender<StreamEvent>,
) -> anyhow::Result<()> {
    let system: String = initial_messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut messages: Vec<Value> = initial_messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();

    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.qualified_name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    let client = reqwest::Client::new();

    for turn in 0..MAX_TURNS {
        // The browser went away (Stop button, tab closed): don't keep paying for API calls and
        // tool runs nobody will see.
        if tx.is_closed() {
            return Ok(());
        }
        let mut body = json!({
            "model": model,
            "max_tokens": 4096,
            "stream": true,
            "messages": messages,
            "tools": tools_json,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        // "MCP only" mode: force the very first turn to invoke a tool rather than let Claude
        // answer from its own knowledge. This is an actual API-level constraint (Anthropic's
        // tool_choice), not a prompt nudge, so it's reliable. Only the first turn — forcing it on
        // every turn would mean the model could never stop calling tools to produce a final
        // text-only answer.
        if force_first_tool_use && turn == 0 {
            body["tool_choice"] = json!({ "type": "any" });
        }
        // Last turn: forbid tools so the model has to answer with what it has gathered, instead
        // of the loop ending on an error after all that work.
        if turn == MAX_TURNS - 1 {
            body["tool_choice"] = json!({ "type": "none" });
        }

        let resp = client
            .post(API_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("anthropic error {status}: {text}");
        }

        let mut events = sse_events(resp.bytes_stream());

        // Per-content-block accumulators, indexed by the block's `index` in this turn.
        let mut blocks: HashMap<u64, BlockAcc> = HashMap::new();
        let mut order: Vec<u64> = Vec::new();

        while let Some(data) = events.next().await {
            let v: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v.get("type").and_then(|t| t.as_str()) {
                Some("content_block_start") => {
                    let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let block = v.get("content_block").cloned().unwrap_or(json!({}));
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                    order.push(index);
                    if block_type == "tool_use" {
                        blocks.insert(
                            index,
                            BlockAcc::ToolUse {
                                id: block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                                name: block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                                input_json: String::new(),
                            },
                        );
                    } else {
                        blocks.insert(index, BlockAcc::Text(String::new()));
                    }
                }
                Some("content_block_delta") => {
                    let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let delta = v.get("delta").cloned().unwrap_or(json!({}));
                    match delta.get("type").and_then(|t| t.as_str()) {
                        Some("text_delta") => {
                            let text = delta.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            if let Some(BlockAcc::Text(buf)) = blocks.get_mut(&index) {
                                buf.push_str(text);
                            }
                            let _ = tx.send(StreamEvent::Delta { text: text.to_string() }).await;
                        }
                        Some("input_json_delta") => {
                            let partial = delta.get("partial_json").and_then(|t| t.as_str()).unwrap_or("");
                            if let Some(BlockAcc::ToolUse { input_json, .. }) = blocks.get_mut(&index) {
                                input_json.push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
                Some("error") => {
                    let message = v
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown anthropic error")
                        .to_string();
                    anyhow::bail!(message);
                }
                _ => {}
            }
        }

        // Reconstruct the assistant turn's content blocks, in the order they streamed.
        let mut content_blocks: Vec<Value> = Vec::new();
        let mut tool_uses: Vec<(String, String, Value)> = Vec::new(); // (tool_use_id, qualified_name, input)
        for index in &order {
            match blocks.get(index) {
                Some(BlockAcc::Text(text)) => {
                    content_blocks.push(json!({ "type": "text", "text": text }));
                }
                Some(BlockAcc::ToolUse { id, name, input_json }) => {
                    let input: Value = serde_json::from_str(input_json).unwrap_or_else(|_| json!({}));
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                    tool_uses.push((id.clone(), name.clone(), input));
                }
                None => {}
            }
        }

        messages.push(json!({ "role": "assistant", "content": content_blocks }));

        if tool_uses.is_empty() {
            let _ = tx.send(StreamEvent::Done).await;
            return Ok(());
        }

        let mut tool_results: Vec<Value> = Vec::new();
        for (tool_use_id, qualified_name, input) in tool_uses {
            let _ = tx
                .send(StreamEvent::ToolCall { name: qualified_name.clone() })
                .await;

            let result_text = call_mcp_tool(&qualified_name, input, mcp_clients).await;

            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": result_text,
            }));
        }

        messages.push(json!({ "role": "user", "content": tool_results }));
    }

    anyhow::bail!("tool-use loop exceeded {MAX_TURNS} turns without a final answer")
}

/// Routes a "{server}__{tool}" call to the right MCP server and returns the text to feed back to
/// the model. Failures come back as text too, so the model can see and react to them. Shared by
/// this loop and the OpenAI-compatible one in agent_openai.rs.
pub async fn call_mcp_tool(
    qualified_name: &str,
    input: Value,
    mcp_clients: &HashMap<String, Arc<McpClient>>,
) -> String {
    let (server, tool) = qualified_name.split_once("__").unwrap_or(("", qualified_name));

    match mcp_clients.get(server) {
        Some(mcp) => match mcp.call_tool(tool, input).await {
            Ok(text) => truncate_tool_result(text),
            Err(e) => format!("Error calling tool: {e}"),
        },
        None => format!("Error: no MCP server registered for '{server}'"),
    }
}

/// Tool results are fed back to the model on every later turn, so one huge result (e.g. every
/// UEBA endpoint) can push the conversation past the model's context window. Cap each result and
/// tell the model it was cut, so it can retry with a narrower query instead of trusting a partial.
fn truncate_tool_result(text: String) -> String {
    if text.len() <= MAX_TOOL_RESULT_CHARS {
        return text;
    }
    let mut cut = MAX_TOOL_RESULT_CHARS;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n\n[TRUNCATED: this tool result was {} characters and only the first {} are shown. \
         Do not treat it as complete. Re-run the tool with a narrower query (fewer fields, a \
         lower limit, stricter filters or a shorter time range).]",
        &text[..cut],
        text.len(),
        cut
    )
}
