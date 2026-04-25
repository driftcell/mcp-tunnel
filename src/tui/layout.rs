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
