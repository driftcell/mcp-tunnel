use crate::app::{App, Tab};
use crate::constants::MCP_PATH;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

pub fn render_tunnel(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_active = app.current_tab == Tab::Tunnel;

    // ── Layout: two main sections ──
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(14), // Status card
            Constraint::Min(6),     // Info / endpoints
        ])
        .spacing(1)
        .split(area);

    render_status_card(frame, app, chunks[0], is_active);
    render_info_panel(frame, app, chunks[1], is_active);
}

fn render_status_card(frame: &mut Frame, app: &App, area: Rect, is_active: bool) {
    let (status_label, status_color, status_icon) = if app.is_tunnel_running() {
        ("Running", Color::Green, "●")
    } else {
        ("Stopped", Color::DarkGray, "○")
    };

    let tunnel_mode_label = match app.config.tunnel.mode {
        crate::config::TunnelMode::Disabled => "Disabled",
        crate::config::TunnelMode::Quick => "Quick (TryCloudflare)",
    };

    let tunnel_mode_color = match app.config.tunnel.mode {
        crate::config::TunnelMode::Disabled => Color::DarkGray,
        crate::config::TunnelMode::Quick => Color::Cyan,
    };

    let serve_label = if app.serve_running {
        ("Running", Color::Green)
    } else {
        ("Stopped", Color::DarkGray)
    };

    let mut lines: Vec<Line> = Vec::new();

    // Row 1: Tunnel status
    lines.push(Line::from(vec![
        Span::styled("  Status  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} {}", status_icon, status_label),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Row 2: Mode
    lines.push(Line::from(vec![
        Span::styled("  Mode    ", Style::default().fg(Color::DarkGray)),
        Span::styled(tunnel_mode_label, Style::default().fg(tunnel_mode_color)),
    ]));

    // Row 3: Serve dependency
    lines.push(Line::from(vec![
        Span::styled("  Serve   ", Style::default().fg(Color::DarkGray)),
        Span::styled(serve_label.0, Style::default().fg(serve_label.1)),
    ]));

    lines.push(Line::from(""));

    // URL block (prominent)
    let url = app
        .tunnel_url
        .as_ref()
        .map(|u| format!("{}{}", u, MCP_PATH))
        .unwrap_or_else(|| "Not available".to_string());

    let url_color = if app.is_tunnel_running() && app.serve_running {
        Color::Green
    } else if app.is_tunnel_running() {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    lines.push(Line::from(vec![
        Span::styled("  URL     ", Style::default().fg(Color::DarkGray)),
        Span::styled(url, Style::default().fg(url_color)),
    ]));

    lines.push(Line::from(""));

    // Token block
    let token_display = app
        .config
        .tunnel
        .token
        .as_ref()
        .map(|t| {
            if t.len() > 12 {
                format!("{}...{}", &t[..6], &t[t.len() - 6..])
            } else {
                t.clone()
            }
        })
        .unwrap_or_else(|| "Not set".to_string());
    lines.push(Line::from(vec![
        Span::styled("  Token   ", Style::default().fg(Color::DarkGray)),
        Span::styled(token_display, Style::default().fg(Color::Gray)),
    ]));

    // Quick hints
    let hints = if app.is_tunnel_running() {
        "  'c' = copy URL  |  'o' = open browser  |  't' = stop  |  'e' = edit bind  |  'C' = copy token"
    } else {
        "  't' = start tunnel  |  'e' = edit bind address  |  'C' = copy token"
    };
    lines.push(Line::from(Span::styled(
        hints,
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));

    let block = Block::default()
        .title(" Quick Tunnel ")
        .borders(Borders::ALL)
        .border_style(if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .padding(Padding::uniform(1));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_info_panel(frame: &mut Frame, app: &App, area: Rect, is_active: bool) {
    let mut lines: Vec<Line> = Vec::new();

    // Local endpoint info
    lines.push(Line::from(vec![
        Span::styled(
            "Local Endpoint",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    let base_url = app.config.tunnel.base_url();

    lines.push(Line::from(vec![
        Span::styled("  Bind   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &app.config.tunnel.bind_addr,
            Style::default().fg(Color::Gray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  MCP    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}{}", base_url, MCP_PATH),
            Style::default().fg(Color::Gray),
        ),
    ]));

    let block = Block::default()
        .title(" Tunnel Info ")
        .borders(Borders::ALL)
        .border_style(if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .padding(Padding::uniform(1));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
