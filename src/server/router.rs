use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData as McpError, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::constants::{DEFAULT_BIND_ADDR, MCP_PATH};
use crate::error::Result;
use crate::mcp::client::AggregatedClient;
use crate::server::audit::{AuditChannel, AuditLogger};

/// Aggregated MCP Server handler.
///
/// Implements rmcp's `ServerHandler` trait to handle client tools/list and tools/call requests,
/// routing them to the corresponding upstream MCP service.
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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "MCP Tunnel - Aggregated MCP server proxying multiple upstream services. \
                 Tool names are prefixed with upstream_name__tool_name.",
            )
    }

    /// Handle tools/list request: return the aggregated tool list.
    #[tracing::instrument(skip(self))]
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        let tools = self.client.list_tools().await.map_err(|e| {
            McpError::internal_error(format!("failed to list tools: {}", e), None)
        })?;
        let upstream_names = self.client.upstream_names().await;

        info!(
            "list_tools: returning {} tool(s) from {} upstream(s)",
            tools.len(),
            upstream_names.len()
        );

        for name in &upstream_names {
            let prefix = format!("{}__", name);
            let count = tools.iter().filter(|t| t.name.starts_with(&prefix)).count();
            self.audit.log_list(name.clone(), count).await;
        }

        Ok(ListToolsResult::with_all_items(tools))
    }

    /// Handle tools/call request: parse the tool name prefix and route to the corresponding upstream.
    #[tracing::instrument(skip(self))]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let tool_name = request.name.as_ref().to_string();
        let arguments = match request.arguments {
            Some(args) => serde_json::Value::Object(args),
            None => serde_json::Value::Object(serde_json::Map::new()),
        };

        info!("call_tool: {}", tool_name);

        let upstream_name = crate::mcp::tool_filter::parse_tool_name(&tool_name)
            .map(|(upstream, _)| upstream.to_string());

        if let Some(ref name) = upstream_name {
            self.audit
                .log_call(name.clone(), Some(tool_name.clone()), Some(arguments.clone()))
                .await;
        }

        let start = std::time::Instant::now();

        match self.client.call_tool(&tool_name, arguments).await {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;

                if let Some(ref name) = upstream_name {
                    let result_json = serde_json::to_value(&result).ok();
                    self.audit
                        .log_response(name.clone(), Some(tool_name.clone()), result_json, duration_ms)
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
                        .log_error(name.clone(), Some(tool_name.clone()), error_msg.clone(), duration_ms)
                        .await;
                }

                tracing::error!(error = %e, upstream = %upstream_name.as_deref().unwrap_or("unknown"), "tool call failed");
                Err(McpError::internal_error(
                    format!("tool call failed on upstream '{}': {}", upstream_name.as_deref().unwrap_or("unknown"), error_msg),
                    None,
                ))
            }
        }
    }
}

/// Start the aggregated MCP Server (Streamable HTTP transport).
///
/// 1. Initialize AggregatedClient and connect all configured upstreams
/// 2. Create the audit log channel
/// 3. Start the axum HTTP server, mounting StreamableHttpService at /mcp
#[tracing::instrument(skip(config))]
pub async fn start_server(config: &Config) -> Result<()> {
    let client = Arc::new(AggregatedClient::new());
    client.connect_all(&config.servers).await?;

    if !client.has_any_client().await {
        return Err(crate::error::AppError::Mcp(
            "No upstream servers connected".to_string(),
        ));
    }

    let audit_channel = AuditChannel::new(1000);
    let audit = AuditLogger::new(audit_channel.sender);

    let mut audit_receiver = audit_channel.receiver;
    tokio::spawn(async move {
        while let Some(log) = audit_receiver.recv().await {
            info!(
                "[AUDIT] {:?} | upstream={} | tool={:?} | duration={}ms | error={:?}",
                log.direction, log.upstream, log.tool, log.duration_ms, log.error
            );
        }
    });

    let tools = client.list_tools().await.unwrap_or_default();
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

    let server = AggregatedServer::new(client, audit);

    let bind_addr: SocketAddr = DEFAULT_BIND_ADDR
        .parse()
        .map_err(|e| crate::error::AppError::Mcp(format!("invalid bind address: {}", e)))?;
    info!(
        "Starting MCP Tunnel server on http://{}{}",
        bind_addr, MCP_PATH
    );

    let ct = CancellationToken::new();
    let session_manager = Arc::new(LocalSessionManager::default());
    let http_config =
        StreamableHttpServerConfig::default().with_cancellation_token(ct.clone());

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        session_manager,
        http_config,
    );

    let router = axum::Router::new().nest_service(MCP_PATH, service);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| crate::error::AppError::Mcp(format!("bind error: {}", e)))?;

    info!("MCP Tunnel server is running. Press Ctrl+C to stop.");

    let serve_fut = axum::serve(listener, router).with_graceful_shutdown({
        let ct = ct.clone();
        async move { ct.cancelled_owned().await }
    });

    tokio::select! {
        result = serve_fut => {
            if let Err(e) = result {
                warn!("server exited with error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
            ct.cancel();
        }
    }

    info!("Server stopped.");
    Ok(())
}
