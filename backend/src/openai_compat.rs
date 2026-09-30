//! OpenAI-compatible API (`/v1/models`, `/v1/chat/completions`) so that Open WebUI can use
//! LASEASK as a model. Open WebUI keeps the conversations, users, knowledge bases (RAG), notes,
//! channels and usage statistics; every request still goes through LASEASK's own agent, MCP
//! tools, system prompt and tool-result compaction (routes::start_chat).
//!
//! How the agent's events map onto the OpenAI stream:
//! - the answer                          -> `delta.content`
//! - thinking, tool calls, working notes -> `delta.reasoning_content`, which Open WebUI shows as
//!   a folded "Thought" block, live while the investigation runs
//! - token usage                         -> `usage` on the last chunk (Open WebUI's statistics)
//!
//! Only the answer is ever stored as the reply's text, and anything Open WebUI folds into its
//! messages (`<details>` blocks, `<think>` tags) is stripped before a conversation is sent back to
//! the model, so earlier investigations don't cost tokens again on the next question.

use crate::config::ModelPricing;
use crate::error::AppError;
use crate::providers::{ChatMessage, StreamEvent};
use crate::routes::{start_chat, ChatParams};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[derive(Debug, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<InMessage>,
    #[serde(default)]
    pub stream: bool,
    /// Open WebUI may tag its own background requests (title, tags, follow-ups...).
    #[serde(default)]
    pub metadata: Option<Value>,
    /// Per-request options, same fields as `<laseask-options>` (see `RequestOptions`), for
    /// clients that can add body fields.
    #[serde(default)]
    pub laseask: Option<RequestOptions>,
}

/// What the Open WebUI filters (deploy/openwebui/functions) switch per chat: the LASEASK UI's
/// MCP checkboxes and "MCP only". They travel as `<laseask-options>{json}</laseask-options>` in a
/// system message, which is removed before the model sees it.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct RequestOptions {
    #[serde(default)]
    pub tools: Option<bool>,
    #[serde(default)]
    pub mcp_only: Option<bool>,
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
}

impl RequestOptions {
    /// `other`'s fields win where set.
    fn merge(mut self, other: RequestOptions) -> Self {
        self.tools = other.tools.or(self.tools);
        self.mcp_only = other.mcp_only.or(self.mcp_only);
        self.mcp_servers = other.mcp_servers.or(self.mcp_servers);
        self
    }
}

const OPTIONS_OPEN: &str = "<laseask-options>";
const OPTIONS_CLOSE: &str = "</laseask-options>";

#[derive(Debug, Deserialize)]
pub struct InMessage {
    pub role: String,
    /// A string, or OpenAI's list of parts (`{"type": "text", "text": ...}`, images...).
    #[serde(default)]
    pub content: Value,
}

pub async fn list_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    let data: Vec<Value> = state
        .config
        .openwebui
        .all_models(&state.config.providers)
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "object": "model",
                "created": 0,
                "owned_by": "laseask",
                "name": m.display_name(),
            })
        })
        .collect();
    Json(json!({ "object": "list", "data": data })).into_response()
}

pub async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CompletionRequest>,
) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    let owui = &state.config.openwebui;
    let Some(preset) = owui.resolve(&state.config.providers, &req.model) else {
        return AppError::BadRequest(format!("unknown model '{}'", req.model)).into_response();
    };

    let (messages, marked) = convert_messages(&req.messages);
    if !messages.iter().any(|m| m.role == "user") {
        return AppError::BadRequest("the conversation has no user message".into()).into_response();
    }
    let options = marked.merge(req.laseask.clone().unwrap_or_default());

    // Open WebUI's background tasks (titles, tags, follow-up questions, search queries) should go
    // to its task model (TASK_MODEL_EXTERNAL = laseask-chat). If one reaches a tool model anyway,
    // answer it plainly: no investigation, no LASEASK prompt.
    let task = is_background_task(&messages, req.metadata.as_ref());
    let use_tools = options.tools.unwrap_or(preset.tools) && !task;
    let servers = options
        .mcp_servers
        .filter(|s| !s.is_empty())
        .or_else(|| preset.mcp_servers.clone());
    let params = ChatParams {
        provider: preset.provider.clone(),
        model: preset.model.clone(),
        messages,
        mcp_only: options.mcp_only.unwrap_or(preset.mcp_only) && use_tools,
        mcp_servers: if use_tools { servers } else { Some(Vec::new()) },
        bare: task,
    };

    let started = match start_chat(&state, params).await {
        Ok(started) => started,
        Err(e) => return e.into_response(),
    };

    let translator = Translator::new(
        req.model.clone(),
        Lang::parse(&owui.language),
        owui.live_answer || !started.has_tools,
        state.config.pricing.get(&started.model).cloned(),
    );

    if req.stream {
        stream_response(started.events, translator)
    } else {
        collect_response(started.events, translator).await
    }
}

