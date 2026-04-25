pub mod layout;
pub mod servers;
pub mod tunnel;
pub mod audit_log;

use crate::app::{App, Tab, AddDialogType};
use crate::config::{Config, ServerConfig};
use crate::constants::{DEFAULT_BASE_URL, DEFAULT_BIND_ADDR, MCP_PATH, TICK_RATE_MS};
use crate::error::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

#[tracing::instrument(skip(config))]
pub async fn run_tui(config: Config, config_path: PathBuf) -> Result<()> {
    info!("Starting TUI");
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config, config_path);

    let result = run_app(&mut terminal, &mut app).await;

    if app.serve_running {
        info!("TUI quitting, stopping serve...");
        stop_serve(&mut app).await;
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

enum BackgroundInitMsg {
    Tools(String, Vec<crate::config::ToolInfo>),
    Status(String),
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = std::time::Duration::from_millis(TICK_RATE_MS);

    let (init_tx, mut init_rx) = mpsc::channel::<BackgroundInitMsg>(16);

    let http_servers: Vec<ServerConfig> = app
        .config
        .servers
        .iter()
        .filter(|s| matches!(s.ty, crate::config::UpstreamType::Http { .. }))
        .cloned()
        .collect();

    let mut bg_init_handle = if !http_servers.is_empty() {
        let init_cancel = CancellationToken::new();
        let init_cancel_clone = init_cancel.clone();
        let handle = tokio::spawn(async move {
            let mut tasks = Vec::new();
            for server in http_servers {
                let tx = init_tx.clone();
                let ct = init_cancel_clone.clone();
                let task = tokio::spawn(async move {
                    let name = server.name.clone();
                    let _url = match &server.ty {
                        crate::config::UpstreamType::Http { url } => url.clone(),
                        _ => return,
                    };

                    // Check if token exists before any network I/O.
                    let has_token = match check_oauth_token_exists(&name).await {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = tx
                                .send(BackgroundInitMsg::Status(format!(
                                    "Token check failed for '{}': {}",
                                    name, e
                                )))
                                .await;
                            return;
                        }
                    };

                    if !has_token {
                        let _ = tx
                            .send(BackgroundInitMsg::Status(format!(
                                "'{}' needs OAuth (press 'o')",
                                name
                            )))
                            .await;
                        return;
                    }

                    if ct.is_cancelled() {
                        return;
                    }

                    // Token exists — run tool discovery with a timeout.
                    let discovery_fut = crate::mcp::client::discover_tools(&server);
                    match tokio::time::timeout(std::time::Duration::from_secs(30), discovery_fut).await
                    {
                        Ok(Ok(tools)) => {
                            let tool_infos: Vec<crate::config::ToolInfo> = tools
                                .into_iter()
                                .map(|t| crate::config::ToolInfo {
                                    name: t.name.as_ref().to_string(),
                                    description: t.description.unwrap_or_default().to_string(),
                                    enabled: true,
                                })
                                .collect();
                            let _ = tx
                                .send(BackgroundInitMsg::Tools(name.clone(), tool_infos))
                                .await;
                        }
                        Ok(Err(e)) => {
                            let _ = tx
                                .send(BackgroundInitMsg::Status(format!(
                                    "Tool discovery failed for '{}': {}",
                                    name, e
                                )))
                                .await;
                        }
                        Err(_) => {
                            let _ = tx
                                .send(BackgroundInitMsg::Status(format!(
                                    "Tool discovery timed out for '{}'",
                                    name
                                )))
                                .await;
                        }
                    }
                });
                tasks.push(task);
            }
            for t in tasks {
                let _ = t.await;
            }
        });
        Some((handle, init_cancel))
    } else {
        None
    };

    loop {
        terminal.draw(|f| render(f, app)).map_err(|e| std::io::Error::other(e.to_string()))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| std::time::Duration::from_secs(0));

        if crossterm::event::poll(timeout)?
            && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press {
                    handle_key(app, key).await?;
                }

        if last_tick.elapsed() >= tick_rate {
            app.clear_message_if_expired();

            // Process background OAuth / tool-discovery results.
            while let Ok(msg) = init_rx.try_recv() {
                match msg {
                    BackgroundInitMsg::Tools(server_name, tool_infos) => {
                        let count = tool_infos.len();
                        if let Some(cache) =
                            app.tool_cache.iter_mut().find(|c| c.server == server_name)
                        {
                            // Merge discovered tools into existing cache.
                            // The `enabled` field on ToolInfo is kept for backward compat
                            // but is no longer used for logic; tool state comes from
                            // ServerConfig.disabled_tools instead.
                            let mut merged = Vec::new();
                            for new_tool in tool_infos {
                                if cache.tools.iter().any(|t| t.name == new_tool.name) {
                                    // Tool already cached — keep the existing entry (which may
                                    // have a description that was manually edited, etc.)
                                    if let Some(existing) = cache.tools.iter().find(|t| t.name == new_tool.name) {
                                        merged.push(existing.clone());
                                    }
                                } else {
                                    merged.push(new_tool);
                                }
                            }
                            cache.tools = merged;
                        } else {
                            app.tool_cache.push(crate::config::ToolCache {
                                server: server_name.clone(),
                                tools: tool_infos,
                            });
                        }
                        info!("Auto-OAuth: discovered {} tools from '{}'", count, server_name);
                        app.set_message(format!(
                            "Auto-OAuth: {} tools from '{}'",
                            count, server_name
                        ));
                    }
                    BackgroundInitMsg::Status(msg) => {
                        warn!("Auto-OAuth status: {}", msg);
                        app.set_message(msg);
                    }
                }
            }

            // Drain audit logs from the serve background task into the UI buffer.
            // Use try_lock to avoid blocking the async executor; skip if contended.
            // Cap the total audit log count to prevent unbounded growth.
            if app.serve_running
                && let Ok(mut buf) = app.serve_audit_buffer.try_lock()
                    && !buf.is_empty() {
                        app.audit_logs.extend(buf.drain(..));
                        const MAX_AUDIT_LOGS: usize = 10000;
                        if app.audit_logs.len() > MAX_AUDIT_LOGS {
                            let drop_count = app.audit_logs.len() - MAX_AUDIT_LOGS;
                            app.audit_logs.drain(0..drop_count);
                        }
                    }

            last_tick = std::time::Instant::now();
        }

        if app.should_quit {
            if let Some((_, cancel)) = bg_init_handle.take() {
                cancel.cancel();
            }
            break;
        }
    }

    // Ensure background init task is cancelled on exit.
    if let Some((handle, cancel)) = bg_init_handle {
        cancel.cancel();
        let _ = handle.await;
    }

    Ok(())
}

