use serde::{Deserialize, Serialize};
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
    /// Anthropic only: how much the model thinks and works per step, "low" | "medium" | "high" |
    /// "xhigh" | "max". Unset = the model's default (only `medium` on Opus 5.5). Investigations
    /// want "high" or more.
    #[serde(default)]
    pub effort: Option<String>,
    /// Anthropic only: ask for a readable summary of the model's thinking, shown folded in the
    /// UI. Off = thinking still happens but streams as empty text.
    #[serde(default = "default_true")]
    pub show_thinking: bool,
}

fn default_true() -> bool {
    true
}

/// Prices of one model in US dollars per million tokens, for the cost estimate under each reply.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    /// Tokens read from the prompt cache. Unset = the input price.
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Tokens written to the prompt cache (Anthropic). Unset = the input price.
    #[serde(default)]
    pub cache_write: Option<f64>,
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
    /// Your own notes on using this server (what fields mean, known quirks), given to the model
    /// whenever the server is ticked, after the server's own instructions.
    #[serde(default)]
    pub notes: Option<String>,
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
    /// Output token limit per model request. Only sent to Anthropic, where it's mandatory. On
    /// models that think, the thinking counts towards it; you only pay for what's generated.
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
            max_output_tokens: 64_000,
            max_tool_result_chars: 20_000,
            plan_first: true,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Only needed for LASEASK's own chat (its console, /api/chat, /v1). A tool server for Open
    /// WebUI, where the models are set up in Open WebUI, needs no provider at all.
    #[serde(default)]
    pub default_provider: String,
    #[serde(default = "default_server")]
    pub server: ServerConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
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
    /// Per model name, for the UI's cost estimate. Models without prices only show token counts.
    #[serde(default)]
    pub pricing: HashMap<String, ModelPricing>,
    #[serde(default)]
    pub intel: IntelConfig,
    /// The OpenAI-compatible API (/v1) through which Open WebUI uses LASEASK (openai_compat.rs).
    #[serde(default)]
    pub openwebui: OpenWebUiConfig,
    /// LASEASK's tools as an MCP server that Open WebUI connects to (tool_server.rs).
    #[serde(default)]
    pub tool_server: ToolServerConfig,
}

/// Open WebUI is the tool; LASEASK serves it the MCP tools (FortiAnalyzer through `[mcp.*]`,
/// with include/exclude_tools and result compaction, plus "intel") on POST /mcp.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct ToolServerConfig {
    pub enabled: bool,
    /// Bearer token Open WebUI sends (its MCP connection's auth). At least 16 characters.
    pub api_key: String,
    /// Which `[mcp.<name>]` servers (and "intel") to serve. Unset = all of them.
    pub servers: Option<Vec<String>>,
}

impl ToolServerConfig {
    pub fn serves(&self, server: &str) -> bool {
        self.servers.as_ref().map_or(true, |s| s.iter().any(|n| n == server))
    }
}

/// Open WebUI in front, LASEASK as its engine: Open WebUI sees each entry of `models` as a model
/// it can chat with, and every request runs through the same agent, tools and prompt as the
/// LASEASK UI.
#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct OpenWebUiConfig {
    pub enabled: bool,
    /// Shared secret Open WebUI sends as "Authorization: Bearer <api_key>" (its OPENAI_API_KEYS).
    /// Required when enabled; at least 16 characters.
    pub api_key: String,
    /// false (default): while tools are in use, the text the model writes between tool calls goes
    /// to Open WebUI's folded "Thought" block and only the final answer appears as the reply, once
    /// complete. true: all text streams straight into the reply.
    pub live_answer: bool,
    /// Language of the tool timeline and summary shown in the "Thought" block: "fr" or "en".
    pub language: String,
    /// The models Open WebUI can pick. Empty = "laseask" (all tools) and "laseask-chat" (no tools).
    pub models: Vec<OpenWebUiModel>,
    /// Also offer one model per `[providers.*]` ("laseask-anthropic", "laseask-mistral"...), with
    /// all tools and the provider's default_model. Any "<provider>/<model>" id (e.g.
    /// "anthropic/claude-sonnet-5-5") is accepted too, like the LASEASK UI's free model field:
    /// add such ids in Open WebUI's connection settings to see them in its model list.
    pub provider_models: bool,
}

impl Default for OpenWebUiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            live_answer: false,
            language: "fr".to_string(),
            models: Vec::new(),
            provider_models: true,
        }
    }
}

impl OpenWebUiConfig {
    pub fn model_list(&self) -> Vec<OpenWebUiModel> {
        if !self.models.is_empty() {
            return self.models.clone();
        }
        vec![
            OpenWebUiModel {
                id: "laseask".into(),
                name: Some("LASEASK".into()),
                provider: None,
                model: None,
                tools: true,
                mcp_servers: None,
                mcp_only: false,
            },
            OpenWebUiModel {
                id: "laseask-chat".into(),
                name: Some("LASEASK (sans outils)".into()),
                provider: None,
                model: None,
                tools: false,
                mcp_servers: None,
                mcp_only: false,
            },
        ]
    }

    /// Everything /v1/models lists: the configured models, then one per provider (unless an id
    /// is already taken or provider_models = false), in a stable order.
    pub fn all_models(&self, providers: &HashMap<String, ProviderConfig>) -> Vec<OpenWebUiModel> {
        let mut all = self.model_list();
        if self.provider_models {
            let mut names: Vec<&String> = providers.keys().collect();
            names.sort();
            for name in names {
                let id = format!("laseask-{name}");
                if all.iter().any(|m| m.id == id) {
                    continue;
                }
                let cfg = &providers[name];
                all.push(OpenWebUiModel {
                    id,
                    name: Some(format!("LASEASK · {name} · {}", cfg.default_model)),
                    provider: Some(name.clone()),
                    model: None,
                    tools: true,
                    mcp_servers: None,
                    mcp_only: false,
                });
            }
        }
        all
    }