// ---------------------------------------------------------------------------------------------
// Authentication: Open WebUI sends the shared key as a bearer token.

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    constant_time_eq(state.config.openwebui.api_key.trim().as_bytes(), given.as_bytes())
}

pub(crate) fn constant_time_eq(expected: &[u8], given: &[u8]) -> bool {
    if expected.is_empty() || expected.len() != given.len() {
        return false;
    }
    expected.iter().zip(given).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": { "message": "invalid or missing API key", "type": "invalid_request_error" } })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------------------------
// Incoming messages.

/// OpenAI messages -> LASEASK messages: text only, "developer" treated as "system", tool-role
/// messages dropped, Open WebUI's folded blocks stripped from earlier replies, empty messages
/// dropped and consecutive messages of the same role merged (Anthropic wants alternating turns).
/// Also takes the `<laseask-options>` out of system messages.
fn convert_messages(input: &[InMessage]) -> (Vec<ChatMessage>, RequestOptions) {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(input.len());
    let mut options = RequestOptions::default();
    for m in input {
        let role = match m.role.as_str() {
            "system" | "developer" => "system",
            "user" => "user",
            "assistant" => "assistant",
            _ => continue,
        };
        let mut text = text_of(&m.content);
        if role == "assistant" {
            text = strip_folded_blocks(&text);
        }
        if role == "system" && text.contains(OPTIONS_OPEN) {
            let (rest, found) = take_options(&text);
            text = rest;
            for o in found {
                options = options.merge(o);
            }
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(prev) if prev.role == role && role != "system" => {
                prev.content.push_str("\n\n");
                prev.content.push_str(&text);
            }
            _ => out.push(ChatMessage { role: role.to_string(), content: text }),
        }
    }
    (out, options)
}

/// Removes every `<laseask-options>{json}</laseask-options>` from a system message and returns
/// the parsed options, in order. Malformed JSON is dropped (and logged), never shown to the model.
fn take_options(text: &str) -> (String, Vec<RequestOptions>) {
    let mut found = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPTIONS_OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPTIONS_OPEN.len()..];
        match after.find(OPTIONS_CLOSE) {
            Some(end) => {
                match serde_json::from_str::<RequestOptions>(after[..end].trim()) {
                    Ok(o) => found.push(o),
                    Err(e) => tracing::warn!("ignoring malformed <laseask-options>: {e}"),
                }
                rest = &after[end + OPTIONS_CLOSE.len()..];
            }
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    (out, found)
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Removes `<details ...>...</details>` (Open WebUI's folded reasoning and tool blocks) and
/// `<think>...</think>` from a previous reply.
fn strip_folded_blocks(text: &str) -> String {
    let without_details = remove_blocks(text, "<details", "</details>");
    remove_blocks(&without_details, "<think>", "</think>")
}

fn remove_blocks(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        match rest[start..].find(close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Open WebUI's task prompts start with "### Task:" and carry the chat in `<chat_history>`. Its
/// knowledge-base (RAG) prompt also starts with "### Task:", but carries `<context>`: that one is
/// a real question and keeps its tools.
fn is_background_task(messages: &[ChatMessage], metadata: Option<&Value>) -> bool {
    if metadata
        .and_then(|m| m.get("task"))
        .and_then(Value::as_str)
        .is_some_and(|t| !t.is_empty())
    {
        return true;
    }
    messages.iter().rev().find(|m| m.role == "user").is_some_and(|m| {
        m.content.trim_start().starts_with("### Task:")
            && m.content.contains("<chat_history>")
            && !m.content.contains("<context>")
    })
}

// ---------------------------------------------------------------------------------------------
// Outgoing stream.

#[derive(Clone, Copy)]
enum Lang {
    Fr,
    En,
}

impl Lang {
    fn parse(s: &str) -> Self {
        if s.trim().to_ascii_lowercase().starts_with("en") {
            Lang::En
        } else {
            Lang::Fr
        }
    }
    fn notes(self) -> &'static str {
        match self {
            Lang::Fr => "📝 Notes de travail",
            Lang::En => "📝 Working notes",
        }
    }
    fn follow_up(self) -> &'static str {
        match self {
            Lang::Fr => "↻ Vérification de complétude : reprise sur les points ouverts",
            Lang::En => "↻ Completion check: following up on open points",
        }
    }
    fn chars(self) -> &'static str {
        match self {
            Lang::Fr => "car.",
            Lang::En => "chars",
        }
    }
    fn tool_calls(self) -> &'static str {
        match self {
            Lang::Fr => "appel(s) d'outils",
            Lang::En => "tool call(s)",
        }
    }
    fn failed(self) -> &'static str {
        match self {
            Lang::Fr => "en erreur",
            Lang::En => "failed",
        }
    }
    fn tokens(self) -> (&'static str, &'static str) {
        match self {
            Lang::Fr => ("entrée", "sortie"),
            Lang::En => ("in", "out"),
        }
    }
    fn error(self) -> &'static str {
        match self {
            Lang::Fr => "Erreur LASEASK :",
            Lang::En => "LASEASK error:",
        }
    }
}