fn render(frame: &mut ratatui::Frame, app: &mut App) {
    let (header, content, footer) = layout::main_layout(frame.area());

    let title = Paragraph::new("mcp-tunnel v0.1.0")
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(title, header);

    let (sidebar, main_area) = layout::content_layout(content);
    let tabs = ["Servers", "Tunnel", "Audit"];
    let tab_index = match app.current_tab {
        Tab::Servers => 0,
        Tab::Tunnel => 1,
        Tab::AuditLog => 2,
    };

    let tab_text: Vec<Line> = tabs.iter().enumerate().map(|(i, t)| {
        let style = if i == tab_index {
            Style::default().fg(Color::Yellow).add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            Style::default()
        };
        Line::from(Span::styled(format!(" {} ", t), style))
    }).collect();

    let sidebar_widget = Paragraph::new(tab_text)
        .block(Block::default().borders(Borders::ALL).title("Tabs"));
    frame.render_widget(sidebar_widget, sidebar);

    match app.current_tab {
        Tab::Servers => servers::render_servers(frame, app, main_area),
        Tab::Tunnel => tunnel::render_tunnel(frame, app, main_area),
        Tab::AuditLog => audit_log::render_audit_log(frame, app, main_area),
    }

    let servers_help;
    let tools_help;
    let help_text: &str = match app.current_tab {
        Tab::Servers => {
            if app.show_tools {
                tools_help = "q:quit | ↑↓:nav | Space:toggle | f:fold | Esc:back | Tab:switch".to_string();
                &tools_help
            } else {
                let serve_hint = if app.serve_running { "s:stop serve" } else { "s:serve" };
                servers_help = format!(
                    "q:quit | ↑↓:nav | Enter:tools | a:add | e:edit | d:delete | {} | o:OAuth | O:clear OAuth | Tab:switch",
                    serve_hint
                );
                &servers_help
            }
        }
        Tab::Tunnel => "q:quit | Tab:switch | t:toggle | r:named tunnel ref",
        Tab::AuditLog => "q:quit | Tab:switch | ↑↓:scroll | c:clear",
    };
    let footer_widget = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer_widget, footer);

    if let Some(ref msg) = app.message {
        let msg_area = layout::centered_rect(60, 20, frame.area());
        let msg_widget = Paragraph::new(msg.as_str())
            .block(Block::default().borders(Borders::ALL).title("Message"))
            .style(Style::default().fg(Color::Green));
        frame.render_widget(msg_widget, msg_area);
    }
}

async fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<()> {
    debug!("key pressed: {:?}", key.code);

    if app.show_add_dialog {
        handle_add_dialog_key(app, key).await?;
        return Ok(());
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            info!("TUI quit requested");
            app.should_quit = true;
        }
        KeyCode::Tab => {
            app.next_tab();
        }
        KeyCode::BackTab => {
            app.prev_tab();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.current_tab == Tab::Servers && app.show_tools {
                app.prev_tool();
            } else {
                match app.current_tab {
                    Tab::Servers => app.prev_server(),
                    Tab::AuditLog => app.audit_scroll = app.audit_scroll.saturating_sub(1),
                    _ => {}
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.current_tab == Tab::Servers && app.show_tools {
                app.next_tool();
            } else {
                match app.current_tab {
                    Tab::Servers => app.next_server(),
                    Tab::AuditLog => app.audit_scroll = app.audit_scroll.saturating_add(1),
                    _ => {}
                }
            }
        }
        KeyCode::Enter
            if app.current_tab == Tab::Servers && !app.show_tools => {
                app.enter_tools_view();
            }
        KeyCode::Char('a')
            if app.current_tab == Tab::Servers && !app.show_tools => {
                app.show_add_dialog = true;
                app.is_edit_mode = false;
                app.add_dialog_type = AddDialogType::Http;
                app.add_dialog_fields = vec![String::new(), String::new()];
                app.add_dialog_focus = 0;
            }
        KeyCode::Char('e')
            if app.current_tab == Tab::Servers && !app.show_tools => {
                if let Some(server) = app.selected_server_config().cloned() {
                    app.show_add_dialog = true;
                    app.is_edit_mode = true;
                    app.add_dialog_type = match server.ty {
                        crate::config::UpstreamType::Http { .. } => AddDialogType::Http,
                        crate::config::UpstreamType::Stdio { .. } => AddDialogType::Stdio,
                    };
                    let value = match &server.ty {
                        crate::config::UpstreamType::Http { url } => url.clone(),
                        crate::config::UpstreamType::Stdio { command, args } => {
                            if args.is_empty() {
                                command.clone()
                            } else {
                                format!("{} {}", command, args.join(" "))
                            }
                        }
                    };
                    app.add_dialog_fields = vec![server.name, value];
                    app.add_dialog_focus = 0;
                }
            }
        KeyCode::Char('d')
            if app.current_tab == Tab::Servers && !app.show_tools => {
                if let Some(server) = app.selected_server_config() {
                    info!("Removing server: {}", server.name);
                }
                match app.remove_selected_server() {
                    Ok(()) => app.set_message("Server removed".to_string()),
                    Err(e) => {
                        warn!("Failed to remove server: {}", e);
                        app.set_message(format!("Failed to remove server: {}", e));
                    }
                }
            }
        KeyCode::Char('o')
            if app.current_tab == Tab::Servers && !app.show_tools => {
                if let Some(server) = app.selected_server_config().cloned() {
                    let server_name = server.name.clone();
                    let url = match &server.ty {
                        crate::config::UpstreamType::Http { url } => url.clone(),
                        _ => {
                            app.set_message("OAuth only supported for HTTP servers".to_string());
                            return Ok(());
                        }
                    };

                    info!("Starting OAuth login for server: {}", server_name);
                    match run_oauth_login(&server_name, &url).await {
                        Ok(_) => {
                            // OAuth succeeded — discover tools from the server
                            match crate::mcp::client::discover_tools(&server).await {
                                Ok(tools) => {
                                    let tool_infos: Vec<crate::config::ToolInfo> = tools
                                        .into_iter()
                                        .map(|t| crate::config::ToolInfo {
                                            name: t.name.as_ref().to_string(),
                                            description: t.description.unwrap_or_default().to_string(),
                                            enabled: true,
                                        })
                                        .collect();

                                    let count = tool_infos.len();

                                    // Update or add to tool_cache
                                    if let Some(cache) = app.tool_cache.iter_mut().find(|c| c.server == server_name) {
                                        cache.tools = tool_infos;
                                    } else {
                                        app.tool_cache.push(crate::config::ToolCache {
                                            server: server_name.clone(),
                                            tools: tool_infos,
                                        });
                                    }

                                    info!("OAuth success for '{}', discovered {} tools", server_name, count);
                                    app.set_message(format!(
                                        "OAuth ok for '{}'. {} tools discovered. Press Enter to view.",
                                        server_name, count
                                    ));
                                }
                                Err(e) => {
                                    warn!("OAuth ok for '{}', but tool discovery failed: {}", server_name, e);
                                    app.set_message(format!(
                                        "OAuth ok for '{}', but tool discovery failed: {}",
                                        server_name, e
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            warn!("OAuth login failed for '{}': {}", server_name, e);
                            app.set_message(format!("OAuth login failed: {}", e))
                        }
                    }
                }
            }
        KeyCode::Char('O')
            if app.current_tab == Tab::Servers && !app.show_tools => {
                if let Some(server) = app.selected_server_config() {
                    info!("Clearing OAuth token for server: {}", server.name);
                    let store = match crate::mcp::oauth::FileCredentialStore::new(&server.name) {
                        Ok(s) => s,
                        Err(e) => {
                            warn!("Failed to create credential store: {}", e);
                            app.set_message(format!("OAuth error: {}", e));
                            return Ok(());
                        }
                    };
                    let _ = store.clear().await;
                    app.set_message(format!("OAuth token cleared for '{}'", server.name));
                }
            }
        KeyCode::Char(' ')
            if app.current_tab == Tab::Servers && app.show_tools => {
                if let Err(e) = app.toggle_selected_tool() {
                    warn!("Failed to toggle tool: {}", e);
                    app.set_message(format!("Failed to toggle tool: {}", e));
                }
            }
        KeyCode::Esc
            if app.current_tab == Tab::Servers && app.show_tools => {
                app.show_tools = false;
            }
        KeyCode::Char('f')
            if app.current_tab == Tab::Servers && app.show_tools => {
                app.tools_folded = !app.tools_folded;
            }
        KeyCode::Char('s')
            if app.current_tab == Tab::Servers && !app.show_tools => {
                if app.serve_running {
                    stop_serve(app).await;
                    app.set_message("Serve stopped".to_string());
                } else {
                    match start_serve(app).await {
                        Ok(()) => {
                            app.set_message(format!("Serve started on {}{}", DEFAULT_BASE_URL, MCP_PATH));
                        }
                        Err(e) => {
                            warn!("Failed to start serve: {}", e);
                            app.set_message(format!("Failed to start serve: {}", e));
                        }
                    }
                }
            }
        KeyCode::Char('t')
            if app.current_tab == Tab::Tunnel => {
                let local_url = DEFAULT_BASE_URL.to_string();

                if app.quick_tunnel.is_none() {
                    info!("Starting QuickTunnel");
                    let mut qt = crate::tunnel::quick::QuickTunnel::new();
                    match qt.start(&local_url).await {
                        Ok(url) => {
                            info!("QuickTunnel started: {}", url);
                            app.tunnel_url = Some(url.clone());
                            app.quick_tunnel_running = true;
                            app.set_message(format!("QuickTunnel started: {}", url));
                        }
                        Err(e) => {
                            warn!("Failed to start QuickTunnel: {}", e);
                            app.set_message(format!("Failed to start QuickTunnel: {}", e));
                            return Ok(());
                        }
                    }
                    app.quick_tunnel = Some(qt);
                } else if let Some(qt) = app.quick_tunnel.as_mut() {
                    if qt.is_running() {
                        info!("Stopping QuickTunnel");
                        if let Err(e) = qt.stop().await {
                            warn!("Failed to stop QuickTunnel: {}", e);
                            app.set_message(format!("Failed to stop QuickTunnel: {}", e));
                            return Ok(());
                        }
                        info!("QuickTunnel stopped");
                        app.quick_tunnel_running = false;
                        app.tunnel_url = None;
                        app.set_message("QuickTunnel stopped.".to_string());
                    } else {
                        info!("Restarting QuickTunnel");
                        match qt.start(&local_url).await {
                            Ok(url) => {
                                info!("QuickTunnel restarted: {}", url);
                                app.tunnel_url = Some(url.clone());
                                app.quick_tunnel_running = true;
                                app.set_message(format!("QuickTunnel started: {}", url));
                            }
                            Err(e) => {
                                warn!("Failed to restart QuickTunnel: {}", e);
                                app.set_message(format!("Failed to start QuickTunnel: {}", e));
                                return Ok(());
                            }
                        }
                    }
                }
            }
        KeyCode::Char('r')
            if app.current_tab == Tab::Tunnel => {
                app.set_message("Named tunnel: use CLI 'mcp-tunnel tunnel run <name>'".to_string());
            }
        KeyCode::Char('c')
            if app.current_tab == Tab::AuditLog => {
                app.audit_logs.clear();
                app.audit_scroll = 0;
            }
        _ => {}
    }
    Ok(())
}

/// Check whether a valid (non-expired) OAuth token already exists for a server.
/// This performs NO network I/O.
#[tracing::instrument]
async fn check_oauth_token_exists(name: &str) -> Result<bool> {
    let store = crate::mcp::oauth::FileCredentialStore::new(name)?;
    Ok(store.load().await?.is_some())
}

#[tracing::instrument]
async fn run_oauth_login(name: &str, url: &str) -> Result<()> {
    // Check if a valid (non-expired) token already exists BEFORE any network I/O.
    let store = crate::mcp::oauth::FileCredentialStore::new(name)?;
    if store.load().await?.is_some() {
        return Ok(());
    }

    let (client_id, token_response) = match crate::mcp::oauth::run_pkce_flow(url).await? {
        crate::mcp::oauth::PkceFlowResult::Success { client_id, token } => (client_id, token),
        crate::mcp::oauth::PkceFlowResult::NoAuthorizationSupport => {
            return Err(crate::error::AppError::OAuth(
                "Server does not support OAuth".to_string(),
            ));
        }
    };
    store.save_with_client_id(&token_response, &client_id).await?;
    Ok(())
}

#[tracing::instrument(skip(app))]
async fn start_serve(app: &mut App) -> Result<()> {
    let config = app.config.clone();

    let client = Arc::new(crate::mcp::client::AggregatedClient::new());
    client.connect_all(&config.servers).await?;

    if !client.has_any_client().await {
        return Err(crate::error::AppError::Mcp(
            "No upstream servers connected".to_string(),
        ));
    }

    let tools = client.list_tools().await;
    let upstreams = client.upstream_names().await;
    info!(
        "Aggregated {} tool(s) from {} upstream(s): {:?}",
        tools.len(),
        upstreams.len(),
        upstreams
    );

    let ct = CancellationToken::new();
    app.serve_cancel = Some(ct.clone());

    let mut audit_channel = crate::server::audit::AuditChannel::new(1000);
    let audit = crate::server::audit::AuditLogger::new(audit_channel.sender);
    let audit_buffer = app.serve_audit_buffer.clone();

    let audit_ct = ct.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(log) = audit_channel.receiver.recv() => {
                    let mut buf = audit_buffer.lock().unwrap();
                    buf.push(log);
                    // Cap the background buffer to prevent unbounded growth
                    const MAX_BUFFERED_AUDIT_LOGS: usize = 5000;
                    if buf.len() > MAX_BUFFERED_AUDIT_LOGS {
                        let drop_count = buf.len() - MAX_BUFFERED_AUDIT_LOGS;
                        buf.drain(0..drop_count);
                    }
                }
                _ = audit_ct.cancelled() => {
                    info!("Audit log receiver shutting down gracefully");
                    break;
                }
            }
        }
    });

    let server = crate::server::router::AggregatedServer::new(client, audit);

    let bind_addr: std::net::SocketAddr = DEFAULT_BIND_ADDR
        .parse()
        .map_err(|e| crate::error::AppError::Mcp(format!("invalid bind address: {e}")))?;

    let session_manager = Arc::new(LocalSessionManager::default());
    let http_config = StreamableHttpServerConfig::default().with_cancellation_token(ct.clone());

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        session_manager,
        http_config,
    );

    let router = axum::Router::new().nest_service(MCP_PATH, service);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| crate::error::AppError::Mcp(format!("bind error: {e}")))?;

    app.serve_running = true;
    info!("Serve started on http://{bind_addr}{MCP_PATH}");

    tokio::spawn(async move {
        let serve_fut = axum::serve(listener, router).with_graceful_shutdown({
            let ct = ct.clone();
            async move { ct.cancelled_owned().await }
        });

        if let Err(e) = serve_fut.await {
            warn!("server exited with error: {e}");
        }
        info!("Serve stopped.");
    });

    Ok(())
}

