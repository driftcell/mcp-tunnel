# MCP Tunnel Code Review

**Project:** mcp-tunnel  
**Version:** 0.1.0  
**Edition:** 2024  
**Total Lines of Code:** ~2,693  
**Review Date:** 2026-04-25  

---

## Executive Summary

This is a Rust-based MCP (Model Context Protocol) tunnel/proxy application that aggregates multiple upstream MCP services into a single server. It provides a TUI (Terminal User Interface), CLI commands, OAuth authentication, Cloudflare tunnel integration, and audit logging. The codebase is relatively young and functional but contains several critical issues, particularly around error handling, resource management, security, and code quality.

---

## Critical Issues (bugs, panics, security)

### 1. PANIC: Unsupported Platform in `tunnel/binary.rs`
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tunnel/binary.rs`
- **Line:** 91
- **Description:** The `get_download_url()` function panics with `panic!("Unsupported platform: {}-{}", os, arch)` for any platform other than macOS aarch64/x86_64 or Linux x86_64/aarch64. This will crash the application on Windows, FreeBSD, or other platforms instead of gracefully returning an error.
- **Suggested Fix:** Return a `Result` with a descriptive error instead of panicking:
  ```rust
  fn get_download_url() -> Result<(&'static str, bool)> {
      match (std::env::consts::OS, std::env::consts::ARCH) {
          ("macos", "aarch64") => Ok(("...", true)),
          // ... other platforms
          _ => Err(AppError::Tunnel(format!("Unsupported platform: {}-{}", os, arch))),
      }
  }
  ```

### 2. SECURITY: OAuth Tokens Stored in Plaintext JSON
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/oauth/store.rs`
- **Lines:** 33-42
- **Description:** OAuth tokens are stored as plaintext JSON files in `~/.local/share/mcp-tunnel/oauth/{server_name}.json` (or equivalent). There is no encryption, file permission restriction (beyond default), or keychain integration. Tokens could be read by any process with user-level access.
- **Suggested Fix:** 
  - Use platform-specific credential stores (macOS Keychain, Windows Credential Manager, Linux libsecret)
  - Or at minimum, set restrictive file permissions (0o600) on the token files
  - Consider encrypting with a user-derived key

### 3. SECURITY: Missing TLS Certificate Validation Control
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 247-261
- **Description:** The `build_reqwest_client()` function creates a default `reqwest::Client` without any TLS configuration. While reqwest defaults are generally secure, there's no way for users to configure custom CA certificates, client certificates, or TLS settings for corporate environments or self-signed certificates.
- **Suggested Fix:** Expose TLS configuration options in `ServerConfig` (ca_cert, client_cert, insecure_skip_verify for development).

### 4. SECURITY: Hardcoded Localhost Bind Address
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/server/router.rs`
- **Line:** 21
- **Description:** The server binds to `127.0.0.1:3000` with no configuration option. While localhost binding is secure by default, users cannot configure it to bind to specific interfaces, use different ports, or expose it on the network.
- **Suggested Fix:** Make bind address configurable via `Config` or CLI arguments.

### 5. SECURITY: OAuth Callback Server Missing CSRF Validation
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/oauth/flow.rs`
- **Lines:** 54-76
- **Description:** The `wait_for_callback()` function starts a TCP listener and accepts the first connection that provides a valid code and state. However, the CSRF token (state parameter) is extracted but never validated against the value generated during `start_authorization()`. This makes the OAuth flow vulnerable to CSRF attacks.
- **Suggested Fix:** Store the expected CSRF token from `state.get_csrf_token()` (or equivalent) and validate it against the received `state` parameter in `handle_callback_request()`.

### 6. BUG: Quick Tunnel Child Process Leak on Timeout
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tunnel/quick.rs`
- **Lines:** 47-66
- **Description:** When the tunnel URL extraction times out, the code calls `child.kill().await` but does not wait for the process to actually exit. The child process handle is then dropped without proper cleanup, potentially creating zombie processes.
- **Suggested Fix:** After `kill()`, call `child.wait().await` to reap the process. Also handle the case where `kill()` itself fails.

### 7. BUG: Missing OAuth Token Expiration Check
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 101-104
- **Description:** The code comment explicitly states: "We don't track token issue time, so we use stored tokens optimistically." This means expired tokens will be used until the server returns 401, causing unnecessary failed requests.
- **Suggested Fix:** Store token issue time alongside the token and check expiration using `token.expires_in()` before using it. Implement automatic refresh if refresh tokens are available.

### 8. BUG: HTTP Callback Response Not Flushed
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/oauth/flow.rs`
- **Line:** 143
- **Description:** The HTTP response is written with `stream.write_all()` but the stream is not flushed or shut down. The browser may not receive the complete response, leaving the connection hanging.
- **Suggested Fix:** Call `stream.flush().await` and `stream.shutdown().await` after writing the response.

