use std::collections::HashMap;

use oauth2::TokenResponse;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::auth::{AuthError, OAuthState};
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
}

/// 单个上游的客户端包装
struct UpstreamClient {
    /// rmcp 客户端 Peer（用于发送请求）
    peer: Peer<RoleClient>,
    /// 缓存的工具列表（原始名称）
    tools: Vec<Tool>,
    /// Keep the RunningService alive so the background transport task isn't cancelled.
    /// Both stdio and HTTP use `()` as the service type, so the type is the same.
    #[allow(dead_code)]
    _service: RunningService<RoleClient, ()>,
}

impl AggregatedClient {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// 连接到所有配置的上游服务
    #[tracing::instrument(skip(self, configs))]
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
                    _service: client,
                })
            }
            crate::config::UpstreamType::Http { url } => {
                let store = crate::mcp::oauth::FileCredentialStore::new(&config.name);
                let stored_token = store.load().await?;

                let token = if let Some(t) = stored_token {
                    // We don't track token issue time, so we use stored tokens optimistically.
                    // If the server returns 401, the user can clear the token and re-auth.
                    Some(t.access_token().secret().clone())
                } else if upstream_supports_oauth(url).await? {
                    let new_token = crate::mcp::oauth::run_pkce_flow(url).await?;
                    store.save(&new_token).await?;
                    info!("OAuth token saved for server '{}'", config.name);
                    Some(new_token.access_token().secret().clone())
                } else {
                    None
                };

                let reqwest_client = build_reqwest_client(token.as_deref())?;
                let (peer, service) = connect_http(url, reqwest_client).await?;

                let tools = match peer.list_all_tools().await {
                    Ok(tools) => tools,
                    Err(e) => {
                        warn!("Failed to list tools for '{}': {}", config.name, e);
                        Vec::new()
                    }
                };

                Ok(UpstreamClient {
                    peer,
                    tools: apply_filter(&config, tools),
                    _service: service,
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
            let store = crate::mcp::oauth::FileCredentialStore::new(&config.name);
            let token = store
                .load()
                .await?
                .map(|t| t.access_token().secret().clone());

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

/// 探测上游 HTTP 服务是否支持 OAuth。失败时回退为 false。
async fn upstream_supports_oauth(url: &str) -> Result<bool> {
    let mut state = OAuthState::new(url, None)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    match state
        .start_authorization(
            &[],
            crate::mcp::oauth::OAUTH_CALLBACK_URL,
            Some(env!("CARGO_PKG_NAME")),
        )
        .await
    {
        Ok(()) => Ok(true),
        Err(AuthError::NoAuthorizationSupport) => Ok(false),
        Err(e) => Err(AppError::OAuth(e.to_string())),
    }
}
