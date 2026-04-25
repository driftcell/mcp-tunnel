use std::collections::HashMap;
use std::sync::Arc;

use oauth2::TokenResponse;
use rmcp::model::{CallToolRequestParam, CallToolResult, Tool};
use rmcp::service::{Peer, RoleClient};
use rmcp::transport::auth::{AuthError, OAuthState};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::sse_client::{SseClientConfig, SseClientTransport};
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
}

/// 单个上游的客户端包装
struct UpstreamClient {
    /// rmcp 客户端 Peer（用于发送请求）
    peer: Peer<RoleClient>,
    /// 缓存的工具列表（原始名称）
    tools: Vec<Tool>,
}

impl AggregatedClient {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// 连接到所有配置的上游服务
    pub async fn connect_all(&self, configs: &[ServerConfig]) -> Result<()> {
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

                // 获取工具列表
                let tools = match peer.list_all_tools().await {
                    Ok(tools) => tools,
                    Err(e) => {
                        warn!("Failed to list tools for '{}': {}", config.name, e);
                        Vec::new()
                    }
                };

                let filtered_tools = apply_filter(&config, tools);

                Ok(UpstreamClient {
                    peer,
                    tools: filtered_tools,
                })
            }
            crate::config::UpstreamType::Http { url } => {
                let store = crate::mcp::oauth::FileCredentialStore::new(&config.name);
                let stored_token = store.load().await?;

                let token = if let Some(token) = stored_token {
                    // Check if token is expired
                    let is_expired = match token.expires_in() {
                        Some(_expires_in) => {
                            // The expires_in() returns a Duration from when the token was issued.
                            // Since we don't know when it was issued, we check if the token
                            // response has an expires_in field - if it does, we assume it might be expired.
                            // For a more robust check, we'd need to store the issue time.
                            // For now, if expires_in is present and short (e.g., < 60s remaining),
                            // we consider it expired. But we don't have issue time stored.
                            //
                            // Simpler approach: always try to use the token first. If it fails,
                            // the server will return 401 and we can re-auth.
                            // For now, just use the token if we have it.
                            false
                        }
                        None => false,
                    };

                    if !is_expired {
                        Some(token.access_token().secret().clone())
                    } else {
                        // Token expired - run PKCE flow
                        let new_token = crate::mcp::oauth::run_pkce_flow(url).await?;
                        store.save(&new_token).await?;
                        info!("OAuth token saved for server '{}'", config.name);
                        Some(new_token.access_token().secret().clone())
                    }
                } else {
                    // No stored token - check if server supports OAuth
                    let mut state = OAuthState::new(url, None)
                        .await
                        .map_err(|e| AppError::OAuth(e.to_string()))?;

                    let has_oauth = match state.start_authorization(&[], "http://127.0.0.1:9876/callback").await {
                        Ok(()) => true,
                        Err(AuthError::NoAuthorizationSupport) => false,
                        Err(e) => return Err(AppError::OAuth(e.to_string())),
                    };

                    if has_oauth {
                        // Run PKCE flow
                        let new_token = crate::mcp::oauth::run_pkce_flow(url).await?;
                        store.save(&new_token).await?;
                        info!("OAuth token saved for server '{}'", config.name);
                        Some(new_token.access_token().secret().clone())
                    } else {
                        None
                    }
                };

                // Build reqwest client (with or without auth header)
                let reqwest_client = if let Some(token) = token {
                    reqwest::Client::builder()
                        .default_headers({
                            let mut headers = reqwest::header::HeaderMap::new();
                            let auth_value = reqwest::header::HeaderValue::from_str(
                                &format!("Bearer {}", token),
                            )
                            .map_err(|e| {
                                AppError::OAuth(format!("invalid auth header value: {}", e))
                            })?;
                            headers.insert(reqwest::header::AUTHORIZATION, auth_value);
                            headers
                        })
                        .build()
                        .map_err(|e| AppError::Mcp(format!("reqwest client build error: {}", e)))?
                } else {
                    reqwest::Client::default()
                };

                let peer = connect_http(url, reqwest_client).await?;

                let tools = match peer.list_all_tools().await {
                    Ok(tools) => tools,
                    Err(e) => {
                        warn!("Failed to list tools for '{}': {}", config.name, e);
                        Vec::new()
                    }
                };

                let filtered_tools = apply_filter(&config, tools);

                Ok(UpstreamClient {
                    peer,
                    tools: filtered_tools,
                })
            }
        }
    }

    /// 获取所有聚合后的工具列表（带前缀名称）
    pub async fn list_tools(&self) -> Vec<Tool> {
        let clients = self.clients.read().await;
        let mut all_tools = Vec::new();

        for (upstream_name, client) in clients.iter() {
            for tool in &client.tools {
                let mut prefixed_tool = tool.clone();
                prefixed_tool.name =
                    prefix_tool_name(upstream_name, tool.name.as_ref()).into();
                all_tools.push(prefixed_tool);
            }
        }

        all_tools
    }

    /// 调用工具（使用带前缀的名称）
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

        let param = CallToolRequestParam {
            name: tool_name.to_string().into(),
            arguments: match arguments {
                serde_json::Value::Object(map) => Some(map),
                _ => None,
            },
        };

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
            let store = crate::mcp::oauth::FileCredentialStore::new(&config.name);
            let stored_token = store.load().await?;

            let reqwest_client = if let Some(token) = stored_token {
                let token_str = token.access_token().secret().clone();
                reqwest::Client::builder()
                    .default_headers({
                        let mut headers = reqwest::header::HeaderMap::new();
                        let auth_value = reqwest::header::HeaderValue::from_str(
                            &format!("Bearer {}", token_str),
                        )
                        .map_err(|e| {
                            AppError::OAuth(format!("invalid auth header value: {}", e))
                        })?;
                        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
                        headers
                    })
                    .build()
                    .map_err(|e| AppError::Mcp(format!("reqwest client build error: {}", e)))?
            } else {
                reqwest::Client::default()
            };

            let peer = connect_http(url, reqwest_client).await?;
            let tools = peer
                .list_all_tools()
                .await
                .map_err(|e| AppError::Mcp(format!("list tools error: {}", e)))?;
            Ok(tools)
        }
    }
}

/// Try streamable HTTP first, fall back to SSE.
async fn connect_http(url: &str, reqwest_client: reqwest::Client) -> Result<Peer<RoleClient>> {
    // Try streamable HTTP first
    let streamable_config = StreamableHttpClientTransportConfig::with_uri(url);
    let streamable_transport =
        StreamableHttpClientTransport::with_client(reqwest_client.clone(), streamable_config);

    match ().serve(streamable_transport).await {
        Ok(client) => {
            info!("Connected via streamable HTTP to {}", url);
            return Ok(client.peer().clone());
        }
        Err(e) => {
            warn!(
                "Streamable HTTP failed for {}, falling back to SSE: {}",
                url, e
            );
        }
    }

    // Fall back to SSE
    let sse_config = SseClientConfig {
        sse_endpoint: Arc::from(url),
        ..Default::default()
    };

    let transport = SseClientTransport::start_with_client(reqwest_client, sse_config)
        .await
        .map_err(|e| AppError::Mcp(format!("sse transport error: {}", e)))?;

    let client = ()
        .serve(transport)
        .await
        .map_err(|e| AppError::Mcp(format!("client start error: {}", e)))?;

    info!("Connected via SSE to {}", url);
    Ok(client.peer().clone())
}

/// 构建 stdio 命令
fn build_command(command: &str, args: &[String]) -> Command {
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd
}