enum Out {
    Reasoning(String),
    Content(String),
}

/// Turns the agent's events into the two OpenAI channels (answer and reasoning).
struct Translator {
    id: String,
    model: String,
    created: u64,
    lang: Lang,
    /// true: answer text streams as it comes. false: text is held until we know whether a tool
    /// call follows (then it was a working note) or the reply ends (then it's the answer).
    live: bool,
    pricing: Option<ModelPricing>,
    segment: String,
    tool_names: HashMap<String, String>,
    tool_calls: usize,
    tool_errors: usize,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_write: u64,
    started: Instant,
    finished: bool,
}

impl Translator {
    fn new(model: String, lang: Lang, live: bool, pricing: Option<ModelPricing>) -> Self {
        let created = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self {
            id: format!("chatcmpl-laseask-{created}-{}", std::process::id()),
            model,
            created,
            lang,
            live,
            pricing,
            segment: String::new(),
            tool_names: HashMap::new(),
            tool_calls: 0,
            tool_errors: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            started: Instant::now(),
            finished: false,
        }
    }

    fn on_event(&mut self, ev: StreamEvent) -> Vec<Out> {
        match ev {
            StreamEvent::Delta { text } => {
                if self.live {
                    vec![Out::Content(text)]
                } else {
                    self.segment.push_str(&text);
                    Vec::new()
                }
            }
            StreamEvent::Thinking { text } => vec![Out::Reasoning(text)],
            StreamEvent::ToolStart { id, name, args } => {
                let mut out = self.flush_note();
                self.tool_calls += 1;
                out.push(Out::Reasoning(format!("\n\n▶ `{name}` {}\n", short(&args.to_string(), 300))));
                self.tool_names.insert(id, name);
                out
            }
            StreamEvent::ToolEnd { id, ok, duration_ms, chars, excerpt } => {
                if !ok {
                    self.tool_errors += 1;
                }
                let name = self.tool_names.get(&id).cloned().unwrap_or_default();
                let mark = if ok { "✔" } else { "✖" };
                let excerpt = short(&excerpt.replace(['\n', '\r'], " "), 200);
                vec![Out::Reasoning(format!(
                    "{mark} `{name}` · {:.1} s · {chars} {} — {excerpt}\n",
                    duration_ms as f64 / 1000.0,
                    self.lang.chars()
                ))]
            }
            StreamEvent::Usage { input_tokens, output_tokens, cache_read, cache_write } => {
                self.input_tokens += input_tokens;
                self.output_tokens += output_tokens;
                self.cache_read += cache_read;
                self.cache_write += cache_write;
                Vec::new()
            }
            StreamEvent::FollowUp => {
                let mut out = self.flush_note();
                out.push(Out::Reasoning(format!("\n\n{}\n", self.lang.follow_up())));
                out
            }
            StreamEvent::Done => self.finish(),
            StreamEvent::Error { message } => {
                let mut out = self.finish();
                out.push(Out::Content(format!("\n\n**{}** {message}", self.lang.error())));
                out
            }
        }
    }

