use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub default_model: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpServerConfig {
    pub url: String,
    #[serde(default)]
    pub bearer_token: Option<String>,
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
        if cfg.tls.enabled && (cfg.tls.cert_path.is_empty() || cfg.tls.key_path.is_empty()) {
            anyhow::bail!("[tls] enabled = true requires both cert_path and key_path to be set");
        }
        Ok(cfg)
    }
}
