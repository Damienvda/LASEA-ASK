mod agent;
mod agent_openai;
mod config;
mod error;
mod mcp;
mod providers;
mod routes;
mod state;

use agent::ToolSpec;
use axum::routing::{get, post};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use config::Config;
use mcp::McpClient;
use state::AppState;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "laseask_backend=info,tower_http=info".into()),
        )
        .init();

    let config_path = std::env::var("LASEASK_CONFIG").unwrap_or_else(|_| "config.toml".to_string());
    let config = Config::load(&config_path)?;
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    let static_dir = config.server.static_dir.clone();
    let tls_config = config.tls.clone();

    let (mcp_clients, mcp_tools) = connect_mcp_servers(&config).await;

    let state = AppState {
        config: Arc::new(config),
        mcp_clients: Arc::new(mcp_clients),
        mcp_tools: Arc::new(mcp_tools),
    };

    let app = Router::new()
        .route("/api/providers", get(routes::list_providers))
        .route("/api/chat", post(routes::chat))
        .with_state(state)
        .fallback_service(ServeDir::new(&static_dir))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if tls_config.enabled {
        serve_tls(addr, app, tls_config, static_dir).await?;
    } else {
        tracing::info!("LASEASK backend listening on http://{addr} (static: {static_dir})");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}

async fn serve_tls(
    addr: SocketAddr,
    app: Router,
    tls: config::TlsConfig,
    static_dir: String,
) -> anyhow::Result<()> {
    let rustls_config = RustlsConfig::from_pem_file(&tls.cert_path, &tls.key_path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to load TLS cert/key ({} / {}): {e}. Run deploy/tls/request-cert.sh first.",
                tls.cert_path,
                tls.key_path
            )
        })?;

    // Re-read the cert/key files periodically so a certbot renewal (which replaces them in
    // place) is picked up without restarting the process.
    let reload_config = rustls_config.clone();
    let cert_path = tls.cert_path.clone();
    let key_path = tls.key_path.clone();
    let interval = tls.reload_check_seconds;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval.max(60)));
        loop {
            ticker.tick().await;
            if let Err(e) = reload_config.reload_from_pem_file(&cert_path, &key_path).await {
                tracing::warn!("TLS cert reload failed (keeping previous cert): {e}");
            } else {
                tracing::info!("TLS cert reloaded from disk");
            }
        }
    });

    tracing::info!("LASEASK backend listening on https://{addr} (static: {static_dir})");
    axum_server::bind_rustls(addr, rustls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

/// Connects to every `[mcp.<name>]` server, discovers its tools, and flattens them into a single
/// qualified tool list. A server that fails to connect is logged and skipped rather than treated
/// as fatal — plain chat should still work even if an MCP server is temporarily down.
async fn connect_mcp_servers(
    config: &Config,
) -> (HashMap<String, Arc<McpClient>>, Vec<ToolSpec>) {
    let mut clients = HashMap::new();
    let mut tools = Vec::new();

    for (name, cfg) in &config.mcp {
        let client = Arc::new(McpClient::new(cfg.url.clone(), cfg.bearer_token.clone()));
        match client.initialize().await {
            Ok(()) => match client.list_tools().await {
                Ok(discovered) => {
                    tracing::info!("MCP '{name}': connected, {} tool(s) available", discovered.len());
                    for tool in discovered {
                        tools.push(ToolSpec {
                            qualified_name: format!("{name}__{}", tool.name),
                            description: tool.description,
                            input_schema: tool.input_schema,
                        });
                    }
                    clients.insert(name.clone(), client);
                }
                Err(e) => tracing::warn!("MCP '{name}': connected but tools/list failed: {e}"),
            },
            Err(e) => tracing::warn!("MCP '{name}': failed to initialize, skipping: {e}"),
        }
    }

    (clients, tools)
}
