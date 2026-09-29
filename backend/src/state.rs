use crate::agent::ToolSpec;
use crate::config::Config;
use crate::mcp::McpClient;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    /// Connected + initialized MCP clients, keyed by the name under `[mcp.<name>]` in config.toml.
    pub mcp_clients: Arc<HashMap<String, Arc<McpClient>>>,
    /// Every tool discovered across all configured MCP servers, flattened and qualified
    /// ("{server}__{tool}"), ready to hand to any tool-capable provider.
    pub mcp_tools: Arc<Vec<ToolSpec>>,
    /// Per server: its own `instructions` from the MCP handshake plus the `notes` from
    /// config.toml, for the system prompt (see prompt.rs). Servers with neither are absent.
    pub mcp_notes: Arc<HashMap<String, String>>,
}
