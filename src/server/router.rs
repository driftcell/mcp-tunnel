use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ErrorData as McpError, ListToolsResult, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::sse_server::{SseServer, SseServerConfig};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::error::Result;
use crate::mcp::client::AggregatedClient;
use crate::server::audit::{AuditChannel, AuditLogger};

/// 聚合 MCP Server 的 handler
///
/// 实现 rmcp 的 `ServerHandler` trait，处理客户端的 tools/list 和 tools/call 请求，
/// 将请求路由到对应的上游 MCP 服务。
#[derive(Clone)]
pub struct AggregatedServer {
    client: Arc<AggregatedClient>,
    audit: AuditLogger,
}

impl AggregatedServer {
    pub fn new(client: Arc<AggregatedClient>, audit: AuditLogger) -> Self {
        Self { client, audit }
    }
}

impl ServerHandler for AggregatedServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: rmcp::model::ProtocolVersion::default(),
            capabilities: rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: rmcp::model::Implementation {
                name: "mcp-tunnel".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "MCP Tunnel - Aggregated MCP server proxying multiple upstream services. \
                 Tool names are prefixed with upstream_name__tool_name."
                    .to_string(),
            ),
        }
    }

    /// 处理 tools/list 请求：返回聚合后的工具列表
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        let tools = self.client.list_tools().await;
        let upstream_names = self.client.upstream_names().await;

        info!(
            "list_tools: returning {} tool(s) from {} upstream(s)",
            tools.len(),
            upstream_names.len()
        );

        // 审计日志：记录每个上游的工具数量
        for name in &upstream_names {
            let count = tools
                .iter()
                .filter(|t| t.name.starts_with(&format!("{}__", name)))
                .count();
            self.audit.log_list(name.clone(), count).await;
        }

        Ok(ListToolsResult::with_all_items(tools))
    }

    /// 处理 tools/call 请求：解析工具名前缀，路由到对应上游
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let tool_name = request.name.as_ref();
        let arguments = match request.arguments {
            Some(args) => serde_json::Value::Object(args),
            None => serde_json::Value::Object(serde_json::Map::new()),
        };

        info!("call_tool: {}", tool_name);

        // 解析上游名称
        let upstream_name = crate::mcp::tool_filter::parse_tool_name(tool_name)
            .map(|(upstream, _)| upstream.to_string());

        // 记录审计日志（调用前）
        if let Some(ref name) = upstream_name {
            self.audit
                .log_call(name.clone(), Some(tool_name.to_string()), Some(arguments.clone()))
                .await;
        }

        let start = std::time::Instant::now();

        match self.client.call_tool(tool_name, arguments.clone()).await {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;

                if let Some(ref name) = upstream_name {
                    let result_json = serde_json::to_value(&result).ok();
                    self.audit
                        .log_response(
                            name.clone(),
                            Some(tool_name.to_string()),
                            result_json,
                            duration_ms,
                        )
                        .await;
                }

                info!("call_tool: {} completed in {}ms", tool_name, duration_ms);
                Ok(result)
            }
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                let error_msg = e.to_string();

                warn!("call_tool: {} failed: {}", tool_name, error_msg);

                if let Some(ref name) = upstream_name {
                    self.audit
                        .log_error(
                            name.clone(),
                            Some(tool_name.to_string()),
                            error_msg.clone(),
                            duration_ms,
                        )
                        .await;
                }

                Err(McpError::internal_error(
                    format!("tool call failed: {}", error_msg),
                    None,
                ))
            }
        }
    }
}

/// 启动聚合 MCP Server
///
/// 1. 初始化 AggregatedClient 并连接所有配置的上游
/// 2. 创建审计日志通道
/// 3. 启动 SSE HTTP server，监听 MCP 客户端连接
/// 4. 每个 SSE 连接对应一个独立的 MCP session
pub async fn start_server(config: &Config) -> Result<()> {
    // 初始化聚合客户端
    let client = Arc::new(AggregatedClient::new());
    client.connect_all(&config.servers).await?;

    if !client.has_any_client().await {
        return Err(crate::error::AppError::Mcp(
            "No upstream servers connected".to_string(),
        ));
    }

    // 创建审计通道
    let audit_channel = AuditChannel::new(1000);
    let audit = AuditLogger::new(audit_channel.sender);

    // 启动审计日志消费者（后台任务）
    let mut audit_receiver = audit_channel.receiver;
    tokio::spawn(async move {
        while let Some(log) = audit_receiver.recv().await {
            info!(
                "[AUDIT] {:?} | upstream={} | tool={:?} | duration={}ms | error={:?}",
                log.direction, log.upstream, log.tool, log.duration_ms, log.error
            );
        }
    });

    // 打印聚合工具列表
    let tools = client.list_tools().await;
    let upstreams = client.upstream_names().await;
    info!(
        "Aggregated {} tool(s) from {} upstream(s): {:?}",
        tools.len(),
        upstreams.len(),
        upstreams
    );
    for tool in &tools {
        info!("  - {}", tool.name);
    }

    // 构建聚合 server handler
    let server = AggregatedServer::new(client, audit);

    // 启动 SSE HTTP server
    let bind_addr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
    info!("Starting MCP Tunnel server on http://{}", bind_addr);
    info!("  SSE endpoint: http://{}/sse", bind_addr);
    info!("  Message endpoint: http://{}/message", bind_addr);

    let ct = CancellationToken::new();
    let sse_config = SseServerConfig {
        bind: bind_addr,
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        ct: ct.clone(),
        sse_keep_alive: None,
    };

    let sse_server = SseServer::serve_with_config(sse_config)
        .await
        .map_err(|e| crate::error::AppError::Mcp(format!("SSE server error: {}", e)))?;

    // 为每个传入的 SSE 连接创建一个新的 server 实例
    let server_ct = sse_server.with_service(move || server.clone());

    info!("MCP Tunnel server is running. Press Ctrl+C to stop.");

    // 等待中断信号
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
    }

    server_ct.cancel();
    info!("Server stopped.");

    Ok(())
}
