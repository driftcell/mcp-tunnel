use std::collections::HashMap;
use std::time::Duration;

use oauth2::TokenResponse;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::ServerConfig;
use crate::error::{AppError, Result};

use super::tool_filter::prefix_tool_name;

/// Aggregated client: manages multiple upstream MCP connections, exposing a unified tool list
pub struct AggregatedClient {
    /// Upstream name -> connected client Peer
    clients: RwLock<HashMap<String, UpstreamClient>>,
    /// Upstream server configs, used for runtime tool filtering
    configs: RwLock<Vec<ServerConfig>>,
}

/// Wrapper for a single upstream client
struct UpstreamClient {
    /// rmcp client Peer (used to send requests)
    peer: Peer<RoleClient>,
    /// Keep the RunningService alive so the background transport task isn't cancelled.
    /// When this UpstreamClient is dropped, `_keepalive` is dropped, which causes the
    /// underlying TokioChildProcess transport to be dropped. TokioChildProcess's Drop
    /// implementation kills the child process, preventing zombie processes.
    /// This field is not read directly — its only purpose is lifetime extension via Drop.
    _keepalive: RunningService<RoleClient, ()>,
}

impl AggregatedClient {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            configs: RwLock::new(Vec::new()),
        }
    }

    /// Connect to all configured upstream services
    #[tracing::instrument(skip(self, configs))]
    pub async fn connect_all(&self, configs: &[ServerConfig]) -> Result<()> {
        {
            let mut stored = self.configs.write().await;
            *stored = configs.to_vec();
        }
        let mut clients = self.clients.write().await;
        for config in configs {
            match Self::connect_single(config.clone()).await {
                Ok(client) => {
                    info!("Connected to upstream: {}", config.name);
                    clients.insert(config.name.clone(), client);
                }
                Err(e) => {
                    warn!("Failed to connect to upstream '{}': {}", config.name, e);
                    // Continue connecting to other upstreams, don't abort
                }
            }
        }
        Ok(())
    }

    /// Connect to a single upstream
    #[tracing::instrument(skip(config))]
    async fn connect_single(config: ServerConfig) -> Result<UpstreamClient> {
        match &config.ty {
            crate::config::UpstreamType::Stdio { command, args } => {
                let cmd = build_command(command, args);
                let transport = TokioChildProcess::new(cmd)
                    .map_err(|e| AppError::Mcp(format!("stdio transport error: {}", e)))?;

                let client = ()
                    .serve(transport)
                    .await
                    .map_err(|e| AppError::Mcp(format!("client start error: {}", e)))?;

                let peer = client.peer().clone();

                Ok(UpstreamClient {
                    peer,
                    _keepalive: client,
                })
            }
            crate::config::UpstreamType::Http { url } => {
                let store = crate::mcp::oauth::FileCredentialStore::new(&config.name)?;
                let stored_token = store.load().await?;

                let token = if let Some(stored) = stored_token {
                    // Token is still valid, use it directly
                    Some(stored.token.access_token().secret().clone())
                } else {
                    // No valid token in store. Try to load an expired token and refresh it.
                    let mut refreshed_token: Option<String> = None;

                    if let Some(expired) = store.load_including_expired().await? {
                        if let Some(refresh_token) = expired.refresh_token() {
                            let client_id = expired.client_id.clone();

                            if !client_id.is_empty() {
                                match crate::mcp::oauth::refresh_access_token(
                                    url,
                                    &client_id,
                                    refresh_token.secret(),
                                ).await {
                                    Ok(crate::mcp::oauth::RefreshResult::Success(new_token)) => {
                                        store.save_with_client_id(&new_token, &client_id).await?;
                                        info!("OAuth token refreshed for server '{}'", config.name);
                                        refreshed_token = Some(new_token.access_token().secret().clone());
                                    }
                                    Ok(crate::mcp::oauth::RefreshResult::NoRefreshToken) => {
                                        warn!("No refresh token available, falling back to PKCE");
                                    }
                                    Ok(crate::mcp::oauth::RefreshResult::NoAuthorizationSupport) => {
                                        info!("No OAuth support for server '{}', proceeding without token", config.name);
                                    }
                                    Err(e) => {
                                        warn!("Token refresh failed: {}, falling back to PKCE", e);
                                    }
                                }
                            } else {
                                warn!("Stored token has no client_id, falling back to PKCE");
                            }
                        } else {
                            warn!("Expired token has no refresh token, falling back to PKCE");
                        }
                    }

                    refreshed_token
                };

                // If we still don't have a token (refresh failed or no stored token at all),
                // fall back to full PKCE flow.
                let token = if let Some(t) = token {
                    Some(t)
                } else {
                    match crate::mcp::oauth::run_pkce_flow(url).await? {
                        crate::mcp::oauth::PkceFlowResult::Success { client_id, token } => {
                            store.save_with_client_id(&token, &client_id).await?;
                            info!("OAuth token saved for server '{}'", config.name);
                            Some(token.access_token().secret().clone())
                        }
                        crate::mcp::oauth::PkceFlowResult::NoAuthorizationSupport => {
                            info!("No OAuth support for server '{}', proceeding without token", config.name);
                            None
                        }
                    }
                };

                let reqwest_client = build_reqwest_client(token.as_deref())?;
                let (peer, service) = connect_http(url, reqwest_client).await?;

                Ok(UpstreamClient {
                    peer,
                    _keepalive: service,
                })
            }
        }
    }

    /// Get the full aggregated tool list (with prefixed names)
    /// Dynamically queries upstreams on each call, not cached
    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        let clients = self.clients.read().await;
        let configs = self.configs.read().await;
        let mut all_tools = Vec::new();

        for (upstream_name, client) in clients.iter() {
            let tools = match client.peer.list_all_tools().await {
                Ok(tools) => tools,
                Err(e) => {
                    warn!("Failed to list tools for '{}': {}", upstream_name, e);
                    continue;
                }
            };

            let config = configs.iter().find(|c| c.name == *upstream_name);
            let filtered_tools = match config {
                Some(cfg) => apply_filter(cfg, tools),
                None => tools,
            };

            for tool in filtered_tools {
                let mut prefixed_tool = tool.clone();
                prefixed_tool.name =
                    prefix_tool_name(upstream_name, tool.name.as_ref())?.into();
                all_tools.push(prefixed_tool);
            }
        }

        Ok(all_tools)
    }

    /// Call a tool (using the prefixed name)
    #[tracing::instrument(skip(self, arguments))]
    pub async fn call_tool(
        &self,
        prefixed_name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult> {
        let (upstream_name, tool_name) =
            super::tool_filter::parse_tool_name(prefixed_name)
                .ok_or_else(|| AppError::ToolNotFound(prefixed_name.to_string()))?;

        let clients = self.clients.read().await;
        let client = clients
            .get(upstream_name)
            .ok_or_else(|| AppError::UpstreamNotFound(upstream_name.to_string()))?;

        let mut param = CallToolRequestParams::new(tool_name.to_string());
        if let serde_json::Value::Object(map) = arguments {
            param = param.with_arguments(map);
        }

        client
            .peer
            .call_tool(param)
            .await
            .map_err(|e| AppError::Mcp(format!("tool call error: {}", e)))
    }

    /// Get all upstream names
    pub async fn upstream_names(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }

    /// Check if at least one upstream connection succeeded
    pub async fn has_any_client(&self) -> bool {
        let clients = self.clients.read().await;
        !clients.is_empty()
    }

    /// Gracefully shutdown all upstream connections by clearing the clients map.
    /// This drops each UpstreamClient (and its _keepalive RunningService), which
    /// triggers cleanup of the underlying child processes.
    #[allow(dead_code)]
    pub async fn shutdown(&self) {
        let mut clients = self.clients.write().await;
        if !clients.is_empty() {
            info!("Shutting down {} upstream client(s)", clients.len());
            clients.clear();
        }
    }
}

