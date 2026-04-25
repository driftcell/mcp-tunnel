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
            .border_style(if is_active && !app.show_tools {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            }),
    );

    frame.render_stateful_widget(list, list_area, &mut app.server_list_state);

    // 渲染详情面板（工具列表或服务器详情）
    if app.show_tools {
        render_tools_detail(frame, app, detail_area);
    } else {
        render_server_detail(frame, app, detail_area);
    }
}

fn render_server_detail(frame: &mut Frame, app: &App, area: Rect) {
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
    frame.render_widget(detail, area);
}

fn render_tools_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let server_name = app.tools_for_server.clone().unwrap_or_default();

    let server_disabled_tools: Option<&std::collections::BTreeSet<String>> = app.config.servers.iter()
        .find(|s| s.name == server_name)
        .map(|s| &s.disabled_tools);

    let items: Vec<ListItem> = if let Some(cache) = app.tool_cache.iter().find(|c| c.server == server_name) {
        cache.tools.iter().enumerate().map(|(i, tool)| {
            let disabled = server_disabled_tools
                .map(|set| set.contains(&tool.name))
                .unwrap_or(false);
            let icon = if !disabled { "[x]" } else { "[ ]" };
            let style = if i == app.selected_tool {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if disabled {
                Style::default().fg(Color::Gray)
            } else {
                Style::default()
            };
            let desc = if tool.description.is_empty() {
                String::new()
            } else {
                let brief: String = tool.description.chars().take(50).collect();
                if tool.description.len() > 50 {
                    format!(" - {}...", brief)
                } else {
                    format!(" - {}", brief)
                }
            };
            ListItem::new(format!("{} {}{}", icon, tool.name, desc))
                .style(style)
        }).collect()
    } else {
        vec![ListItem::new("No tools cached for this server.")]
    };

    let list = List::new(items)
        .block(Block::default()
            .title(format!("Tools - {}", server_name))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)));

    frame.render_stateful_widget(list, area, &mut app.tool_list_state);
}

fn render_add_dialog(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let popup_area = crate::tui::layout::centered_rect(50, 40, area);

    // 清除背景
    frame.render_widget(Clear, popup_area);

    let (title, hint) = if app.is_edit_mode {
        let t = match app.add_dialog_type {
            crate::app::AddDialogType::Http => "Edit HTTP Server",
            crate::app::AddDialogType::Stdio => "Edit stdio Server",
        };
        (t, "Enter: confirm | Esc: cancel | Tab: next field")
    } else {
        let t = match app.add_dialog_type {
            crate::app::AddDialogType::Http => "Add HTTP Server",
            crate::app::AddDialogType::Stdio => "Add stdio Server",
        };
        (t, "Enter: confirm | Esc: cancel | Tab: next field / switch type")
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

