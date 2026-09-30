//! LASEASK's tools as an MCP server, for Open WebUI (Admin Panel > Settings > External Tools, type
//! MCP / Streamable HTTP, URL http://laseask:8787/mcp, Bearer = `[tool_server] api_key`).
//!
//! Open WebUI is the tool: it has the models, the conversations and the tool-calling loop. What
//! LASEASK adds sits between Open WebUI and the real MCP servers:
//! - one tool list across every `[mcp.*]` server, with `include_tools` / `exclude_tools` (the
//!   FortiAnalyzer tools that change things are never offered), plus the "intel" lookups;
//! - every result compacted and capped (tool_result.rs), so a big log pull doesn't eat the
//!   model's context;
//! - the investigation rules and each server's guide as the MCP `instructions`, and as a full
//!   system prompt on GET /api/prompt for the Open WebUI filter that injects it.
//!
//! Transport: JSON-RPC 2.0 over POST, answered with plain `application/json` (allowed by the
//! Streamable HTTP transport). No session, no server-initiated messages: GET is refused with 405.

use crate::agent::{self, ToolSpec};
use crate::openai_compat::constant_time_eq;
use crate::prompt;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

const PROTOCOL_VERSION: &str = "2025-06-18";

pub async fn mcp_post(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing bearer token" })),
        )
            .into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return Json(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))).into_response(),
    };
    match parsed {
        Value::Array(batch) => {
            let mut replies = Vec::new();
            for msg in batch {
                if let Some(reply) = handle(&state, msg).await {
                    replies.push(reply);
                }
            }
            if replies.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(Value::Array(replies)).into_response()
            }
        }
        msg => match handle(&state, msg).await {
            Some(reply) => Json(reply).into_response(),
            // A notification (e.g. notifications/initialized): accepted, no body.
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

/// No server-to-client stream is offered.
pub async fn mcp_get() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PromptQuery {
    #[serde(default)]
    mcp_only: bool,
}

/// The full investigation system prompt (current date, rules, each server's guide, the optional
/// `system_prompt` from config.toml), for the Open WebUI "LASEASK · Investigation" filter.
pub async fn system_prompt(State(state): State<AppState>, Query(q): Query<PromptQuery>) -> Json<Value> {
    let servers = served_servers(&state);
    Json(json!({
        "prompt": prompt::system_prompt(&servers, &state.mcp_notes, q.mcp_only, state.config.system_prompt.as_deref()),
        "servers": servers,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    constant_time_eq(state.config.tool_server.api_key.trim().as_bytes(), given.as_bytes())
}

/// Answers one JSON-RPC message; None for notifications, which get no reply.
async fn handle(state: &AppState, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(initialize(state, &params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": offered(state).map(tool_json).collect::<Vec<_>>() })),
        "tools/call" => call(state, &params).await,
        // Some clients ask for these even when the capability isn't announced.
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(state: &AppState, params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "laseask-tools", "version": env!("CARGO_PKG_VERSION") },
        "instructions": prompt::tool_guide(&served_servers(state), &state.mcp_notes),
    })
}

/// "fortianalyzer", "intel"...: the servers whose tools are offered, sorted.
fn served_servers(state: &AppState) -> Vec<String> {
    let mut servers: Vec<String> = offered(state)
        .map(|t| server_of(&t.qualified_name).to_string())
        .collect();
    servers.sort();
    servers.dedup();
    servers
}

fn offered(state: &AppState) -> impl Iterator<Item = &ToolSpec> {
    state
        .mcp_tools
        .iter()
        .filter(|t| state.config.tool_server.serves(server_of(&t.qualified_name)))
}

fn server_of(qualified_name: &str) -> &str {
    qualified_name.split_once("__").map_or("", |(server, _)| server)
}

fn tool_json(t: &ToolSpec) -> Value {
    json!({
        "name": t.qualified_name,
        "description": t.description,
        "inputSchema": t.input_schema,
    })
}

async fn call(state: &AppState, params: &Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing tool name".to_string()))?;
    if !offered(state).any(|t| t.qualified_name == name) {
        return Err((-32602, format!("unknown tool: {name}")));
    }
    let args = match params.get("arguments") {
        Some(Value::Object(_)) => params["arguments"].clone(),
        _ => json!({}),
    };
    let started = Instant::now();
    let (text, ok) = agent::call_mcp_tool(name, args, &state.mcp_clients, &state.config.agent).await;
    tracing::info!(
        "tool server: {name} {} in {} ms, {} chars",
        if ok { "ok" } else { "failed" },
        started.elapsed().as_millis(),
        text.chars().count()
    );
    // A failed call is a tool result with isError, not a protocol error: the model sees why.
    Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": !ok }))
}