    /// The model behind an id Open WebUI sends: a listed one, or "<provider>/<model>" (also with
    /// a "laseask/" prefix) for any model of a configured provider, with all tools.
    pub fn resolve(&self, providers: &HashMap<String, ProviderConfig>, id: &str) -> Option<OpenWebUiModel> {
        if let Some(m) = self.all_models(providers).into_iter().find(|m| m.id == id) {
            return Some(m);
        }
        if !self.provider_models {
            return None;
        }
        let rest = id.strip_prefix("laseask/").unwrap_or(id);
        let (provider, model) = rest.split_once('/')?;
        if model.trim().is_empty() || !providers.contains_key(provider) {
            return None;
        }
        Some(OpenWebUiModel {
            id: id.to_string(),
            name: None,
            provider: Some(provider.to_string()),
            model: Some(model.trim().to_string()),
            tools: true,
            mcp_servers: None,
            mcp_only: false,
        })
    }
}

/// One model as Open WebUI sees it: a provider/model pair plus which tools it gets.
#[derive(Debug, Deserialize, Clone)]
pub struct OpenWebUiModel {
    /// The model id Open WebUI shows and sends back, e.g. "laseask".
    pub id: String,
    /// Display name. Unset = the id.
    #[serde(default)]
    pub name: Option<String>,
    /// A `[providers.<name>]` key. Unset = default_provider.
    #[serde(default)]
    pub provider: Option<String>,
    /// Unset = the provider's default_model.
    #[serde(default)]
    pub model: Option<String>,
    /// false = plain chat, no tools (e.g. for Open WebUI's titles, tags and follow-up questions).
    #[serde(default = "default_true")]
    pub tools: bool,
    /// Which `[mcp.<name>]` servers (and "intel") to offer. Unset = all of them.
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
    /// Force a tool call before answering (the LASEASK UI's "MCP only").
    #[serde(default)]
    pub mcp_only: bool,
}

impl OpenWebUiModel {
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }
}

/// External threat intelligence on public IPs and domains (see intel.rs). RDAP needs no key; the
/// other sources are used when their key is set.
#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct IntelConfig {
    pub enabled: bool,
    /// Owner, network and country of IPs, registration date of domains. Free, no key.
    pub rdap: bool,
    pub abuseipdb_api_key: Option<String>,
    pub virustotal_api_key: Option<String>,
    /// VirusTotal's free API allows 4 lookups a minute; beyond that they're skipped, not queued.
    pub virustotal_per_minute: usize,
    pub otx_api_key: Option<String>,
    /// Your own domains (e.g. "example.local"), never sent out. .local, .lan, .internal, .corp,
    /// .home and .arpa are always treated as internal, as are private IP ranges.
    pub internal_domains: Vec<String>,
    pub cache_hours: u64,
    pub timeout_seconds: u64,
}

impl Default for IntelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rdap: true,
            abuseipdb_api_key: None,
            virustotal_api_key: None,
            virustotal_per_minute: 4,
            otx_api_key: None,
            internal_domains: Vec::new(),
            cache_hours: 24,
            timeout_seconds: 10,
        }
    }
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
        let chat_configured = !cfg.default_provider.is_empty() || !cfg.providers.is_empty();
        if chat_configured && !cfg.providers.contains_key(&cfg.default_provider) {
            anyhow::bail!(
                "default_provider '{}' has no matching [providers.{}] section",
                cfg.default_provider,
                cfg.default_provider
            );
        }
        if !chat_configured && !cfg.tool_server.enabled {
            anyhow::bail!(
                "nothing to do: set default_provider and a [providers.*] section (LASEASK chat), or [tool_server] enabled = true (tools for Open WebUI)"
            );
        }
        if cfg.openwebui.enabled && !chat_configured {
            anyhow::bail!("[openwebui] enabled = true needs default_provider and [providers.*]");
        }
        if cfg.tool_server.enabled && cfg.tool_server.api_key.trim().len() < 16 {
            anyhow::bail!("[tool_server] enabled = true requires an api_key of at least 16 characters");
        }
        if cfg.agent.max_turns < 2 {
            anyhow::bail!("[agent] max_turns must be at least 2");
        }
        if cfg.tls.enabled &&(cfg.tls.cert_path.is_empty() || cfg.tls.key_path.is_empty()) {
            anyhow::bail!("[tls] enabled = true requires both cert_path and key_path to be set");
        }
        if cfg.openwebui.enabled {
            if cfg.openwebui.api_key.trim().len() < 16 {
                anyhow::bail!("[openwebui] enabled = true requires an api_key of at least 16 characters");
            }
            let models = cfg.openwebui.model_list();
            for (i, m) in models.iter().enumerate() {
                if m.id.trim().is_empty() {
                    anyhow::bail!("[[openwebui.models]] entry {} has an empty id", i + 1);
                }
                if models[..i].iter().any(|other| other.id == m.id) {
                    anyhow::bail!("[[openwebui.models]] id '{}' is used twice", m.id);
                }
                if let Some(p) = &m.provider {
                    if !cfg.providers.contains_key(p) {
                        anyhow::bail!("[[openwebui.models]] '{}' uses provider '{p}', which has no [providers.{p}] section", m.id);
                    }
                }
            }
        }
        Ok(cfg)
    }
}