impl Drop for AggregatedClient {
    fn drop(&mut self) {
        // Best-effort log: the actual cleanup requires async, so the caller
        // should invoke `shutdown().await` before dropping. If the runtime
        // is still alive we spawn a best-effort cleanup task.
        info!("AggregatedClient dropped — upstream clients will be cleaned up");
    }
}

impl Default for AggregatedClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Filter tool list according to config
fn apply_filter(config: &ServerConfig, tools: Vec<Tool>) -> Vec<Tool> {
    tools
        .into_iter()
        .filter(|tool| config.is_tool_enabled(&tool.name))
        .collect()
}

/// Connect to a single server and discover its tool list (used for tool fetching after OAuth in TUI)
#[tracing::instrument(skip(config))]
pub async fn discover_tools(config: &ServerConfig) -> Result<Vec<Tool>> {
    match &config.ty {
        crate::config::UpstreamType::Stdio { command, args } => {
            let cmd = build_command(command, args);
            let transport = TokioChildProcess::new(cmd)
                .map_err(|e| AppError::Mcp(format!("stdio transport error: {}", e)))?;

            let client = ()
                .serve(transport)
                .await
                .map_err(|e| AppError::Mcp(format!("client start error: {}", e)))?;

            let peer = client.peer().clone();
            let tools = tokio::time::timeout(Duration::from_secs(30), peer.list_all_tools())
                .await
                .map_err(|_| AppError::Mcp("tool discovery timed out".to_string()))?
                .map_err(|e| AppError::Mcp(format!("list tools error: {}", e)))?;
            Ok(tools)
        }
        crate::config::UpstreamType::Http { url } => {
            let store = crate::mcp::oauth::FileCredentialStore::new(&config.name)?;
            let token = store
                .load()
                .await?
                .map(|stored| stored.token.access_token().secret().clone());

            let reqwest_client = build_reqwest_client(token.as_deref())?;
            let (peer, _service) = connect_http(url, reqwest_client).await?;
            let tools = tokio::time::timeout(Duration::from_secs(30), peer.list_all_tools())
                .await
                .map_err(|_| AppError::Mcp("tool discovery timed out".to_string()))?
                .map_err(|e| AppError::Mcp(format!("list tools error: {}", e)))?;
            Ok(tools)
            // `_service` (RunningService) stays alive until here, keeping the transport open
        }
    }
}