---

## Code Quality Issues (refactoring opportunities, dead code, duplication)

### 9. DEAD CODE: `#[allow(dead_code)]` on `_service` Field
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 34-35
- **Description:** The `_service` field in `UpstreamClient` is marked with `#[allow(dead_code)]` because it's only used to keep the `RunningService` alive. This is a code smell - the field IS used (for its Drop impl), just not explicitly read.
- **Suggested Fix:** Add a comment explaining the purpose, or rename to `_keepalive` to make intent clearer. Consider using `std::mem::ManuallyDrop` if explicit lifecycle control is needed.

### 10. CODE DUPLICATION: `connect_http()` and `discover_tools()` Duplicate Connection Logic
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 207-244 (discover_tools) vs 65-132 (connect_single)
- **Description:** The `discover_tools()` function duplicates most of the connection and tool discovery logic from `connect_single()`. Both handle stdio and HTTP transports with nearly identical code.
- **Suggested Fix:** Refactor to use a shared `connect_and_discover()` helper that both functions call, or make `discover_tools()` reuse `connect_single()` and extract the tools.

### 11. CODE DUPLICATION: `start_server()` and `start_serve()` Are Nearly Identical
- **Files:** `/Users/driftcell/Projects/mcp-tunnel/src/server/router.rs` (lines 145-227) and `/Users/driftcell/Projects/mcp-tunnel/src/tui/mod.rs` (lines 422-495)
- **Description:** Both functions initialize `AggregatedClient`, connect to upstreams, create audit channels, and start an axum server with identical configuration. This violates DRY and makes maintenance difficult.
- **Suggested Fix:** Extract a shared `start_server_internal()` function that both CLI serve mode and TUI serve mode can call.

### 12. CODE SMELL: `let _ =` Pattern Used Extensively
- **Files:** Multiple files
- **Lines:** See detailed list below
- **Description:** Many operations that could fail are silently ignored with `let _ =`. This includes:
  - `app.rs:191` - `let _ = self.save_config();` (config save failures ignored)
  - `app.rs:209` - `let _ = self.save_config();` (config save failures ignored)
  - `tui/mod.rs:259` - `let _ = app.save_config();` (config save failures ignored)
  - `tui/mod.rs:287` - `let _ = store.clear().await;` (token clear failures ignored)
  - `server/audit.rs:65,85,105,123` - `let _ = self.sender.send(log).await;` (audit log drops silently)
  - `tunnel/quick.rs:62,82` - `let _ = child.kill().await;` (kill failures ignored)
  - `mcp/oauth/store.rs:35` - `let _ = tokio::fs::create_dir_all(parent).await;` (directory creation failures ignored)
  - `mcp/oauth/store.rs:47` - `let _ = tokio::fs::remove_file(&self.path).await;` (deletion failures ignored)
  - `mcp/oauth/flow.rs:29` - `let _ = open::that(&auth_url);` (browser open failures ignored)
  - `mcp/oauth/flow.rs:143` - `let _ = stream.write_all(response.as_bytes()).await;` (write failures ignored)
- **Suggested Fix:** Log warnings or errors for all ignored results. For critical operations like config saves, propagate errors to the user.

### 13. CODE SMELL: `unwrap_or_default()` on `Option<String>`
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tui/mod.rs`
- **Line:** 242
- **Description:** `t.description.unwrap_or_default().to_string()` converts `Option<String>` to `String` by using unwrap_or_default. This is unnecessary since `Option<String>` already implements `Into<String>` with `unwrap_or_default()` behavior.
- **Suggested Fix:** Use `t.description.unwrap_or_default()` directly, or `t.description.as_deref().unwrap_or("")`.

### 14. CODE SMELL: `unwrap()` in TUI Quick Tunnel Toggle
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tui/mod.rs`
- **Line:** 338
- **Description:** `let qt = app.quick_tunnel.as_mut().unwrap();` is called inside an `else` branch where `app.quick_tunnel.is_none()` was already checked in the `if` condition, so this is technically safe. However, it's fragile - if the logic changes, this could panic.
- **Suggested Fix:** Use `if let Some(qt) = app.quick_tunnel.as_mut()` instead of `unwrap()`.

