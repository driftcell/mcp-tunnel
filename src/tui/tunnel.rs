use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::{App, Tab};

pub fn render_tunnel(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_active = app.current_tab == Tab::Tunnel;

    let tunnel_mode = match app.config.tunnel.mode {
        crate::config::TunnelMode::Disabled => "Disabled",
        crate::config::TunnelMode::Quick => "Quick (TryCloudflare)",
        crate::config::TunnelMode::Named => "Named",
    };

    let (status, url_text) = match app.quick_tunnel.as_ref() {
        Some(qt) if qt.is_running() => ("Running", qt.url().unwrap_or("N/A")),
        _ => ("Stopped", "N/A"),
    };

    let text = format!(
        "Tunnel Mode: {}\n\
         Status: {}\n\
         URL: {}\n\
         \n\
         Config:\n\
         mode = {}\n\
         name = {:?}\n\n\
         Press 't' to toggle tunnel",
        tunnel_mode, status, url_text, tunnel_mode, app.config.tunnel.name
    );

    let paragraph = Paragraph::new(text)
        .block(Block::default()
            .title("Tunnel")
            .borders(Borders::ALL)
            .border_style(if is_active { Style::default().fg(Color::Cyan) } else { Style::default() }));

    frame.render_widget(paragraph, area);
}
