use crate::app::{App, Tab};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

pub fn render_servers(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.show_add_dialog {
        render_add_dialog(frame, app);
        return;
    }

    let is_active = app.current_tab == Tab::Servers;
    let (list_area, detail_area) = crate::tui::layout::detail_layout(area);

    // 渲染服务列表
    let items: Vec<ListItem> = app
        .config
        .servers
        .iter()
        .enumerate()
        .map(|(i, server)| {
            let style = if i == app.selected_server && is_active {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.selected_server { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, server.name)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Servers")
            .borders(Borders::ALL)
            .border_style(if is_active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            }),
    );

    frame.render_stateful_widget(list, list_area, &mut app.server_list_state);

    // 渲染详情面板
    let detail_text = if let Some(server) = app.selected_server_config() {
        let ty_str = match &server.ty {
            crate::config::UpstreamType::Http { url } => format!("Type: HTTP\nURL: {}", url),
            crate::config::UpstreamType::Stdio { command, args } => {
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", args.join(" "))
                };
                format!("Type: stdio\nCommand: {}{}", command, args_str)
            }
        };
        let tools_count = app
            .config
            .tool_cache
            .iter()
            .find(|c| c.server == server.name)
            .map(|c| c.tools.len())
            .unwrap_or(0);
        let oauth_status = {
            let token_path = dirs::data_local_dir()
                .map(|d| d.join("mcp-tunnel").join("oauth").join(format!("{}.json", server.name)));
            match token_path {
                Some(p) if p.exists() => "OAuth: Token stored",
                _ => "OAuth: No token",
            }
        };
        format!(
            "Server: {}\n{}\n{}\n\nTools: {}\nDisabled: {}\n",
            server.name,
            ty_str,
            oauth_status,
            tools_count,
            server.disabled_tools.len()
        )
    } else {
        "No servers configured.\nPress 'a' to add one.".to_string()
    };

    let detail_title = if app.serve_running {
        "Details ● SERVE ON"
    } else {
        "Details"
    };
    let detail_block = Block::default()
        .title(detail_title)
        .borders(Borders::ALL)
        .border_style(if app.serve_running {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        });
    let detail = Paragraph::new(detail_text).block(detail_block);
    frame.render_widget(detail, detail_area);
}

fn render_add_dialog(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let popup_area = crate::tui::layout::centered_rect(50, 40, area);

    // 清除背景
    frame.render_widget(Clear, popup_area);

    let title = match app.add_dialog_type {
        crate::app::AddDialogType::Http => "Add HTTP Server",
        crate::app::AddDialogType::Stdio => "Add stdio Server",
    };

    let type_label = match app.add_dialog_type {
        crate::app::AddDialogType::Http => "Type: HTTP",
        crate::app::AddDialogType::Stdio => "Type: stdio",
    };

    let value_label = match app.add_dialog_type {
        crate::app::AddDialogType::Http => "URL:",
        crate::app::AddDialogType::Stdio => "Command:",
    };

    let name_field = format!("Name: {}", app.add_dialog_fields.get(0).map(|s| s.as_str()).unwrap_or(""));
    let value_field = format!("{} {}", value_label, app.add_dialog_fields.get(1).map(|s| s.as_str()).unwrap_or(""));

    let hint = "Enter: confirm | Esc: cancel | Tab: next field / switch type";

    let content = format!(
        "{}\n\n{}\n{}\n\n{}",
        type_label,
        if app.add_dialog_focus == 0 {
            format!("{} <--", name_field)
        } else {
            name_field
        },
        if app.add_dialog_focus == 1 {
            format!("{} <--", value_field)
        } else {
            value_field
        },
        hint
    );

    let paragraph = Paragraph::new(content).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(paragraph, popup_area);
}