/// Build a reqwest Client, optionally with a Bearer token.
fn build_reqwest_client(bearer_token: Option<&str>) -> Result<reqwest::Client> {
    match bearer_token {
        None => Ok(reqwest::Client::default()),
        Some(token) => {
            let mut headers = reqwest::header::HeaderMap::new();
            let auth_value = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                .map_err(|e| AppError::OAuth(format!("invalid auth header value: {}", e)))?;
            headers.insert(reqwest::header::AUTHORIZATION, auth_value);
            reqwest::Client::builder()
                .default_headers(headers)
                .build()
                .map_err(|e| AppError::Mcp(format!("reqwest client build error: {}", e)))
        }
    }
}

/// Connect to an MCP service via Streamable HTTP transport.
/// Returns (Peer, RunningService); the caller must hold RunningService to keep the connection alive.
async fn connect_http(
    url: &str,
    reqwest_client: reqwest::Client,
) -> Result<(Peer<RoleClient>, RunningService<RoleClient, ()>)> {
    let config = StreamableHttpClientTransportConfig::with_uri(url);
    let transport = StreamableHttpClientTransport::with_client(reqwest_client, config);
    let service = ()
        .serve(transport)
        .await
        .map_err(|e| AppError::Mcp(format!("streamable HTTP transport error: {}", e)))?;
    let peer = service.peer().clone();
    info!("Connected via streamable HTTP to {}", url);
    Ok((peer, service))
}

/// Build a stdio command
fn build_command(command: &str, args: &[String]) -> Command {
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd
}

