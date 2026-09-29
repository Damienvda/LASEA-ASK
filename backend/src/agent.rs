//! The Anthropic tool-use loop: streams a chat turn, and whenever Claude asks to invoke a tool,
//! calls it against the right MCP server, feeds the result back, and continues — repeating until
//! Claude answers without requesting a tool, or `max_turns` is hit as a safety valve.
//!
//! The wire format here is Anthropic-specific (content-block streaming, `tool_use`/`tool_result`
//! blocks). OpenAI-compatible providers (OpenAI, Mistral) use a different tool-calling format and
//! have their own loop in agent_openai.rs; both share `call_mcp_tool`, `Reply` and the completion
//! check below. routes.rs picks the right loop whenever at least one MCP tool is configured, and
//! otherwise uses the plain Provider::stream_chat path.

use crate::config::{AgentConfig, ProviderConfig};
use crate::mcp::McpClient;
use crate::providers::sse::sse_events;
use crate::providers::{ChatMessage, StreamEvent};
use crate::tool_result;
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

/// Sent as a user turn when the model gives its final answer after using tools (see
/// `should_check`). Models tend to stop at the first plausible answer; this makes it look for
/// what it skipped before the answer counts as final.
pub const COMPLETION_CHECK: &str = "\
[Automatic completion check from LASEASK, not written by the user.] Before your answer is final, \
review it against the question, in this order of priority:
1. Leads you saw but did not check: IPs, users, hosts, alerts or incidents that appeared in any \
result (also as a client or source) and could change the answer.
2. Completeness: did you exclude what you found and re-query until nothing new appeared? Did you \
treat a sample as complete (a single page, a PAGING NOTICE, a TRUNCATED result)?
3. Claims: is any statement based on a field you may have misread, or unverified when a tool \
call could confirm it? Is any part of the question unanswered, or any plan step skipped?
Refining numbers you already labelled as estimates is the lowest priority.
If everything is covered, reply with exactly DONE and nothing else. Otherwise, call the tools you \
need now, then write the complete, corrected final answer (all of it, not only the additions, \
and without the word DONE).";

/// Appended to the user's question on the planning turn (`[agent] plan_first`), which runs with
/// tools disabled.
const PLAN_REQUEST: &str = "\n\n[Automatic note from LASEASK: before using any tool, reply with \
your investigation plan only: a short numbered checklist of what you will check, with which tool, \
over which time window, and what would confirm or rule out each point. Do not answer yet.]";

/// Sent after the plan to start carrying it out.
pub const PLAN_GO: &str = "[Automatic message from LASEASK, not written by the user.] Now carry \
out your plan with the tools, step by step, adding steps when the results call for it. Then give \
the final answer.";

/// Adds `PLAN_REQUEST` to the user's question (the last user message).
pub fn request_plan(messages: &mut [Value]) {
    let question = messages
        .iter_mut()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m.get_mut("content"));
    if let Some(Value::String(text)) = question {
        text.push_str(PLAN_REQUEST);
    }
}

/// Shown to the user when the completion check sends the model back to work.
const CHECK_SEPARATOR: &str = "\n\n---\n\n*Completion check: following up on open points.*\n\n";

/// Whether to run the completion check now that the model has answered without asking for tools.
/// Once per question, only after tools were used, and only with turns left to act on it.
pub fn should_check(cfg: &AgentConfig, checked: bool, tools_used: usize, turn: usize) -> bool {
    cfg.completion_check && !checked && tools_used > 0 && turn + 2 < cfg.max_turns
}

/// The completion check's "nothing left to do" reply.
fn is_done(text: &str) -> bool {
    let t = text.trim().trim_matches(|c: char| !c.is_alphanumeric());
    t.is_empty() || t.eq_ignore_ascii_case("done")
}

/// Streams the model's text to the browser. Keeps separate turns apart (the text before and after
/// a tool call would otherwise run together), and holds back the completion-check turn until it's
/// clear whether it's more than "DONE".
pub struct Reply<'a> {
    tx: &'a Sender<StreamEvent>,
    emitted: bool,
    turn_started: bool,
    checking: bool,
    held: String,
}

