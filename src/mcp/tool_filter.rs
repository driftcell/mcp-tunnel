/// The delimiter used to separate upstream name from tool name.
const TOOL_NAME_DELIMITER: &str = "__";

/// Prefix tool names to avoid collisions: upstream_name__tool_name
/// Panics if either name already contains the delimiter (would cause ambiguous parsing).
pub fn prefix_tool_name(upstream_name: &str, tool_name: &str) -> String {
    if upstream_name.contains(TOOL_NAME_DELIMITER) {
        panic!(
            "upstream name '{}' contains reserved delimiter '{}'",
            upstream_name, TOOL_NAME_DELIMITER
        );
    }
    if tool_name.contains(TOOL_NAME_DELIMITER) {
        panic!(
            "tool name '{}' contains reserved delimiter '{}'",
            tool_name, TOOL_NAME_DELIMITER
        );
    }
    format!("{}{}{}", upstream_name, TOOL_NAME_DELIMITER, tool_name)
}

/// Parse the upstream name and original tool name from a prefixed tool name
/// Returns (upstream_name, original_tool_name)
pub fn parse_tool_name(prefixed: &str) -> Option<(&str, &str)> {
    prefixed.split_once(TOOL_NAME_DELIMITER)
}
