pub mod layout;
pub mod servers;
pub mod tools;
pub mod tunnel;
pub mod audit_log;

use crate::app::{App, Tab, AddDialogType};
use crate::config::{Config, ServerConfig};
use crate::error::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io;
use std::path::PathBuf;

pub async fn run_tui(config: Config, config_path: PathBuf) -> Result<()> {
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
        Tab::Servers => "q:quit | Tab:switch | ↑↓:nav | Enter:tools | a:add | d:delete | s:serve | o:OAuth login | O:clear OAuth",
        Tab::Tools => "q:quit | Tab:switch | ↑↓:nav | Space:toggle | Esc:back",
        Tab::Tunnel => "q:quit | Tab:switch | t:toggle | r:named tunnel ref",
        Tab::AuditLog => "q:quit | Tab:switch | ↑↓:scroll | c:clear",
    };
    let footer_widget = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer_widget, footer);

    if let Some(ref msg) = app.message {
        let msg_area = centered_rect(60, 20, frame.area());
        let msg_widget = Paragraph::new(msg.as_str())
            .block(Block::default().borders(Borders::ALL).title("Message"))
            .style(Style::default().fg(Color::Green));
        frame.render_widget(msg_widget, msg_area);
    }
}

async fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<()> {
    if app.show_add_dialog {
        handle_add_dialog_key(app, key).await?;
        return Ok(());
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
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
                                    app.set_message(format!(
                                        "OAuth ok for '{}'. {} tools discovered. Press Enter to view.",
                                        server_name, count
                                    ));
                                }
                                Err(e) => {
                                    app.set_message(format!(
                                        "OAuth ok for '{}', but tool discovery failed: {}",
                                        server_name, e
                                    ));
                                }
                            }
                        }
                        Err(e) => app.set_message(format!("OAuth login failed: {}", e)),
                    }
                }
            }
        KeyCode::Char('O')
            if app.current_tab == Tab::Servers => {
                if let Some(server) = app.selected_server_config() {
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
        KeyCode::Char('s') => {
            app.set_message("Serve: not yet implemented in TUI".to_string());
        }
        KeyCode::Char('t')
            if app.current_tab == Tab::Tunnel => {
                let local_url = "http://127.0.0.1:3000".to_string();

                if app.quick_tunnel.is_none() {
                    let mut qt = crate::tunnel::quick::QuickTunnel::new();
                    match qt.start(&local_url).await {
                        Ok(url) => {
                            app.tunnel_url = Some(url.clone());
                            app.quick_tunnel_running = true;
                            app.set_message(format!("QuickTunnel started: {}", url));
                        }
                        Err(e) => {
                            app.set_message(format!("Failed to start QuickTunnel: {}", e));
                            return Ok(());
                        }
                    }
                    app.quick_tunnel = Some(qt);
                } else {
                    let qt = app.quick_tunnel.as_mut().unwrap();
                    if qt.is_running() {
                        if let Err(e) = qt.stop().await {
                            app.set_message(format!("Failed to stop QuickTunnel: {}", e));
                            return Ok(());
                        }
                        app.quick_tunnel_running = false;
                        app.tunnel_url = None;
                        app.set_message("QuickTunnel stopped.".to_string());
                    } else {
                        match qt.start(&local_url).await {
                            Ok(url) => {
                                app.tunnel_url = Some(url.clone());
                                app.quick_tunnel_running = true;
                                app.set_message(format!("QuickTunnel started: {}", url));
                            }
                            Err(e) => {
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

async fn run_oauth_login(name: &str, url: &str) -> Result<()> {
    use rmcp::transport::auth::AuthError;

    let mut state = rmcp::transport::auth::OAuthState::new(url, None)
        .await
        .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;

    let has_oauth = match state.start_authorization(&[], "http://127.0.0.1:9876/callback", Some("mcp-tunnel")).await {
        Ok(()) => true,
        Err(AuthError::NoAuthorizationSupport) => false,
        Err(e) => return Err(crate::error::AppError::OAuth(e.to_string())),
    };

    if !has_oauth {
        return Err(crate::error::AppError::OAuth("Server does not support OAuth".to_string()));
    }

    let store = crate::mcp::oauth::FileCredentialStore::new(name);

    // Check if we already have valid credentials
    if let Some(token) = store.load().await? {
        // Check expiration using oauth2 TokenResponse trait
        use oauth2::TokenResponse;
        let is_expired = match token.expires_in() {
            Some(_) => {
                // We don't store issue time, so we can't reliably check expiration.
                // For now, assume the token is valid and let the server tell us if not.
                false
            }
            None => false,
        };
        if !is_expired {
            return Ok(());
        }
    }

    let token_response = crate::mcp::oauth::run_pkce_flow(url).await?;
    store.save(&token_response).await?;
    Ok(())
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

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    use ratatui::layout::{Constraint, Direction, Layout};
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