### 15. DEAD CODE: `run_tunnel` Function Referenced but Never Called
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tui/mod.rs`
- **Line:** 370
- **Description:** `let _ = crate::tunnel::named::run_tunnel;` takes a reference to the function but does nothing with it. This is dead code that should be removed or properly implemented.
- **Suggested Fix:** Remove this line or implement the named tunnel run functionality properly.

### 16. INCONSISTENT ERROR HANDLING: Mixed `anyhow` and Custom Error Types
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/main.rs`
- **Lines:** 17, 77-80
- **Description:** `main()` returns `anyhow::Result<()>` but the application defines its own `AppError` enum. In some places, errors are converted to `anyhow::anyhow!()` (e.g., line 80), losing structured error information.
- **Suggested Fix:** Standardize on `AppError` throughout the application, or implement `From<AppError> for anyhow::Error` to allow seamless conversion while preserving error context.

### 17. CODE SMELL: Hardcoded Strings Scattered Throughout
- **Files:** Multiple
- **Description:** Many hardcoded strings like `"http://127.0.0.1:3000"`, `"127.0.0.1:3000"`, `"/mcp"`, `"127.0.0.1:9876"` are duplicated across files instead of being centralized constants.
- **Suggested Fix:** Centralize all hardcoded addresses, ports, and paths in a `constants` module or in the `Config` struct with defaults.

---

## Error Handling Issues (missing error propagation, unwrap/expect abuse)

### 18. MISSING ERROR HANDLING: `save_config()` Failures Silently Ignored
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/app.rs`
- **Lines:** 191, 209
- **Description:** When removing a server or toggling a tool, `self.save_config()` is called with `let _ = self.save_config();`. If the config file is read-only, on a full disk, or has permission issues, the user will not be notified that their changes were not persisted.
- **Suggested Fix:** Return `Result<()>` from these methods and propagate errors to the UI layer to display a message to the user.

### 19. MISSING ERROR HANDLING: Audit Log Channel Saturation
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/server/audit.rs`
- **Lines:** 65, 85, 105, 123
- **Description:** All audit logging methods use `let _ = self.sender.send(log).await;`. If the channel is full (capacity 1000), logs are silently dropped. This violates audit integrity requirements.
- **Suggested Fix:** Use `send().await` with proper error handling. If the channel is full, either block until space is available, or log to a fallback (stderr/file). Consider using a bounded channel with backpressure or an unbounded channel for critical audit logs.

### 20. MISSING ERROR HANDLING: `create_dir_all` Failures Ignored
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/oauth/store.rs`
- **Line:** 35
- **Description:** `let _ = tokio::fs::create_dir_all(parent).await;` silently ignores directory creation failures. If the parent directory cannot be created, the subsequent `write()` will also fail, but the error message will be confusing.
- **Suggested Fix:** Propagate the error: `tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::OAuth(format!("failed to create directory: {}", e)))?;`

### 21. MISSING ERROR HANDLING: Browser Open Failure Ignored
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/oauth/flow.rs`
- **Line:** 29
- **Description:** `let _ = open::that(&auth_url);` silently fails if the browser cannot be opened. The user will be left waiting for a callback that never comes, with no indication of what went wrong.
- **Suggested Fix:** Log a warning and print the URL to stdout so the user can manually navigate to it.

### 22. MISSING ERROR HANDLING: `stream.write_all` Failure Ignored
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/oauth/flow.rs`
- **Line:** 143
- **Description:** The HTTP response to the browser is written with `let _ = stream.write_all(response.as_bytes()).await;`. If this fails, the browser will hang waiting for a response.
- **Suggested Fix:** Handle the error, at minimum by logging it. Consider retrying or closing the connection.

### 23. MISSING ERROR HANDLING: `kill()` Failures Ignored
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tunnel/quick.rs`
- **Lines:** 62, 82
- **Description:** `let _ = child.kill().await;` ignores failures to kill the cloudflared process. If the process is already dead or cannot be killed, the error is swallowed.
- **Suggested Fix:** Log warnings on kill failure and attempt to wait for the process to exit.

