use crate::app::{App, Tab};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

pub fn render_servers(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.show_add_dialog {
        render_add_dialog(frame, app);
        return;
    }

    let is_active = app.current_tab == Tab::Servers;
    let (list_area, detail_area) = crate::tui::layout::detail_layout(area);

    // Render server list
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

    // Render detail panel (tool list or server details)
    if app.show_tools {
        render_tools_detail(frame, app, detail_area);
    } else {
        render_server_detail(frame, app, detail_area);
    }
}

fn render_server_detail(frame: &mut Frame, app: &App, area: Rect) {
    let is_active = app.current_tab == Tab::Servers;

    let lines: Vec<Line> = if let Some(server) = app.selected_server_config() {
        let mut lines = Vec::new();

        // ── Server info header ──
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                &server.name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

        // Type info
        match &server.ty {
            crate::config::UpstreamType::Http { url } => {
                lines.push(Line::from(vec![
                    Span::styled("  Type   ", Style::default().fg(Color::DarkGray)),
                    Span::styled("HTTP", Style::default().fg(Color::White)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  URL    ", Style::default().fg(Color::DarkGray)),
                    Span::styled(url.as_str(), Style::default().fg(Color::Cyan)),
                ]));
            }
            crate::config::UpstreamType::Stdio { command, args } => {
                lines.push(Line::from(vec![
                    Span::styled("  Type    ", Style::default().fg(Color::DarkGray)),
                    Span::styled("stdio", Style::default().fg(Color::White)),
                ]));
                let cmd = if args.is_empty() {
                    command.clone()
                } else {
                    format!("{} {}", command, args.join(" "))
                };
                lines.push(Line::from(vec![
                    Span::styled("  Command ", Style::default().fg(Color::DarkGray)),
                    Span::styled(cmd, Style::default().fg(Color::White)),
                ]));
            }
        }

        // OAuth status
        let oauth_status = {
            let token_path = dirs::data_local_dir()
                .map(|d| d.join("mcp-tunnel").join("oauth").join(format!("{}.json", server.name)));
            match token_path {
                Some(p) if p.exists() => ("Token stored", Color::Green),
                _ => ("No token", Color::DarkGray),
            }
        };
        lines.push(Line::from(vec![
            Span::styled("  OAuth  ", Style::default().fg(Color::DarkGray)),
            Span::styled(oauth_status.0, Style::default().fg(oauth_status.1)),
        ]));
        lines.push(Line::from(""));

        // ── Tools preview ──
        let cache = app.tool_cache.iter().find(|c| c.server == server.name);
        let tools_count = cache.map(|c| c.tools.len()).unwrap_or(0);

        let tool_summary = if tools_count == 0 {
            "No tools discovered".to_string()
        } else {
            let disabled = server.disabled_tools.len();
            let active = tools_count - disabled;
            format!("{} tools · {} active · {} disabled", tools_count, active, disabled)
        };
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Tools",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", tool_summary),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Show first few tool names as preview
        if let Some(cache) = cache {
            let preview_count = cache.tools.len().min(5);
            for tool in cache.tools.iter().take(preview_count) {
                let is_disabled = server.disabled_tools.contains(&tool.name);
                let icon = if is_disabled { "○" } else { "●" };
                let icon_color = if is_disabled { Color::DarkGray } else { Color::Green };
                let name_color = if is_disabled { Color::DarkGray } else { Color::Gray };
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(icon, Style::default().fg(icon_color)),
                    Span::styled(" ", Style::default()),
                    Span::styled(&tool.name, Style::default().fg(name_color)),
                ]));
            }
            if cache.tools.len() > preview_count {
                let remaining = cache.tools.len() - preview_count;
                lines.push(Line::from(Span::styled(
                    format!("    ... and {} more", remaining),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                )));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press Enter to manage tools",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));

        lines
    } else {
        vec![
            Line::from("No servers configured."),
            Line::from(""),
            Line::from("Press 'a' to add one."),
        ]
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
        } else if is_active && !app.show_tools {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let detail = Paragraph::new(lines).block(detail_block);
    frame.render_widget(detail, area);
}