    /// Text written before a tool call or a follow-up was a working note, not the answer.
    fn flush_note(&mut self) -> Vec<Out> {
        if self.segment.trim().is_empty() {
            self.segment.clear();
            return Vec::new();
        }
        let note = std::mem::take(&mut self.segment);
        vec![Out::Reasoning(format!("\n\n{}\n\n{}\n", self.lang.notes(), note.trim()))]
    }

    /// End of the reply: the summary line (in the reasoning), then the held-back answer.
    fn finish(&mut self) -> Vec<Out> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut out = Vec::new();
        if self.tool_calls > 0 {
            out.push(Out::Reasoning(format!("\n\n— {}\n", self.summary())));
        }
        let answer = std::mem::take(&mut self.segment);
        if !answer.trim().is_empty() {
            out.push(Out::Content(answer));
        }
        out
    }

    fn summary(&self) -> String {
        let (tin, tout) = self.lang.tokens();
        let mut parts = Vec::new();
        let mut calls = format!("{} {}", self.tool_calls, self.lang.tool_calls());
        if self.tool_errors > 0 {
            calls.push_str(&format!(" ({} {})", self.tool_errors, self.lang.failed()));
        }
        parts.push(calls);
        parts.push(elapsed(self.started.elapsed().as_secs()));
        let mut tokens = format!("{} {tin} / {} {tout}", self.input_tokens + self.cache_read + self.cache_write, self.output_tokens);
        if self.cache_read > 0 {
            tokens.push_str(&format!(" ({} cache)", self.cache_read));
        }
        parts.push(tokens);
        if let Some(p) = &self.pricing {
            let cost = (self.input_tokens as f64 * p.input
                + self.output_tokens as f64 * p.output
                + self.cache_read as f64 * p.cache_read.unwrap_or(p.input)
                + self.cache_write as f64 * p.cache_write.unwrap_or(p.input))
                / 1_000_000.0;
            parts.push(format!("≈ {cost:.2} $"));
        }
        parts.join(" · ")
    }

    fn chunk(&self, delta: Value, finish_reason: Option<&str>) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish_reason }],
        })
    }

    fn out_chunk(&self, out: Out) -> Value {
        match out {
            Out::Reasoning(text) => self.chunk(json!({ "reasoning_content": text }), None),
            Out::Content(text) => self.chunk(json!({ "content": text }), None),
        }
    }

    fn usage(&self) -> Value {
        let prompt = self.input_tokens + self.cache_read + self.cache_write;
        json!({
            "prompt_tokens": prompt,
            "completion_tokens": self.output_tokens,
            "total_tokens": prompt + self.output_tokens,
        })
    }

    fn final_chunk(&self) -> Value {
        let mut chunk = self.chunk(json!({}), Some("stop"));
        chunk["usage"] = self.usage();
        chunk
    }
}

fn stream_response(mut events: BoxStream<'static, StreamEvent>, mut tr: Translator) -> Response {
    let (tx, rx) = mpsc::channel::<String>(64);
    tokio::spawn(async move {
        // When Open WebUI goes away (Stop button), a send fails and this task ends, which drops
        // `events` and with it the agent's channel: the agent stops at its next turn.
        let first = tr.chunk(json!({ "role": "assistant", "content": "" }), None);
        if tx.send(first.to_string()).await.is_err() {
            return;
        }
        while let Some(ev) = events.next().await {
            let last = matches!(ev, StreamEvent::Done | StreamEvent::Error { .. });
            for out in tr.on_event(ev) {
                if tx.send(tr.out_chunk(out).to_string()).await.is_err() {
                    return;
                }
            }
            if last {
                break;
            }
        }
        for out in tr.finish() {
            if tx.send(tr.out_chunk(out).to_string()).await.is_err() {
                return;
            }
        }
        let _ = tx.send(tr.final_chunk().to_string()).await;
        let _ = tx.send("[DONE]".to_string()).await;
    });

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|data| (Ok::<_, Infallible>(Event::default().data(data)), rx))
    });
    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

