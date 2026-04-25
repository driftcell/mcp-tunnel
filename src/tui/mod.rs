pub mod layout;
pub mod servers;
pub mod tools;
pub mod tunnel;
pub mod audit_log;

use crate::app::{App, Tab, AddDialogType};
use crate::config::{Config, ServerConfig};
use crate::error::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use tracing::{info, warn, debug};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io;
use std::path::PathBuf;

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

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = std::time::Duration::from_millis(250);

    loop {
        terminal.draw(|f| render(f, app))?;

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

            // Drain audit logs from the serve background task into the UI buffer.
            if app.serve_running {
                let mut buf = app.serve_audit_buffer.blocking_lock();
                if !buf.is_empty() {
                    app.audit_logs.extend(buf.drain(..));
                }
            }

            last_tick = std::time::Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn render(frame: &mut ratatui::Frame, app: &mut App) {
    let (header, content, footer) = layout::main_layout(frame.area());

    let title = Paragraph::new("mcp-tunnel v0.1.0")
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(title, header);

    let (sidebar, main_area) = layout::content_layout(content);
    let tabs = ["Servers", "Tools", "Tunnel", "Audit"];
    let tab_index = match app.current_tab {
        Tab::Servers => 0,
        Tab::Tools => 1,
        Tab::Tunnel => 2,
        Tab::AuditLog => 3,
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
        Tab::Tools => tools::render_tools(frame, app, main_area),
        Tab::Tunnel => tunnel::render_tunnel(frame, app, main_area),
        Tab::AuditLog => audit_log::render_audit_log(frame, app, main_area),
    }

    let help_text = match app.current_tab {
        Tab::Servers => {
            let serve_hint = if app.serve_running { "s:stop serve" } else { "s:serve" };
            &format!("q:quit | Tab:switch | ↑↓:nav | Enter:tools | a:add | d:delete | {} | o:OAuth | O:clear OAuth", serve_hint)
        }
        Tab::Tools => "q:quit | Tab:switch | ↑↓:nav | Space:toggle | Esc:back",
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
            match app.current_tab {
                Tab::Servers => app.prev_server(),
                Tab::Tools => app.prev_tool(),
                Tab::AuditLog => app.audit_scroll = app.audit_scroll.saturating_add(1),
                _ => {}
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            match app.current_tab {
                Tab::Servers => app.next_server(),
                Tab::Tools => app.next_tool(),
                Tab::AuditLog => app.audit_scroll = app.audit_scroll.saturating_sub(1),
                _ => {}
            }
        }
        KeyCode::Enter
            if app.current_tab == Tab::Servers => {
                app.enter_tools_tab();
            }
        KeyCode::Char('a')
            if app.current_tab == Tab::Servers => {
                app.show_add_dialog = true;
                app.add_dialog_type = AddDialogType::Http;
                app.add_dialog_fields = vec![String::new(), String::new()];
                app.add_dialog_focus = 0;
            }
        KeyCode::Char('d')
            if app.current_tab == Tab::Servers => {
                if let Some(server) = app.selected_server_config() {
                    info!("Removing server: {}", server.name);
                }
                app.remove_selected_server();
                app.set_message("Server removed".to_string());
            }
        KeyCode::Char('o')
            if app.current_tab == Tab::Servers => {
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
                                    if let Some(cache) = app.config.tool_cache.iter_mut().find(|c| c.server == server_name) {
                                        cache.tools = tool_infos;
                                    } else {
                                        app.config.tool_cache.push(crate::config::ToolCache {
                                            server: server_name.clone(),
                                            tools: tool_infos,
                                        });
                                    }

                                    let _ = app.save_config();
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
            if app.current_tab == Tab::Servers => {
                if let Some(server) = app.selected_server_config() {
                    info!("Clearing OAuth token for server: {}", server.name);
                    let store = crate::mcp::oauth::FileCredentialStore::new(&server.name);
                    let _ = store.clear().await;
                    app.set_message(format!("OAuth token cleared for '{}'", server.name));
                }
            }
        KeyCode::Char(' ')
            if app.current_tab == Tab::Tools => {
                app.toggle_selected_tool();
            }
        KeyCode::Esc
            if app.current_tab == Tab::Tools => {
                app.current_tab = Tab::Servers;
            }
        KeyCode::Char('s')
            if app.current_tab == Tab::Servers => {
                if app.serve_running {
                    stop_serve(app).await;
                    app.set_message("Serve stopped".to_string());
                } else {
                    match start_serve(app).await {
                        Ok(()) => {
                            app.set_message("Serve started on http://127.0.0.1:3000/mcp".to_string());
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
                let local_url = "http://127.0.0.1:3000".to_string();

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
                } else {
                    let qt = app.quick_tunnel.as_mut().unwrap();
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
                let _ = crate::tunnel::named::run_tunnel;
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

#[tracing::instrument]
async fn run_oauth_login(name: &str, url: &str) -> Result<()> {
    use rmcp::transport::auth::{AuthError, OAuthState};

    let mut state = OAuthState::new(url, None)
        .await
        .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;

    match state
        .start_authorization(
            &[],
            crate::mcp::oauth::OAUTH_CALLBACK_URL,
            Some(env!("CARGO_PKG_NAME")),
        )
        .await
    {
        Ok(()) => {}
        Err(AuthError::NoAuthorizationSupport) => {
            return Err(crate::error::AppError::OAuth(
                "Server does not support OAuth".to_string(),
            ));
        }
        Err(e) => return Err(crate::error::AppError::OAuth(e.to_string())),
    }

    let store = crate::mcp::oauth::FileCredentialStore::new(name);

    // We can't reliably check token expiration without an issue timestamp;
    // if a token exists, treat it as valid and let the server signal 401 to re-auth.
    if store.load().await?.is_some() {
        return Ok(());
    }

    let token_response = crate::mcp::oauth::run_pkce_flow(url).await?;
    store.save(&token_response).await?;
    Ok(())
}

#[tracing::instrument(skip(app))]
async fn start_serve(app: &mut App) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    use tracing::{info, warn};

    const BIND_ADDR: &str = "127.0.0.1:3000";
    const MCP_PATH: &str = "/mcp";

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

    let mut audit_channel = crate::server::audit::AuditChannel::new(1000);
    let audit = crate::server::audit::AuditLogger::new(audit_channel.sender);
    let audit_buffer = app.serve_audit_buffer.clone();

    tokio::spawn(async move {
        while let Some(log) = audit_channel.receiver.recv().await {
            audit_buffer.lock().await.push(log);
        }
    });

    let server = crate::server::router::AggregatedServer::new(client, audit);

    let bind_addr: std::net::SocketAddr = BIND_ADDR
        .parse()
        .map_err(|e| crate::error::AppError::Mcp(format!("invalid bind address: {e}")))?;

    let ct = CancellationToken::new();
    app.serve_cancel = Some(ct.clone());

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
        }
        KeyCode::Tab => {
            if app.add_dialog_focus == app.add_dialog_fields.len() - 1 {
                app.add_dialog_type = match app.add_dialog_type {
                    AddDialogType::Http => AddDialogType::Stdio,
                    AddDialogType::Stdio => AddDialogType::Http,
                };
                app.add_dialog_focus = 0;
            } else {
                app.add_dialog_focus += 1;
            }
        }
        KeyCode::Enter => {
            let name = app.add_dialog_fields[0].trim().to_string();
            let value = app.add_dialog_fields[1].trim().to_string();
            if !name.is_empty() && !value.is_empty() {
                let dialog_type = app.add_dialog_type;
                match dialog_type {
                    AddDialogType::Http => {
                        info!("Adding HTTP server: {} -> {}", name, value);
                        app.config.servers.push(ServerConfig {
                            name: name.clone(),
                            ty: crate::config::UpstreamType::Http { url: value },
                            enabled_tools: Default::default(),
                            disabled_tools: Default::default(),
                        });
                    }
                    AddDialogType::Stdio => {
                        let parts: Vec<&str> = value.split_whitespace().collect();
                        let cmd = parts.first().map(|s| s.to_string()).unwrap_or_default();
                        let args = parts.iter().skip(1).map(|s| s.to_string()).collect();
                        info!("Adding stdio server: {} -> {} {:?}", name, cmd, args);
                        app.config.servers.push(ServerConfig {
                            name: name.clone(),
                            ty: crate::config::UpstreamType::Stdio { command: cmd, args },
                            enabled_tools: Default::default(),
                            disabled_tools: Default::default(),
                        });
                    }
                }
                app.save_config()?;
                app.set_message(format!("Added server: {}", name));
            }
            app.show_add_dialog = false;
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