#[tracing::instrument(skip(app))]
async fn stop_serve(app: &mut App) {
    if let Some(ct) = app.serve_cancel.take() {
        ct.cancel();
    }
    app.serve_running = false;
    info!("Serve stopping...");
}

async fn handle_add_dialog_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<()> {
    use crossterm::event::KeyCode;

    match key.code {
        KeyCode::Esc => {
            app.show_add_dialog = false;
            app.is_edit_mode = false;
        }
        KeyCode::Tab => {
            if !app.is_edit_mode {
                if app.add_dialog_focus == app.add_dialog_fields.len() - 1 {
                    app.add_dialog_type = match app.add_dialog_type {
                        AddDialogType::Http => AddDialogType::Stdio,
                        AddDialogType::Stdio => AddDialogType::Http,
                    };
                    app.add_dialog_focus = 0;
                } else {
                    app.add_dialog_focus += 1;
                }
            } else {
                app.add_dialog_focus = (app.add_dialog_focus + 1) % app.add_dialog_fields.len();
            }
        }
        KeyCode::Enter => {
            let name = app.add_dialog_fields[0].trim().to_string();
            let value = app.add_dialog_fields[1].trim().to_string();
            if !name.is_empty() && !value.is_empty() {
                if app.is_edit_mode {
                    let original_name = app.config.servers.get(app.selected_server)
                        .map(|s| s.name.clone());

                    let dialog_type = app.add_dialog_type;
                    let server = match dialog_type {
                        AddDialogType::Http => {
                            info!("Editing HTTP server: {} -> {}", name, value);
                            ServerConfig {
                                name: name.clone(),
                                ty: crate::config::UpstreamType::Http { url: value },
                                enabled_tools: Default::default(),
                                disabled_tools: Default::default(),
                            }
                        }
                        AddDialogType::Stdio => {
                            let parts: Vec<&str> = value.split_whitespace().collect();
                            let cmd = parts.first().map(|s| s.to_string()).unwrap_or_default();
                            let args = parts.iter().skip(1).map(|s| s.to_string()).collect();
                            info!("Editing stdio server: {} -> {} {:?}", name, cmd, args);
                            ServerConfig {
                                name: name.clone(),
                                ty: crate::config::UpstreamType::Stdio { command: cmd, args },
                                enabled_tools: Default::default(),
                                disabled_tools: Default::default(),
                            }
                        }
                    };
                    if let Err(e) = server.validate() {
                        app.set_message(format!("Invalid server: {}", e));
                        app.show_add_dialog = false;
                        app.is_edit_mode = false;
                        return Ok(());
                    }

                    // Preserve enabled_tools and disabled_tools from original server
                    if let Some(original) = app.config.servers.get(app.selected_server) {
                        let mut updated = server;
                        updated.enabled_tools = original.enabled_tools.clone();
                        updated.disabled_tools = original.disabled_tools.clone();
                        app.config.servers[app.selected_server] = updated;
                    }

                    // Update tool_cache server name if it changed
                    if let Some(orig_name) = original_name {
                        if orig_name != name {
                            if let Some(cache) = app.tool_cache.iter_mut().find(|c| c.server == orig_name) {
                                cache.server = name.clone();
                            }
                        }
                    }

                    app.save_config()?;
                    app.set_message(format!("Updated server: {}", name));
                } else {
                    let dialog_type = app.add_dialog_type;
                    let server = match dialog_type {
                        AddDialogType::Http => {
                            info!("Adding HTTP server: {} -> {}", name, value);
                            ServerConfig {
                                name: name.clone(),
                                ty: crate::config::UpstreamType::Http { url: value },
                                enabled_tools: Default::default(),
                                disabled_tools: Default::default(),
                            }
                        }
                        AddDialogType::Stdio => {
                            let parts: Vec<&str> = value.split_whitespace().collect();
                            let cmd = parts.first().map(|s| s.to_string()).unwrap_or_default();
                            let args = parts.iter().skip(1).map(|s| s.to_string()).collect();
                            info!("Adding stdio server: {} -> {} {:?}", name, cmd, args);
                            ServerConfig {
                                name: name.clone(),
                                ty: crate::config::UpstreamType::Stdio { command: cmd, args },
                                enabled_tools: Default::default(),
                                disabled_tools: Default::default(),
                            }
                        }
                    };
                    if let Err(e) = server.validate() {
                        app.set_message(format!("Invalid server: {}", e));
                        app.show_add_dialog = false;
                        return Ok(());
                    }
                    app.config.servers.push(server);
                    app.save_config()?;
                    app.set_message(format!("Added server: {}", name));
                }
            }
            app.show_add_dialog = false;
            app.is_edit_mode = false;
        }
        KeyCode::Char(c) => {
            if let Some(field) = app.add_dialog_fields.get_mut(app.add_dialog_focus) {
                field.push(c);
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = app.add_dialog_fields.get_mut(app.add_dialog_focus) {
                field.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