impl<'a> Reply<'a> {
    pub fn new(tx: &'a Sender<StreamEvent>) -> Self {
        Self { tx, emitted: false, turn_started: false, checking: false, held: String::new() }
    }

    pub fn start_turn(&mut self) {
        self.turn_started = false;
    }

    pub async fn text(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        if self.checking {
            self.held.push_str(chunk);
            return;
        }
        if self.emitted && !self.turn_started {
            self.send("\n\n").await;
        }
        self.turn_started = true;
        self.emitted = true;
        self.send(chunk).await;
    }

    pub fn checking(&self) -> bool {
        self.checking
    }

    pub fn start_check(&mut self) {
        self.checking = true;
        self.held.clear();
    }

    /// Ends the completion-check turn: if it's more than "DONE", shows the separator and what the
    /// model said. Returns true when the model is done.
    pub async fn end_check(&mut self) -> bool {
        self.checking = false;
        let held = std::mem::take(&mut self.held);
        if is_done(&held) {
            return true;
        }
        self.send(CHECK_SEPARATOR).await;
        self.emitted = true;
        self.turn_started = true;
        self.send(&held).await;
        false
    }

    async fn send(&self, text: &str) {
        if !text.is_empty() {
            let _ = self.tx.send(StreamEvent::Delta { text: text.to_string() }).await;
        }
    }
}

enum BlockAcc {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
    /// Models with thinking on (always, on Opus 5.5 / Fable 5.1) stream these before their text
    /// and tool calls. They must go back unchanged, signature included, with the tool results.
    Thinking {
        thinking: String,
        signature: String,
    },
    /// Any other block (e.g. `redacted_thinking`), arriving whole: sent back as-is.
    Other(Value),
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
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
    provider: &ProviderConfig,
    model: &str,
    initial_messages: &[ChatMessage],
    tools: &[ToolSpec],
    mcp_clients: &HashMap<String, Arc<McpClient>>,
    force_first_tool_use: bool,
    cfg: &AgentConfig,
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
        .filter(|m| m.role != "system" && !m.content.trim().is_empty())
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
    let mut reply = Reply::new(tx);
    let mut tools_used = 0;
    let mut checked = false;
    let planning = cfg.plan_first;
    if planning {
        request_plan(&mut messages);
    }
    let first_tool_turn = usize::from(planning);
    let mut forced_tool_use_supported = true;
    let mut effort_supported = true;