fn render_tools_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let server_name = app.tools_for_server.clone().unwrap_or_default();

    let server_disabled_tools: Option<&std::collections::BTreeSet<String>> = app
        .config
        .servers
        .iter()
        .find(|s| s.name == server_name)
        .map(|s| &s.disabled_tools);

    let inner_width = area.width.saturating_sub(2);
    let inner_height = area.height.saturating_sub(2);

    // Find tools for this server
    let tools = match app.tool_cache.iter().find(|c| c.server == server_name) {
        Some(cache) if !cache.tools.is_empty() => &cache.tools[..],
        _ => {
            let content = if app.tool_cache.iter().any(|c| c.server == server_name) {
                "No tools discovered for this server.\nPress 'o' to authenticate and discover tools."
            } else {
                "Loading tools...\nIf this persists, press 'o' to authenticate."
            };
            let msg = Paragraph::new(content)
                .block(
                    Block::default()
                        .title(format!("Tools — {}", server_name))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            frame.render_widget(msg, area);
            return;
        }
    };

    let total = tools.len();
    let disabled_count = server_disabled_tools
        .map(|set| tools.iter().filter(|t| set.contains(&t.name)).count())
        .unwrap_or(0);
    let enabled_count = total - disabled_count;

    let folded = app.tools_folded;

    // Folded: name line + separator = 2 lines per tool
    // Unfolded: name line + desc line + separator = 3 lines per tool
    let lines_per_tool: usize = if folded { 2 } else { 3 };
    let header_lines: usize = 2; // summary + blank line
    let max_visible =
        (inner_height as usize).saturating_sub(header_lines) / lines_per_tool;

    // Scroll so selected tool stays in view
    let scroll = if app.selected_tool >= max_visible {
        app.selected_tool.saturating_sub(max_visible - 1)
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();

    // ── Summary bar ──
    let fold_icon = if folded { "▸" } else { "▾" };
    let summary = format!(
        "  {} {} tools · {} active · {} disabled",
        fold_icon, total, enabled_count, disabled_count
    );
    lines.push(Line::from(Span::styled(
        summary,
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    // ── Tool cards ──
    for (idx, tool) in tools.iter().enumerate().skip(scroll).take(max_visible) {
        let is_selected = idx == app.selected_tool;
        let is_disabled = server_disabled_tools
            .map(|set| set.contains(&tool.name))
            .unwrap_or(false);

        let border_char = if is_disabled { "│" } else { "┃" };
        let select_prefix = if is_selected { "▶ " } else { "  " };

        let (name_fg, border_fg, desc_fg, indicator_text, indicator_fg) = if is_disabled {
            let nf = if is_selected {
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let bf = Color::DarkGray;
            let df = Color::DarkGray;
            let it = "○ disabled";
            let ig = Color::DarkGray;
            (nf, bf, df, it, ig)
        } else {
            let nf = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let bf = if is_selected { Color::Yellow } else { Color::Green };
            let df = Color::Gray;
            let it = "● active";
            let ig = Color::Green;
            (nf, bf, df, it, ig)
        };

        // Name line: ▶ ┃ tool-name  ● active
        let name_part = format!("{}{} {}", select_prefix, border_char, tool.name);
        let indicator = format!("  {}", indicator_text);

        // Right-align indicator by padding with spaces
        let content_width = name_part.len() + indicator.len();
        let pad = (inner_width as usize).saturating_sub(content_width);
        let padding = " ".repeat(pad);

        lines.push(Line::from(vec![
            Span::styled(name_part, name_fg),
            Span::styled(padding, Style::default()),
            Span::styled(indicator, Style::default().fg(indicator_fg)),
        ]));

        // Description line (only when unfolded)
        if !folded {
            let desc_prefix = format!("  {} ", border_char);
            let desc_span = if tool.description.is_empty() {
                Span::styled(
                    "(no description)",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )
            } else {
                let max_desc = (inner_width as usize).saturating_sub(4);
                let desc_text: String = if tool.description.chars().count() > max_desc {
                    let truncated: String = tool
                        .description
                        .chars()
                        .take(max_desc.saturating_sub(3))
                        .collect();
                    format!("{}...", truncated)
                } else {
                    tool.description.clone()
                };
                Span::styled(desc_text, Style::default().fg(desc_fg))
            };
            lines.push(Line::from(vec![
                Span::styled(desc_prefix, Style::default().fg(border_fg)),
                desc_span,
            ]));
        }

        // Separator line between cards
        lines.push(Line::from(""));
    }

    let title = format!("Tools — {}{}", server_name, if folded { " [folded]" } else { "" });
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(paragraph, area);
}

fn render_add_dialog(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let popup_area = crate::tui::layout::centered_rect(50, 40, area);

    // Clear background
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

    let name_field = format!("Name: {}", app.add_dialog_fields.first().map(|s| s.as_str()).unwrap_or(""));
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

