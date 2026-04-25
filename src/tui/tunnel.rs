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
            Constraint::Length(12), // Status card
            Constraint::Min(6),     // Info / endpoints
        ])
        .spacing(1)
        .split(area);

    render_status_card(frame, app, chunks[0], is_active);
    render_info_panel(frame, app, chunks[1], is_active);
}

fn render_status_card(frame: &mut Frame, app: &App, area: Rect, is_active: bool) {
    let (status_label, status_color, status_icon) = match app.quick_tunnel.as_ref() {
        Some(qt) if qt.is_running() => ("Running", Color::Green, "●"),
        _ => ("Stopped", Color::DarkGray, "○"),
    };

    let tunnel_mode_label = match app.config.tunnel.mode {
        crate::config::TunnelMode::Disabled => "Disabled",
        crate::config::TunnelMode::Quick => "Quick (TryCloudflare)",
        crate::config::TunnelMode::Named => "Named",
    };

    let tunnel_mode_color = match app.config.tunnel.mode {
        crate::config::TunnelMode::Disabled => Color::DarkGray,
        crate::config::TunnelMode::Quick => Color::Cyan,
        crate::config::TunnelMode::Named => Color::Magenta,
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

    let url_color = if app.quick_tunnel_running && app.serve_running {
        Color::Green
    } else if app.quick_tunnel_running {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    lines.push(Line::from(vec![
        Span::styled("  URL     ", Style::default().fg(Color::DarkGray)),
        Span::styled(url, Style::default().fg(url_color)),
    ]));

    lines.push(Line::from(""));

    // Quick hints
    let hints = if app.quick_tunnel_running {
        "  'c' = copy URL  |  'o' = open in browser  |  't' = stop"
    } else {
        "  't' = start tunnel"
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

    // Named tunnel config
    lines.push(Line::from(vec![
        Span::styled(
            "Named Tunnel",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    let name_display = app
        .config
        .tunnel
        .name
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("(not configured)");
    lines.push(Line::from(vec![
        Span::styled("  Name ", Style::default().fg(Color::DarkGray)),
        Span::styled(name_display, Style::default().fg(Color::White)),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Named tunnels are managed via CLI:",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  mcp-tunnel tunnel run <name>",
        Style::default().fg(Color::Cyan),
    )));

    // Divider
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(4) as usize),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

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

    lines.push(Line::from(vec![
        Span::styled("  MCP    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("http://127.0.0.1:3000{}", MCP_PATH),
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
