use crate::agent::{self, ToolSpec};
use crate::agent_openai;
use crate::error::AppError;
use crate::mcp::McpClient;
use crate::providers::{provider_by_name, ChatMessage, StreamEvent};
use crate::state::AppState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    /// When true, forces the model to consult an MCP tool before answering (a forced
    /// tool_choice on the first turn) instead of possibly answering from its own knowledge.
    /// Requires a tool-capable provider (anthropic, openai, mistral) and at least one MCP tool.
    #[serde(default)]
    pub mcp_only: bool,
    /// Which `[mcp.<name>]` servers' tools to offer the model for this request (the UI's MCP
    /// checkboxes). Omitted = every connected server, so older clients keep working; an empty
    /// list = no tools at all, i.e. a plain chat.
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub default_model: String,
}

#[derive(Debug, Serialize)]
pub struct McpServerInfo {
    pub name: String,
    /// False when the server is in config.toml but failed to connect at start-up.
    pub connected: bool,
    pub tool_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ProvidersResponse {
    pub default_provider: String,
    pub providers: Vec<ProviderInfo>,
    /// Qualified MCP tool names currently available to tool-capable providers, purely
    /// informational (e.g. so the UI can show a "tools connected" hint).
    pub mcp_tools: Vec<String>,
    /// Every configured MCP server, connected or not, so the UI can offer one checkbox each.
    pub mcp_servers: Vec<McpServerInfo>,
}

pub async fn list_providers(State(state): State<AppState>) -> Json<ProvidersResponse> {
    // Sorted: config.providers is a HashMap, and the UI shows these as buttons that shouldn't
    // shuffle around between page loads.
    let mut providers: Vec<ProviderInfo> = state
        .config
        .providers
        .iter()
        .map(|(name, cfg)| ProviderInfo {
            name: name.clone(),
            default_model: cfg.default_model.clone(),
        })
        .collect();
    providers.sort_by(|a, b| a.name.cmp(&b.name));

    let mut mcp_servers: Vec<McpServerInfo> = state
        .config
        .mcp
        .keys()
        .map(|name| McpServerInfo {
            name: name.clone(),
            connected: state.mcp_clients.contains_key(name),
            tool_count: state
                .mcp_tools
                .iter()
                .filter(|t| tool_server(&t.qualified_name) == name)
                .count(),
        })
        .collect();
    mcp_servers.sort_by(|a, b| a.name.cmp(&b.name));

    Json(ProvidersResponse {
        default_provider: state.config.default_provider.clone(),
        providers,
        mcp_tools: state.mcp_tools.iter().map(|t| t.qualified_name.clone()).collect(),
        mcp_servers,
    })
}

/// The `[mcp.<name>]` a qualified "{server}__{tool}" name belongs to.
fn tool_server(qualified_name: &str) -> &str {
    qualified_name.split_once("__").map_or("", |(server, _)| server)
}

pub async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<BoxStream<'static, Result<Event, Infallible>>>, AppError> {
    if req.messages.is_empty() {
        return Err(AppError::BadRequest("messages must not be empty".into()));
    }

    let provider_name = req
        .provider
        .unwrap_or_else(|| state.config.default_provider.clone());

    let provider_cfg = state
        .config
        .providers
        .get(&provider_name)
        .ok_or_else(|| AppError::UnknownProvider(provider_name.clone()))?
        .clone();

    let model = req.model.unwrap_or_else(|| provider_cfg.default_model.clone());
    let mcp_only = req.mcp_only;

    // Narrow the tools (and the clients allowed to run them) to the servers ticked in the UI.
    let selected = |server: &str| {
        req.mcp_servers
            .as_ref()
            .map_or(true, |names| names.iter().any(|n| n == server))
    };
    let tools: Vec<ToolSpec> = state
        .mcp_tools
        .iter()
        .filter(|t| selected(tool_server(&t.qualified_name)))
        .cloned()
        .collect();
    let mcp_clients: HashMap<String, Arc<McpClient>> = state
        .mcp_clients
        .iter()
        .filter(|(name, _)| selected(name.as_str()))
        .map(|(name, client)| (name.clone(), client.clone()))
        .collect();

    // At least one MCP tool selected -> route through the tool-use loop matching the provider's
    // wire format: agent.rs for Anthropic, agent_openai.rs for OpenAI-compatible APIs (OpenAI,
    // Mistral). Every other case is the plain single-shot provider stream.
    let has_tools = !tools.is_empty();
    let openai_endpoint = agent_openai::endpoint_for(&provider_name);

    let event_stream: BoxStream<'static, StreamEvent> =
        if has_tools && (provider_name == "anthropic" || openai_endpoint.is_some()) {
            let (tx, rx) = mpsc::channel::<StreamEvent>(32);
            let api_key = provider_cfg.api_key.clone();
            let messages = req.messages.clone();

            tokio::spawn(async move {
                match openai_endpoint {
                    Some(endpoint) => {
                        agent_openai::run(
                            endpoint, &api_key, &model, &messages, &tools, &mcp_clients, mcp_only, tx,
                        )
                        .await
                    }
                    None => {
                        agent::run(&api_key, &model, &messages, &tools, &mcp_clients, mcp_only, tx)
                            .await
                    }
                }
            });

            receiver_stream(rx).boxed()
        } else if mcp_only {
            return Err(AppError::BadRequest(
                "mcp_only requires at least one selected MCP server with tools and a tool-capable provider (anthropic, openai or mistral)"
                    .into(),
            ));
        } else {
            let provider = provider_by_name(&provider_name)
                .ok_or_else(|| AppError::UnknownProvider(provider_name.clone()))?;

            provider
                .stream_chat(&provider_cfg.api_key, &model, &req.messages)
                .await
                .map_err(|e| AppError::Provider(e.to_string()))?
        };

    let sse_stream = event_stream
        .map(|ev| {
            let json = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
            Ok(Event::default().data(json))
        })
        .boxed();

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}

fn receiver_stream(rx: mpsc::Receiver<StreamEvent>) -> impl futures::Stream<Item = StreamEvent> {
    futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) })
}