async fn collect_response(mut events: BoxStream<'static, StreamEvent>, mut tr: Translator) -> Response {
    let mut content = String::new();
    let mut reasoning = String::new();
    let take = |outs: Vec<Out>, content: &mut String, reasoning: &mut String| {
        for out in outs {
            match out {
                Out::Content(t) => content.push_str(&t),
                Out::Reasoning(t) => reasoning.push_str(&t),
            }
        }
    };
    while let Some(ev) = events.next().await {
        let last = matches!(ev, StreamEvent::Done | StreamEvent::Error { .. });
        take(tr.on_event(ev), &mut content, &mut reasoning);
        if last {
            break;
        }
    }
    take(tr.finish(), &mut content, &mut reasoning);

    let mut message = json!({ "role": "assistant", "content": content });
    if !reasoning.trim().is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    Json(json!({
        "id": tr.id,
        "object": "chat.completion",
        "created": tr.created,
        "model": tr.model,
        "choices": [{ "index": 0, "message": message, "finish_reason": "stop" }],
        "usage": tr.usage(),
    }))
    .into_response()
}

/// At most `max` characters, cut on a character boundary.
fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut cut: String = s.chars().take(max).collect();
        cut.push('…');
        cut
    }
}

fn elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{} min {:02} s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_folded_blocks_from_replies() {
        let text = "<details type=\"reasoning\" done=\"true\">\n<summary>Thought</summary>\nx\n</details>\nAnswer <think>hidden</think>here";
        assert_eq!(strip_folded_blocks(text).trim(), "Answer here");
    }

    #[test]
    fn merges_same_role_and_drops_empty() {
        let input = vec![
            InMessage { role: "system".into(), content: json!("s") },
            InMessage { role: "user".into(), content: json!("a") },
            InMessage { role: "assistant".into(), content: json!("<details>only</details>") },
            InMessage { role: "user".into(), content: json!([{ "type": "text", "text": "b" }]) },
        ];
        let (out, _) = convert_messages(&input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].content, "a\n\nb");
    }

    #[test]
    fn reads_and_removes_options_from_system_messages() {
        let input = vec![
            InMessage {
                role: "system".into(),
                content: json!("<laseask-options>{\"mcp_only\":true}</laseask-options><laseask-options>{\"mcp_servers\":[\"intel\"]}</laseask-options>"),
            },
            InMessage { role: "system".into(), content: json!("Knowledge: x <laseask-options>{\"tools\":false}</laseask-options>") },
            InMessage { role: "user".into(), content: json!("q") },
        ];
        let (out, options) = convert_messages(&input);
        assert_eq!(options.mcp_only, Some(true));
        assert_eq!(options.tools, Some(false));
        assert_eq!(options.mcp_servers, Some(vec!["intel".to_string()]));
        // The first system message held nothing else and is dropped; the second keeps its text.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content, "Knowledge: x");
    }

    #[test]
    fn detects_tasks_but_not_rag() {
        let task = ChatMessage { role: "user".into(), content: "### Task:\nGenerate a title\n<chat_history>x</chat_history>".into() };
        let rag = ChatMessage { role: "user".into(), content: "### Task:\nRespond\n<context>doc</context>\n<chat_history>x</chat_history>".into() };
        assert!(is_background_task(&[task], None));
        assert!(!is_background_task(&[rag], None));
    }

    #[test]
    fn working_notes_go_to_reasoning_and_answer_to_content() {
        let mut tr = Translator::new("laseask".into(), Lang::Fr, false, None);
        assert!(tr.on_event(StreamEvent::Delta { text: "plan".into() }).is_empty());
        let outs = tr.on_event(StreamEvent::ToolStart { id: "1".into(), name: "t".into(), args: json!({}) });
        assert!(matches!(&outs[0], Out::Reasoning(t) if t.contains("plan")));
        tr.on_event(StreamEvent::Delta { text: "answer".into() });
        let outs = tr.on_event(StreamEvent::Done);
        assert!(matches!(outs.last(), Some(Out::Content(t)) if t == "answer"));
    }
}
