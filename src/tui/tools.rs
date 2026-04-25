use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};
use crate::app::{App, Tab};

pub fn render_tools(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_active = app.current_tab == Tab::Tools;
    let server_name = app.tools_for_server.clone().unwrap_or_default();

    let items: Vec<ListItem> = if let Some(cache) = app.config.tool_cache.iter().find(|c| c.server == server_name) {
        cache.tools.iter().enumerate().map(|(i, tool)| {
            let icon = if tool.enabled { "[x]" } else { "[ ]" };
            let style = if i == app.selected_tool && is_active {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if !tool.enabled {
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
            .border_style(if is_active { Style::default().fg(Color::Cyan) } else { Style::default() }));

    frame.render_stateful_widget(list, area, &mut app.tool_list_state);
}