    for turn in 0..cfg.max_turns {
        // The browser went away (Stop button, tab closed): don't keep paying for API calls and
        // tool runs nobody will see.
        if tx.is_closed() {
            return Ok(());
        }
        let mut body = json!({
            "model": model,
            "max_tokens": cfg.max_output_tokens,
            "stream": true,
            "messages": messages,
            "tools": tools_json,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(t) = provider.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(effort) = provider.effort.as_deref().filter(|_| effort_supported) {
            body["output_config"] = json!({ "effort": effort });
        }
        // Every turn resends the whole conversation (tools, system prompt, all tool results so
        // far). Automatic caching bills the part already sent at the cache-read price (a tenth
        // of the input price) instead of the full price again.
        body["cache_control"] = json!({ "type": "ephemeral" });
        if planning && turn == 0 {
            body["tool_choice"] = json!({ "type": "none" });
        }
        // "MCP only" mode: force the first tool turn to invoke a tool rather than let Claude
        // answer from its own knowledge. This is an actual API-level constraint (Anthropic's
        // tool_choice), not a prompt nudge, so it's reliable. Only that turn — forcing it on
        // every turn would mean the model could never stop calling tools to produce a final
        // text-only answer. The newest models (Opus 5.5, Fable 5.1) reject forced tool use; then
        // the request is retried without it and the system prompt's MCP-only rule (prompt.rs)
        // does the job.
        if force_first_tool_use && forced_tool_use_supported && turn == first_tool_turn {
            body["tool_choice"] = json!({ "type": "any" });
        }
        // Last turn: forbid tools so the model has to answer with what it has gathered, instead
        // of the loop ending on an error after all that work.
        if turn + 1 == cfg.max_turns {
            body["tool_choice"] = json!({ "type": "none" });
        }

        let resp = loop {
            let resp = client
                .post(API_URL)
                .header("x-api-key", &provider.api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;
            if resp.status().is_success() {
                break resp;
            }
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let forced = body.pointer("/tool_choice/type").and_then(Value::as_str) == Some("any");
            if status == reqwest::StatusCode::BAD_REQUEST && forced && text.contains("tool_choice") {
                tracing::warn!("{model} rejects forced tool use; continuing with tool_choice auto");
                forced_tool_use_supported = false;
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("tool_choice");
                }
                continue;
            }
            // Older models (Sonnet 4.5, Haiku 4.5) reject `effort`.
            if status == reqwest::StatusCode::BAD_REQUEST
                && body.get("output_config").is_some()
                && text.contains("effort")
            {
                tracing::warn!("{model} rejects effort; continuing without it");
                effort_supported = false;
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("output_config");
                }
                continue;
            }
            // Same for sampling parameters, which Opus 4.7+ / Sonnet 5 and newer reject.
            if status == reqwest::StatusCode::BAD_REQUEST
                && body.get("temperature").is_some()
                && text.contains("temperature")
            {
                tracing::warn!("{model} rejects temperature; continuing without it");
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("temperature");
                }
                continue;
            }
            anyhow::bail!("anthropic error {status}: {text}");
        };

        let mut events = sse_events(resp.bytes_stream());

        // Per-content-block accumulators, indexed by the block's `index` in this turn.
        let mut blocks: HashMap<u64, BlockAcc> = HashMap::new();
        let mut order: Vec<u64> = Vec::new();
        let mut input_tokens = 0;
        let mut cache_read = 0;
        let mut cache_write = 0;
        let mut output_tokens = 0;
        let mut stop_reason = String::new();
        reply.start_turn();

        while let Some(data) = events.next().await {
            let v: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v.get("type").and_then(|t| t.as_str()) {
                Some("message_start") => {
                    let usage = |k: &str| {
                        v.pointer(&format!("/message/usage/{k}")).and_then(Value::as_u64).unwrap_or(0)
                    };
                    input_tokens = usage("input_tokens");
                    cache_read = usage("cache_read_input_tokens");
                    cache_write = usage("cache_creation_input_tokens");
                }
                Some("message_delta") => {
                    if let Some(n) = v.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                        output_tokens = n;
                    }
                    if let Some(s) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                        stop_reason = s.to_string();
                    }
                }
                Some("content_block_start") => {
                    let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let block = v.get("content_block").cloned().unwrap_or(json!({}));
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                    order.push(index);
                    let str_field = |k: &str| block.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let acc = match block_type {
                        "tool_use" => BlockAcc::ToolUse {
                            id: str_field("id"),
                            name: str_field("name"),
                            input_json: String::new(),
                        },
                        "text" => BlockAcc::Text(str_field("text")),
                        "thinking" => BlockAcc::Thinking {
                            thinking: str_field("thinking"),
                            signature: str_field("signature"),
                        },
                        _ => BlockAcc::Other(block.clone()),
                    };
                    blocks.insert(index, acc);
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
                            reply.text(text).await;
                        }
                        Some("input_json_delta") => {
                            let partial = delta.get("partial_json").and_then(|t| t.as_str()).unwrap_or("");
                            if let Some(BlockAcc::ToolUse { input_json, .. }) = blocks.get_mut(&index) {
                                input_json.push_str(partial);
                            }
                        }
                        Some("thinking_delta") => {
                            let part = delta.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                            if let Some(BlockAcc::Thinking { thinking, .. }) = blocks.get_mut(&index) {
                                thinking.push_str(part);
                            }
                        }
                        Some("signature_delta") => {
                            let part = delta.get("signature").and_then(|t| t.as_str()).unwrap_or("");
                            if let Some(BlockAcc::Thinking { signature, .. }) = blocks.get_mut(&index) {
                                signature.push_str(part);
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

        tracing::info!(
            "anthropic turn {}: {input_tokens} input tokens (+{cache_read} from cache, {cache_write} written to cache), {output_tokens} output tokens, stop: {stop_reason}",
            turn + 1
        );

        // Reconstruct the assistant turn's content blocks, in the order they streamed.
        let mut content_blocks: Vec<Value> = Vec::new();
        let mut tool_uses: Vec<(String, String, Value)> = Vec::new(); // (tool_use_id, qualified_name, input)
        for index in &order {
            match blocks.get(index) {
                // Anthropic rejects empty text blocks when they're sent back ("text content
                // blocks must be non-empty"), and Claude does stream some, e.g. before a tool call.
                Some(BlockAcc::Text(text)) if text.trim().is_empty() => {}
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
                Some(BlockAcc::Thinking { thinking, signature }) => {
                    content_blocks.push(json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": signature,
                    }));
                }
                Some(BlockAcc::Other(block)) => content_blocks.push(block.clone()),
                None => {}
            }
        }

        // An assistant turn without any text or tool call (empty, or thinking only) is rejected.
        let has_output = content_blocks.iter().any(|b| {
            matches!(b.get("type").and_then(Value::as_str), Some("text" | "tool_use"))
        });
        if !has_output {
            content_blocks.push(json!({ "type": "text", "text": "(no output)" }));
        }
        messages.push(json!({ "role": "assistant", "content": content_blocks }));

        if planning && turn == 0 {
            messages.push(json!({ "role": "user", "content": PLAN_GO }));
            continue;
        }

        if tool_uses.is_empty() {
            if reply.checking() {
                reply.end_check().await;
            } else {
                if stop_reason == "max_tokens" {
                    reply
                        .text("\n\n*[Answer cut off: the output limit ([agent] max_output_tokens) was reached.]*")
                        .await;
                }
                if should_check(cfg, checked, tools_used, turn) {
                    checked = true;
                    reply.start_check();
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
        tools_used += tool_uses.len();

        let mut tool_results: Vec<Value> = Vec::new();
        for (tool_use_id, qualified_name, input) in tool_uses {
            let _ = tx
                .send(StreamEvent::ToolCall { name: qualified_name.clone() })
                .await;

            let result_text = call_mcp_tool(&qualified_name, input, mcp_clients, cfg).await;

            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": result_text,
            }));
        }

        messages.push(json!({ "role": "user", "content": tool_results }));
    }

