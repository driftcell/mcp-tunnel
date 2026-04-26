use chrono::Timelike;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::{App, Tab};

fn truncate_json(value: &serde_json::Value, max_len: usize) -> String {
    let s = serde_json::to_string(value).unwrap_or_else(|_| "<?>".to_string());
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s
    }
}

pub fn render_audit_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_active = app.current_tab == Tab::AuditLog;

    let lines: Vec<Line> = if app.audit_logs.is_empty() {
        vec![Line::from("No audit logs yet. Start the server to see logs.")]
    } else {
        app.audit_logs.iter().rev().skip(app.audit_scroll).take(area.height as usize - 2)
            .map(|log| {
                let direction = match log.direction {
                    crate::server::audit::AuditDirection::Call => "[CALL]",
                    crate::server::audit::AuditDirection::Response => "[RESP]",
                    crate::server::audit::AuditDirection::List => "[LIST]",
                    crate::server::audit::AuditDirection::Error => "[ERR]",
                };
                let ts = log.timestamp;
                let ts_str = format!("{:02}:{:02}:{:02}", ts.hour(), ts.minute(), ts.second());
                let tool_str = log.tool.as_deref().unwrap_or("-");
                let error_str = log.error.as_deref().unwrap_or("");
                let mut parts = vec![format!("[{}] {} {} → {}", ts_str, direction, log.upstream, tool_str)];
                if let Some(ref args) = log.args {
                    parts.push(format!("| args: {}", truncate_json(args, 30)));
                }
                if let Some(ref result) = log.result {
                    parts.push(format!("| result: {}", truncate_json(result, 30)));
                }
                if error_str.is_empty() {
                    parts.push(format!("| {}ms", log.duration_ms));
                } else {
                    parts.push(format!("| ERR: {}", error_str));
                }
                Line::from(parts.join(" "))
            })
            .collect()
    };

    let paragraph = Paragraph::new(lines)
        .block(Block::default()
            .title(format!("Audit Log ({} entries)", app.audit_logs.len()))
            .borders(Borders::ALL)
            .border_style(if is_active { Style::default().fg(Color::Cyan) } else { Style::default() }));

    frame.render_widget(paragraph, area);
}
