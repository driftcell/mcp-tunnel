/// Default bind address for the MCP server.
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:3000";

/// Default base URL for the local MCP server (used by TUI / tunnel).
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:3000";

/// Path segment where the MCP streamable HTTP endpoint is mounted.
pub const MCP_PATH: &str = "/mcp";

/// Local OAuth callback listen address (host:port).
pub const OAUTH_CALLBACK_ADDR: &str = "127.0.0.1:9876";

/// Full OAuth callback URL used as redirect_uri.
pub const OAUTH_CALLBACK_URL: &str = "http://127.0.0.1:9876/callback";

/// TUI tick rate in milliseconds.
pub const TICK_RATE_MS: u64 = 250;

/// Duration in seconds that TUI status messages remain visible.
pub const TUI_MESSAGE_DURATION_SECS: u64 = 3;
