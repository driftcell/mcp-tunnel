use std::collections::HashMap;

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

/// 聚合客户端：管理多个上游 MCP 连接，对外暴露统一的工具列表
pub struct AggregatedClient {
    /// 上游名 -> 已连接的客户端 Peer
    clients: RwLock<HashMap<String, UpstreamClient>>,
    /// 上游服务器配置，用于运行时工具过滤
    configs: RwLock<Vec<ServerConfig>>,
}

/// 单个上游的客户端包装
struct UpstreamClient {
    /// rmcp 客户端 Peer（用于发送请求）
    peer: Peer<RoleClient>,
    /// Keep the RunningService alive so the background transport task isn't cancelled.
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

    /// 连接到所有配置的上游服务
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
                    // 继续连接其他上游，不中断
                }
            }
        }
        Ok(())
    }

    /// 连接单个上游
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

    /// 获取所有聚合后的工具列表（带前缀名称）
    /// 每次调用时动态向上游查询，不缓存
    pub async fn list_tools(&self) -> Vec<Tool> {
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
                    prefix_tool_name(upstream_name, tool.name.as_ref()).into();
                all_tools.push(prefixed_tool);
            }
        }

        all_tools
    }

    /// 调用工具（使用带前缀的名称）
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

    /// 获取所有上游名称
    pub async fn upstream_names(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }

    /// 检查是否至少有一个上游连接成功
    pub async fn has_any_client(&self) -> bool {
        let clients = self.clients.read().await;
        !clients.is_empty()
    }
}

impl Default for AggregatedClient {
    fn default() -> Self {
        Self::new()
    }
}

/// 根据配置过滤工具列表
fn apply_filter(config: &ServerConfig, tools: Vec<Tool>) -> Vec<Tool> {
    tools
        .into_iter()
        .filter(|tool| config.is_tool_enabled(&tool.name))
        .collect()
}

/// 连接到单个服务器并发现其工具列表（用于 TUI 中 OAuth 完成后的工具获取）
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
            let tools = peer
                .list_all_tools()
                .await
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
            let tools = peer
                .list_all_tools()
                .await
                .map_err(|e| AppError::Mcp(format!("list tools error: {}", e)))?;
            Ok(tools)
            // `_service` (RunningService) stays alive until here, keeping the transport open
        }
    }
}

/// 构建 reqwest Client，可选附带 Bearer token。
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

/// 通过 Streamable HTTP transport 连接到 MCP 服务。
/// 返回 (Peer, RunningService)；调用方必须持有 RunningService 以保持连接存活。
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

/// 构建 stdio 命令
fn build_command(command: &str, args: &[String]) -> Command {
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd
}

