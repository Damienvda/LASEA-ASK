use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub default_model: String,
    /// Sampling temperature for tool-using conversations. Unset = the provider's default. Lower
    /// (e.g. 0.2-0.3) makes tool use steadier. Leave unset for OpenAI reasoning models, which
    /// reject it.
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpServerConfig {
    pub url: String,
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// If set, only these tools (plain MCP names, without the "{server}__" prefix) are offered to
    /// the model. Every tool description is sent with every request, so a short list saves a lot
    /// of context.
    #[serde(default)]
    pub include_tools: Option<Vec<String>>,
    /// Tools never offered to the model, e.g. ones that change things. Applied after
    /// `include_tools`.
    #[serde(default)]
    pub exclude_tools: Vec<String>,
}

impl McpServerConfig {
    pub fn offers(&self, tool: &str) -> bool {
        self.include_tools
            .as_ref()
            .map_or(true, |names| names.iter().any(|n| n == tool))
            && !self.exclude_tools.iter().any(|n| n == tool)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_static_dir")]
    pub static_dir: String,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8787
}

fn default_static_dir() -> String {
    "frontend".to_string()
}

/// TLS is off by default (plain HTTP, matching the firewall-restricted deployment model). Turn it
/// on once you've requested a cert with deploy/tls/request-cert.sh — see that script and the
/// README for how certs get here; this struct just points at the resulting files.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TlsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// e.g. /etc/letsencrypt/live/<hostname>/fullchain.pem
    #[serde(default)]
    pub cert_path: String,
    /// e.g. /etc/letsencrypt/live/<hostname>/privkey.pem
    #[serde(default)]
    pub key_path: String,
    /// How often to re-read the cert/key files from disk, so a certbot renewal (which replaces
    /// these files in place) gets picked up without restarting the process. Certs are renewed
    /// ~30 days before their 90-day expiry, so checking every few hours is more than enough.
    #[serde(default = "default_reload_seconds")]
    pub reload_check_seconds: u64,
}

fn default_reload_seconds() -> u64 {
    6 * 60 * 60
}

/// How hard the tool-use loops (agent.rs, agent_openai.rs) work on one question.
#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct AgentConfig {
    /// Model requests per question, tool calls included. The last one is forced to answer.
    pub max_turns: usize,
    /// When the model gives its final answer after using tools, ask it once whether anything is
    /// still open that the tools could resolve, and let it continue if so.
    pub completion_check: bool,
    /// Output token limit per model request. Only sent to Anthropic, where it's mandatory.
    pub max_output_tokens: u32,
    /// Size cap per tool result, in characters (after compaction). Log rows are mostly IPs and
    /// numbers, which cost about 1 token per 1.7 characters, so 20k is roughly 12k tokens.
    pub max_tool_result_chars: usize,
    /// Before any tool call, make the model write its investigation plan (a turn with tools
    /// disabled), then carry it out. Costs one extra request, and keeps weaker models from
    /// stopping after the first plausible result.
    pub plan_first: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 40,
            completion_check: true,
            max_output_tokens: 16_000,
            max_tool_result_chars: 20_000,
            plan_first: true,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub default_provider: String,
    #[serde(default = "default_server")]
    pub server: ServerConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    pub providers: HashMap<String, ProviderConfig>,
    /// Optional MCP servers whose tools get made available to tool-capable providers (see
    /// backend/src/agent.rs and agent_openai.rs). Each key becomes the tool name prefix, e.g. a
    /// "fortianalyzer" entry with a "get_alerts" tool is exposed as "fortianalyzer__get_alerts".
    #[serde(default)]
    pub mcp: HashMap<String, McpServerConfig>,
    /// Optional extra instructions appended to the built-in system prompt (see prompt.rs).
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub agent: AgentConfig,
}

fn default_server() -> ServerConfig {
    ServerConfig {
        host: default_host(),
        port: default_port(),
        static_dir: default_static_dir(),
    }
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            anyhow::anyhow!(
                "could not read config file at {:?}: {e}. Copy config.example.toml to config.toml and fill in your API keys.",
                path.as_ref()
            )
        })?;
        let cfg: Config = toml::from_str(&raw)?;
        if !cfg.providers.contains_key(&cfg.default_provider) {
            anyhow::bail!(
                "default_provider '{}' has no matching [providers.{}] section",
                cfg.default_provider,
                cfg.default_provider
            );
        }
        if cfg.agent.max_turns < 2 {
            anyhow::bail!("[agent] max_turns must be at least 2");
        }
        if cfg.tls.enabled &&(cfg.tls.cert_path.is_empty() || cfg.tls.key_path.is_empty()) {
            anyhow::bail!("[tls] enabled = true requires both cert_path and key_path to be set");
        }
        Ok(cfg)
    }
}
