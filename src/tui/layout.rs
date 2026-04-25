use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// 主布局：顶部标题栏 + 中间内容区 + 底部状态栏
pub fn main_layout(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 标题
            Constraint::Min(3),    // 内容
            Constraint::Length(1), // 状态栏
        ])
        .split(area);

    (chunks[0], chunks[1], chunks[2])
}

/// 内容布局：侧边栏标签 + 主内容区
pub fn content_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16), // 侧边栏
            Constraint::Min(20),    // 主内容
        ])
        .split(area);

    (chunks[0], chunks[1])
}

/// 主内容内部布局（用于 Servers tab）
pub fn detail_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30), // 列表
            Constraint::Percentage(70), // 详情
        ])
        .split(area);

    (chunks[0], chunks[1])
}

/// 计算居中矩形区域（按百分比裁剪）
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
