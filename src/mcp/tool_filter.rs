/// 对工具名添加前缀以避免冲突：upstream_name__tool_name
pub fn prefix_tool_name(upstream_name: &str, tool_name: &str) -> String {
    format!("{}__{}", upstream_name, tool_name)
}

/// 从带前缀的工具名解析出上游名和原始工具名
/// 返回 (upstream_name, original_tool_name)
pub fn parse_tool_name(prefixed: &str) -> Option<(&str, &str)> {
    prefixed.split_once("__")
}
