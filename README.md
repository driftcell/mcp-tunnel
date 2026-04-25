# MCP Tunnel

Aggregate multiple upstream MCP (Model Context Protocol) services into a single unified server with a terminal UI, tool filtering, OAuth support, and Cloudflare tunnel integration.

## Features

- **Multi-upstream aggregation** — Combine HTTP and stdio MCP services into one endpoint
- **Tool namespace isolation** — Upstream tools are prefixed as `upstream_name__tool_name`
- **Tool filtering** — Enable or disable individual tools per upstream server
- **OAuth 2.0 PKCE** — Automatic token acquisition and storage for authenticated HTTP upstreams
- **Cloudflare Tunnel** — Quick (temporary) tunnel mode
- **Audit logging** — Track every tool list, call, response, and error
- **Terminal UI** — Interactive ratatui-based management interface
- **Streamable HTTP transport** — Uses rmcp 1.5 with server-side HTTP

## Installation

```bash
cargo build --release
```

The binary is named `mt`.

## Usage

### Interactive TUI (default)

```bash
./mt
```

Navigate with `Tab` / `Shift+Tab` to switch between Servers, Tools, Tunnel, and Audit Log tabs.

### Start the aggregated server (headless)

```bash
./mt serve
```

The server binds to `127.0.0.1:3000` and exposes the MCP endpoint at `/mcp`.

### CLI commands

```bash
# Add an HTTP upstream
./mt add notion https://mcp.notion.com/mcp

# Add a stdio upstream
./mt add-stdio filesystem npx -y @modelcontextprotocol/server-filesystem /path/to/dir

# Remove an upstream
./mt remove notion

# Clear saved OAuth token
./mt clear-token notion
```

## Configuration

Configuration is stored in `config.toml` (override with `-c /path/to/config.toml`):

```toml
[[servers]]
name = "notion"
enabled_tools = []
disabled_tools = ["notion-get-users", "notion-get-teams"]

[servers.type]
type = "http"
url = "https://mcp.notion.com/mcp"

[tunnel]
mode = "disabled"  # "disabled" | "quick"
```

### Server types

| Type | Description |
|------|-------------|
| `http` | Connect to a remote MCP server over Streamable HTTP |
| `stdio` | Spawn a local command and communicate over stdin/stdout |

### Tool filtering rules

- `disabled_tools` — Explicitly disabled tools (takes precedence)
- `enabled_tools` — If non-empty, only these tools are allowed; if empty, all tools are allowed except those in `disabled_tools`

## Dependencies

- [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 1.5 — Rust MCP SDK
- [ratatui](https://github.com/ratatui-org/ratatui) — Terminal UI
- [axum](https://github.com/tokio-rs/axum) — HTTP server
- [tokio](https://github.com/tokio-rs/tokio) — Async runtime
- [oauth2](https://github.com/ramosbugs/oauth2-rs) — OAuth 2.0 PKCE flow

## License

MIT