### 24. MISSING ERROR HANDLING: `dirs::data_local_dir()` Fallback to Current Directory
- **Files:** Multiple (main.rs:110, oauth/store.rs:12, tunnel/binary.rs:7, tui/servers.rs:72)
- **Description:** When `dirs::data_local_dir()` returns `None`, the code falls back to the current directory (`.`). This can cause data files, logs, and OAuth tokens to be written to unexpected locations.
- **Suggested Fix:** Return an error if `dirs::data_local_dir()` is `None`, or use a well-known fallback like `~/.mcp-tunnel`.

### 25. EXPECT USAGE: `expect("failed to clone log file")`
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/main.rs`
- **Line:** 125
- **Description:** The log file clone uses `expect()`, which will panic if the file descriptor cannot be duplicated. This could happen if the process hits file descriptor limits.
- **Suggested Fix:** Return an error or handle the failure gracefully, perhaps by falling back to stderr logging.

---

## Performance Issues

### 26. PERFORMANCE: `list_tools()` Acquires Read Lock and Clones All Tools
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 135-149
- **Description:** Every `list_tools()` call acquires a `RwLock` read lock and clones every `Tool` struct (including name strings). For large tool lists, this is expensive and blocks concurrent tool calls.
- **Suggested Fix:** Cache the prefixed tool list and invalidate only when servers are added/removed. Or use `Arc<Tool>` to share tools without cloning.

### 27. PERFORMANCE: Audit Logs Buffered in Unbounded Vector
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/app.rs`
- **Line:** 47
- **Description:** `serve_audit_buffer: Arc<Mutex<Vec<AuditLog>>>` is an unbounded vector that grows indefinitely while the server runs. In a long-running process, this will consume unbounded memory.
- **Suggested Fix:** Use a bounded channel or ring buffer. If using a vector, implement a maximum size and eviction policy (e.g., keep last N entries).

### 28. PERFORMANCE: `tool_cache` Linear Search
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/app.rs`
- **Line:** 139-143
- **Description:** `current_tools_count()` performs a linear search through `tool_cache` for every UI frame render. With many servers, this is O(n) per frame.
- **Suggested Fix:** Use a `HashMap<String, ToolCache>` instead of `Vec<ToolCache>` for O(1) lookups.

### 29. PERFORMANCE: `config.servers` Linear Search for Tool Toggle
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/app.rs`
- **Lines:** 195-211
- **Description:** `toggle_selected_tool()` does nested linear searches through `tool_cache` and `config.servers`.
- **Suggested Fix:** Use `HashMap` for O(1) server lookups, or maintain indices.

### 30. PERFORMANCE: `client.list_tools().await` Called Twice on Server Start
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/server/router.rs`
- **Lines:** 168, 60
- **Description:** In `start_server()`, `client.list_tools().await` is called at line 168 for logging, and then again inside the `AggregatedServer` handler when clients request the tool list. The tools are already cached in `UpstreamClient` but the aggregation is recomputed.
- **Suggested Fix:** Cache the aggregated tool list in `AggregatedClient` and invalidate on connect/disconnect.

---

## Architecture/Design Concerns

### 31. DESIGN: `App` Struct is a God Object
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/app.rs`
- **Lines:** 20-58
- **Description:** The `App` struct contains state for all tabs (Servers, Tools, Tunnel, AuditLog), serve mode, quick tunnel, add dialog, and messages. This violates the Single Responsibility Principle and makes the struct difficult to test and maintain.
- **Suggested Fix:** Split into smaller structs: `ServerTabState`, `ToolTabState`, `TunnelTabState`, `AuditTabState`, `DialogState`, each managed independently.