    anyhow::bail!("tool-use loop exceeded {} turns without a final answer", cfg.max_turns)
}

/// Routes a "{server}__{tool}" call to the right MCP server and returns the text to feed back to
/// the model. Failures come back as text too, so the model can see and react to them. Shared by
/// this loop and the OpenAI-compatible one in agent_openai.rs.
pub async fn call_mcp_tool(
    qualified_name: &str,
    input: Value,
    mcp_clients: &HashMap<String, Arc<McpClient>>,
    cfg: &AgentConfig,
) -> String {
    let (server, tool) = qualified_name.split_once("__").unwrap_or(("", qualified_name));

    // The arguments are what shows whether the model narrowed its query (time range, filters,
    // fields) or paged on, so log them; capped, a filter list can be long.
    let args: String = input.to_string().chars().take(500).collect();
    tracing::info!("tool call {qualified_name} {args}");

    match mcp_clients.get(server) {
        Some(mcp) => match mcp.call_tool(tool, input.clone()).await {
            Ok(text) => tool_result::prepare(tool, &input, text, cfg.max_tool_result_chars),
            Err(e) => format!("Error calling tool: {e:#}"),
        },
        None => format!("Error: no MCP server registered for '{server}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::is_done;

    #[test]
    fn done_replies() {
        assert!(is_done("DONE"));
        assert!(is_done(" Done. "));
        assert!(is_done("**DONE**"));
        assert!(!is_done("Not done: I still need to check 10.0.0.5."));
    }
}
