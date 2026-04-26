use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Main layout: top title bar + middle content area + bottom status bar
pub fn main_layout(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Min(3),    // Content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    (chunks[0], chunks[1], chunks[2])
}

/// Content layout: sidebar tabs + main content area
pub fn content_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16), // Sidebar
            Constraint::Min(20),    // Main content
        ])
        .split(area);

    (chunks[0], chunks[1])
}

/// Inner main content layout (for Servers tab)
pub fn detail_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30), // List
            Constraint::Percentage(70), // Detail
        ])
        .split(area);

    (chunks[0], chunks[1])
}

/// Compute a centered rectangle (cropped by percentage)
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