### 32. DESIGN: `Config` Struct Mixed with Runtime State
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/config.rs`
- **Lines:** 7-15
- **Description:** `Config` contains both persistent configuration (`servers`, `tunnel`) and runtime cache (`tool_cache`). The `tool_cache` is serialized to config.toml, which is inappropriate - caches should not be persisted in configuration files.
- **Suggested Fix:** Separate `tool_cache` into a separate runtime-only structure, or persist it to a separate cache file.

### 33. DESIGN: `AggregatedClient` Uses `RwLock<HashMap>` Instead of Concurrent Map
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Line:** 23
- **Description:** Using `RwLock<HashMap<String, UpstreamClient>>` means all tool listings and calls acquire a lock, even though the map is mostly read-only after initialization.
- **Suggested Fix:** Use `dashmap::DashMap` or `tokio::sync::RwLock<Arc<HashMap>>` with copy-on-write for lock-free reads.

### 34. DESIGN: No Graceful Shutdown for Background Tasks
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tui/mod.rs`
- **Lines:** 450, 482
- **Description:** Background tasks spawned with `tokio::spawn()` (audit log receiver, server task) are not given graceful shutdown signals. When the TUI exits, these tasks may be abruptly dropped.
- **Suggested Fix:** Use a `CancellationToken` or `tokio::sync::broadcast` channel to signal shutdown, and await task completion before exiting.

### 35. DESIGN: `TunnelConfig` Not Used for Quick Tunnel
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/tui/tunnel.rs`
- **Lines:** 11-16
- **Description:** The `TunnelConfig` from the config file is read and displayed, but the Quick Tunnel implementation ignores `config.tunnel.mode` and `config.tunnel.name`. The TUI always uses quick tunnel mode regardless of configuration.
- **Suggested Fix:** Respect the `tunnel.mode` configuration and implement named tunnel support in the TUI.

### 36. DESIGN: No Health Check or Reconnection Logic
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 47-62
- **Description:** `connect_all()` connects once at startup and never retries. If an upstream server goes down or becomes unreachable, it stays disconnected until the application restarts.
- **Suggested Fix:** Implement periodic health checks and automatic reconnection with exponential backoff.

### 37. DESIGN: `AuditLogger` Uses `mpsc` Channel but No Backpressure Handling
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/server/audit.rs`
- **Lines:** 26-36
- **Description:** The audit channel has a fixed capacity of 1000. When full, `send().await` would block, but the code uses `let _ =` which drops logs. There's no backpressure or overflow strategy.
- **Suggested Fix:** Use an unbounded channel for critical audit logs, or implement a file-based overflow buffer when the channel is full.

### 38. DESIGN: `prefix_tool_name` Uses `__` Separator Without Escaping
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/tool_filter.rs`
- **Lines:** 2-10
- **Description:** Tool names are prefixed with `upstream_name__tool_name`. If either the upstream name or tool name contains `__`, parsing will fail or produce incorrect results.
- **Suggested Fix:** Use a delimiter that is not allowed in names, or implement proper escaping. Alternatively, use a structured format like JSON or a tuple.

### 39. DESIGN: `ServerConfig` Lacks Validation
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/config.rs`
- **Lines:** 18-26
- **Description:** `ServerConfig` accepts any string for `name`, `url`, `command` without validation. Empty names, invalid URLs, or commands with path traversal (`../malicious`) are not rejected.
- **Suggested Fix:** Implement validation in `ServerConfig::new()` or `Config::load()`: check for empty names, valid URLs, and safe command paths.

### 40. DESIGN: `UpstreamType::Stdio` Spawns Commands Without Working Directory Control
- **File:** `/Users/driftcell/Projects/mcp-tunnel/src/mcp/client.rs`
- **Lines:** 281-285
- **Description:** `build_command()` creates a `Command` with no working directory, environment variable filtering, or sandboxing. The command inherits the parent's full environment.
- **Suggested Fix:** Allow configuring working directory and environment variables in `ServerConfig`, and consider using `std::process::Stdio::null()` for stdin if not needed.

---

## Additional Observations

### Positive Aspects
- Good use of `tracing` for structured logging
- Proper async/await patterns with `tokio`
- OAuth PKCE flow implementation is mostly correct
- TUI uses `ratatui` appropriately with proper event handling
- Audit logging architecture is well-designed (channel-based)
- Cancellation tokens used for server shutdown

### Missing Features
- No tests directory or test files found
- No CI/CD configuration
- No CHANGELOG or VERSION file
- No Docker support
- No metrics or monitoring endpoints
- No rate limiting on tool calls
- No request timeout configuration for tool calls

---

## Summary Statistics

| Category | Count |
|----------|-------|
| Critical Issues | 8 |
| Code Quality Issues | 9 |
| Error Handling Issues | 8 |
| Performance Issues | 5 |
| Architecture/Design Concerns | 10 |
| **Total** | **40** |

---

*Review generated by Claude Code - Haiku 4.5*
