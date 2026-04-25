# MCP Tunnel Code Review

**Project:** mcp-tunnel
**Version:** 0.1.0
**Edition:** 2024
**Review Date:** 2026-04-25
**Review Focus:** `config.toml` changes + `src/tui/mod.rs` background auto-OAuth initialization

---

## Changes Under Review

```
config.toml    | 46 ++++++++++++++-------------
src/tui/mod.rs | 93 +++++++++++++++++++++++++++++++++++++++++++++++++
```

---

## Critical Issues

### 1. Background task opens browser tabs unexpectedly on TUI startup
**File:** `src/tui/mod.rs:93`

The spawned background task calls `run_oauth_login` for each HTTP server. If no stored token exists, this triggers `run_pkce_flow` which calls `open::that()` to open the system browser. A user with N HTTP servers and no tokens gets N browser tabs spawned without any interaction.

**Fix:** Do not trigger the full PKCE flow in a background task. Check for existing tokens first (pure local I/O), and if missing, show a TUI status message prompting the user to press `o` to authenticate.

---

### 2. `run_oauth_login` does network I/O before checking the token store
**File:** `src/tui/mod.rs:496-530`

```rust
let mut state = OAuthState::new(url, None).await?;  // network discovery
state.start_authorization(...).await?;               // network
// ...then finally...
if store.load().await?.is_some() { return Ok(()); } // token check
```

Even when a valid token is already stored, the function still performs OAuth server discovery. On slow or flaky networks this blocks the background task unnecessarily.

**Fix:** Check `store.load()` first, before creating `OAuthState`.

---

### 3. Background task runs sequentially with no timeout
**File:** `src/tui/mod.rs:85-129`

```rust
tokio::spawn(async move {
    for server in http_servers {  // sequential
        // run_oauth_login + discover_tools — no timeout
    }
});
```

If `run_oauth_login` or `discover_tools` hangs on an unresponsive server, the entire background init stalls and subsequent servers never get processed.

**Fix:** Wrap each server init in `tokio::time::timeout`, and process servers concurrently with `futures::future::join_all` or by spawning a task per server.

---

### 4. Background init overwrites user's tool enable/disable choices
**File:** `src/tui/mod.rs:153-162`

```rust
if let Some(cache) = app.config.tool_cache.iter_mut().find(|c| c.server == server_name) {
    cache.tools = tool_infos;  // wipes previous enabled flags
}
```

All discovered tools are inserted with `enabled: true`. If the user previously disabled specific tools via the TUI, those choices are lost every time the TUI starts.

**Fix:** Merge discovered tools with existing cache entries, preserving the `enabled` flag for tools that already exist.

---

### 5. `HashSet` serialization produces non-deterministic ordering
**File:** `src/config.rs:23-25`

`enabled_tools` and `disabled_tools` are `HashSet<String>`. TOML serializes them as arrays, but `HashSet` iteration order is randomized. Every `config.save()` may reorder these arrays, creating unnecessary git diff noise and confusing config readers.

**Fix:** Change `HashSet` to `BTreeSet` for deterministic ordering.

---

### 6. Fire-and-forget background task is never cancelled
**File:** `src/tui/mod.rs:85`

The `tokio::spawn` handle is dropped immediately. If the user quits the TUI before background init completes, the task keeps running — potentially opening browsers or doing network I/O after exit.

**Fix:** Store the `JoinHandle`, abort it when `app.should_quit` is detected, or use a `CancellationToken` shared with the task.

---

### 7. `run_oauth_login` errors on `NoAuthorizationSupport`, skipping unauthenticated discovery
**File:** `src/tui/mod.rs:512-515`

If a server does not support OAuth, `run_oauth_login` returns an error and the background task skips tool discovery entirely. Some HTTP servers may be public and not require authentication at all.

**Fix:** Treat `NoAuthorizationSupport` as success (no auth needed), then proceed to tool discovery.

---

## Medium Issues

### 8. `connect_single` duplicates OAuth discovery work
**File:** `src/mcp/client.rs:96-109`

```rust
} else if upstream_supports_oauth(url).await? {  // creates OAuthState + start_authorization
    let new_token = run_pkce_flow(url).await?;     // does it all AGAIN
```

Both `upstream_supports_oauth` and `run_pkce_flow` create `OAuthState` and call `start_authorization`. This is two round-trips when one would suffice.

**Fix:** Reuse the `OAuthState` from the first discovery, or remove `upstream_supports_oauth` and handle `NoAuthorizationSupport` directly.

---

### 9. `tokio::sync::Mutex` used where `std::sync::Mutex` suffices
**File:** `src/app.rs:47`

`serve_audit_buffer: Arc<tokio::sync::Mutex<Vec<AuditLog>>>` is locked only for brief synchronous operations (`push`, `drain`). A `std::sync::Mutex` is more efficient and appropriate here.

---

### 10. No refresh token handling
When an OAuth access token expires, the code treats it as missing and triggers a full new PKCE flow including a browser popup. The `OAuthTokenResponse` likely contains a refresh token that could be used silently.

**Fix:** Implement refresh token flow in `FileCredentialStore` or `run_oauth_login`.

---

### 11. `let _ = app.save_config()` silently ignores errors
**File:** `src/tui/mod.rs:163`

If the config file is read-only or the disk is full, auto-discovered tools are lost with no user feedback.

**Fix:** Match on the result and display an error message via `app.set_message()`.

---

### 12. Add dialog is non-functional / misleading
**File:** `src/tui/servers.rs:119-126`

The dialog renders "Use CLI command instead" and tells users to press Esc. The input handling code still exists but the UI actively discourages using it.

**Fix:** Either remove the dialog code entirely or make the UI support adding servers properly.

---

## Low / Style Issues

### 13. Audit log scroll is inverted
**File:** `src/tui/mod.rs:288-296`

Up arrow increases `audit_scroll` (shows older logs), Down decreases it. This is counterintuitive.

### 14. `extract_url` in `quick.rs` is fragile
**File:** `src/tunnel/quick.rs:115-128`

URL extraction uses manual character trimming that could produce malformed URLs.

### 15. `tool_cache.enabled` is UI-only; actual serving uses `ServerConfig.disabled_tools`
The Tools tab toggle modifies both, but `AggregatedClient` only consults `ServerConfig`. This dual-source-of-truth is confusing.

---

## Positive Aspects

- Background init is a good UX improvement when tokens already exist — tool discovery happens seamlessly.
- Using `tokio::sync::mpsc` for background-to-UI communication is the right pattern.
- `try_recv` in the tick loop avoids blocking the async executor.
- Channel capacity of 16 is sufficient for the expected message volume.

---

## Recommended Fix Priority

| Priority | Issue | Effort |
|----------|-------|--------|
| P0 | Do not trigger PKCE/browser in background task | Small |
| P0 | Check token store before network I/O | Small |
| P0 | Add timeout + concurrency to background init | Small |
| P0 | Merge discovered tools instead of overwriting | Small |
| P1 | Change `HashSet` to `BTreeSet` in config | Tiny |
| P1 | Cancel background task on TUI exit | Small |
| P1 | Handle `NoAuthorizationSupport` gracefully | Small |
| P2 | Dedupe OAuth discovery in `connect_single` | Small |
| P2 | Add refresh token support | Medium |
