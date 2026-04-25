# OAuth Integration Plan (Using rmcp Auth)

## Problem

When a user adds an MCP server that requires OAuth (e.g. Notion), the current flow is broken:

```bash
mt add notion https://mcp.notion.com/mcp
```

This creates a `ServerConfig` with no way to authenticate. When `mt serve` later tries to connect, the HTTP request has no `Authorization` header, and the upstream rejects it with 401 Unauthorized.

The missing piece: **there is no automatic OAuth discovery and flow initiation.**

Other MCP clients handle this transparently — you just add the endpoint, and on first connect the client discovers OAuth configuration from the server, opens the browser, and completes the authorization flow without any manual configuration.

---

## Goals

1. User can add an OAuth-enabled MCP server with just the URL: `mt add notion https://mcp.notion.com/mcp`
2. On first connect, the client automatically discovers OAuth metadata from the server
3. Browser opens automatically for authorization (PKCE flow, no client_secret)
4. Token is saved locally and refreshed automatically
5. TUI shows OAuth status and allows manual re-authorization

---

## Design (Leveraging rmcp Auth Features)

rmcp provides a built-in `auth` feature that handles OAuth 2.0 discovery, PKCE flow, token storage, and automatic Bearer header injection. We use these primitives instead of implementing OAuth from scratch.

### Key rmcp Auth Types

| Type | Purpose |
|------|---------|
| `AuthorizationManager` | Discovers OAuth metadata, configures the OAuth2 client, manages tokens |
| `AuthClient<C>` | Wraps an HTTP client; auto-injects `Authorization: Bearer <token>` on every request |
| `AuthorizationSession` | Initiates PKCE flow: generates auth URL with code challenge, validates callback |
| `CredentialStore` trait | Interface for persisting/loading `StoredCredentials` (we implement file-based) |
| `StoredAuthorizationState` | PKCE verifier + CSRF token for a session |
| `OAuthClientConfig` | `client_id`, `client_secret`, `scopes`, `redirect_uri` |
| `AuthorizationMetadata` | Discovered endpoints: `authorization_endpoint`, `token_endpoint`, etc. |

---

### 1. OAuth Discovery (rmcp handles this)

`AuthorizationManager::discover_metadata()` automatically discovers OAuth configuration:

1. Tries **resource metadata discovery** first (MCP-specific)
2. Falls back to `/.well-known/oauth-authorization-server` at the base URL
3. Returns `AuthorizationMetadata` with endpoints, scopes, etc.
4. Returns `AuthError::NoAuthorizationSupport` if nothing found

**No custom discovery code needed.** We simply create an `AuthorizationManager` with the server URL and call `discover_metadata().await`.

```rust
use rmcp::transport::auth::AuthorizationManager;

let mut auth_mgr = AuthorizationManager::new(&server_url).await?;
match auth_mgr.discover_metadata().await {
    Ok(metadata) => auth_mgr.set_metadata(metadata),
    Err(AuthError::NoAuthorizationSupport) => { /* server doesn't require OAuth */ }
    Err(e) => return Err(e.into()),
}
```

---

### 2. CredentialStore Implementation (File-Based)

rmcp defines the `CredentialStore` trait. We implement it for file-based persistence at `~/.local/share/mcp-tunnel/oauth/<server_name>.json`.

```rust
use rmcp::transport::auth::{CredentialStore, StoredCredentials, AuthError};

pub struct FileCredentialStore {
    path: PathBuf,
}

#[async_trait]
impl CredentialStore for FileCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&self.path).await
            .map_err(|e| AuthError::StorageError(e.to_string()))?;
        let creds: StoredCredentials = serde_json::from_str(&json)
            .map_err(|e| AuthError::StorageError(e.to_string()))?;
        Ok(Some(creds))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let json = serde_json::to_string_pretty(&credentials)
            .map_err(|e| AuthError::StorageError(e.to_string()))?;
        tokio::fs::write(&self.path, json).await
            .map_err(|e| AuthError::StorageError(e.to_string()))?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let _ = tokio::fs::remove_file(&self.path).await;
        Ok(())
    }
}
```

**Note:** `StoredCredentials` contains `client_id` and an optional `token_response` (which includes `access_token`, `refresh_token`, `expires_in`, etc.). rmcp handles refresh automatically via `get_access_token()`.

---

### 3. Simplified OAuth Flow (PKCE, Public Client)

Since this is a native CLI application, we use OAuth 2.0 PKCE without a `client_secret` (public client per RFC 8252).

**Flow using rmcp's `AuthorizationSession`:**

```rust
use rmcp::transport::auth::{AuthorizationSession, OAuthClientConfig};

// 1. Configure the OAuth client (public client — no secret)
let client_config = OAuthClientConfig {
    client_id: "mcp-tunnel".to_string(),  // or server-specific client_id if known
    client_secret: None,
    scopes: vec![],  // use server's default scopes, or populate from metadata
    redirect_uri: "http://127.0.0.1:9876/callback".to_string(),
};

// 2. Configure the AuthorizationManager with the client
auth_mgr.configure_client(client_config)?;

// 3. Create an authorization session
let session = AuthorizationSession::new(
    &auth_mgr,
    vec![],           // scopes
    "http://127.0.0.1:9876/callback",
    Some("mcp-tunnel".to_string()),
    None,             // client_metadata_url
).await?;

// 4. Open browser with the authorization URL
let auth_url = session.get_authorization_url();
open::that(auth_url)?;

// 5. Start local callback server, receive code + csrf_token
let (code, csrf_token) = run_callback_server("127.0.0.1:9876").await?;

// 6. Complete authorization — rmcp exchanges code for token
let token_response = session.handle_callback(code, csrf_token).await?;
// Token is automatically saved via the CredentialStore
```

**Token lifecycle:**
- `AuthClient::get_access_token()` checks saved token → returns if valid
- If expired → rmcp automatically refreshes using `refresh_token`
- If no token or refresh fails → returns `AuthError`, we trigger full re-authorization

---

### 4. CLI: No OAuth Configuration Needed

**Add command stays simple:**

```bash
mt add notion https://mcp.notion.com/mcp
```

No `--oauth-*` flags. No prompts for client credentials. The server URL is all that's needed.

**Clap changes in `src/cli.rs`:**

Remove all OAuth-related flags from the `Add` command:
- ~~`--oauth-provider`~~
- ~~`--oauth-client-id`~~
- ~~`--oauth-client-secret`~~
- ~~`--oauth-authorize-url`~~
- ~~`--oauth-token-url`~~
- ~~`--oauth-scope`~~

The `Add` command becomes just `name` + `url`.

**Config changes in `src/config.rs`:**

Remove the `OAuthConfig` struct entirely. Replace `oauth: Option<OAuthConfig>` on `ServerConfig` with nothing — OAuth is discovered at runtime, not configured.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: UpstreamType,
    // oauth is discovered at runtime, not configured
    #[serde(default)]
    pub enabled_tools: HashSet<String>,
    #[serde(default)]
    pub disabled_tools: HashSet<String>,
}
```

---

### 5. Connection Flow with Auto-OAuth

**In `src/mcp/client.rs` — HTTP upstream connection:**

```rust
use rmcp::transport::{
    streamable_http_client::{StreamableHttpClientTransport, StreamableHttpClientTransportConfig},
    auth::{AuthClient, AuthorizationManager},
};

// For HTTP upstreams that may require OAuth:
crate::config::UpstreamType::Http { url } => {
    // 1. Create the base HTTP client (reqwest-based)
    let http_client = rmcp::transport::common::auth::DefaultHttpClient::new();

    // 2. Set up auth manager for this server
    let mut auth_mgr = AuthorizationManager::new(&url).await?;

    // 3. Try to discover OAuth metadata
    match auth_mgr.discover_metadata().await {
        Ok(metadata) => {
            auth_mgr.set_metadata(metadata);

            // 4. Set up credential store for this server
            let store = FileCredentialStore::new(&server_name);
            auth_mgr.set_credential_store(store);

            // 5. Try to initialize from stored credentials
            let has_creds = auth_mgr.initialize_from_store().await?;

            if !has_creds {
                // No stored token — need to authorize
                let client_config = OAuthClientConfig {
                    client_id: "mcp-tunnel".to_string(),
                    client_secret: None,
                    scopes: vec![],
                    redirect_uri: "http://127.0.0.1:9876/callback".to_string(),
                };
                auth_mgr.configure_client(client_config)?;

                // Trigger PKCE flow (browser + callback)
                let token = run_pkce_flow(&auth_mgr).await?;
            }

            // 6. Wrap HTTP client with AuthClient (auto-injects Bearer tokens)
            let auth_client = AuthClient::new(http_client, auth_mgr);

            // 7. Create transport with the auth-wrapped client
            let config = StreamableHttpClientTransportConfig::with_uri(&url);
            let transport = StreamableHttpClientTransport::with_client(auth_client, config);

            let client = serve_client(MyClientHandler, transport).await?;
            Ok(client)
        }
        Err(AuthError::NoAuthorizationSupport) => {
            // Server doesn't use OAuth — connect directly
            let transport = StreamableHttpClientTransport::with_uri(&url);
            let client = serve_client(MyClientHandler, transport).await?;
            Ok(client)
        }
        Err(e) => Err(e.into()),
    }
}
```

The key insight: **OAuth is handled by rmcp's `AuthClient` wrapper.** It automatically:
- Injects `Authorization: Bearer <token>` on every HTTP request
- Refreshes expired tokens via `refresh_token`
- Reads from / writes to our `FileCredentialStore`

---

### 6. TUI: OAuth Status & Actions

**Servers tab — show OAuth status:**

```
┌──────────────────────────────────────────┐
│  Server: notion                          │
│  Type: HTTP                              │
│  URL: https://mcp.notion.com/mcp         │
│  Status: Connected                       │
│  OAuth: Authenticated (expires in 6h)    │
│  Tools: 12 total (10 enabled, 2 disabled)│
└──────────────────────────────────────────┘
```

OAuth status values:
- `Not required` — server doesn't use OAuth (no auth metadata discovered)
- `Not authenticated` — server requires OAuth but no saved token
- `Authenticated (expires in X)` — has valid token (read from credential store)
- `Expired` — token expired, will auto-refresh on next connect

**Key bindings:**

| Key | Action |
|-----|--------|
| `o` | Initiate OAuth login (opens browser) |
| `O` | Clear saved OAuth token |

When `o` is pressed:
1. Get or create `AuthorizationManager` for the selected server's URL
2. Call `discover_metadata()` if not already cached
3. Configure client and trigger `AuthorizationSession` PKCE flow
4. Show success/error message

When `O` is pressed:
1. Call `CredentialStore::clear()` for the selected server
2. Next connect will trigger full re-authorization

**Add dialog stays simple:**

Just name + URL. No OAuth dropdown or provider selection.

---

### 7. Token Lifecycle UX

**Token saved location:** `~/.local/share/mcp-tunnel/oauth/<server_name>.json`

**Expiration handling:**
- `AuthClient::get_access_token()` automatically refreshes before expiration
- If refresh fails, returns `AuthError` — we fall back to full re-authorization (browser flow)
- User sees a brief "Re-authorizing with <server>..." message

**Manual re-authorization:**
- `mt clear-token <server>` → calls `CredentialStore::clear()`
- Next connect triggers full OAuth flow again
- TUI: `O` key clears token, `o` key triggers authorization

---

## Implementation Phases

### Phase 1: FileCredentialStore
- [ ] Implement `FileCredentialStore` in `src/mcp/oauth/store.rs`
- [ ] Implement `CredentialStore` trait for file-based persistence
- [ ] Store tokens at `~/.local/share/mcp-tunnel/oauth/<server_name>.json`
- [ ] Add unit tests for save/load/clear

### Phase 2: Simplify Config (Remove OAuthConfig)
- [ ] Remove `OAuthConfig` struct from `src/config.rs`
- [ ] Remove `oauth` field from `ServerConfig`
- [ ] Remove all `--oauth-*` flags from `src/cli.rs`
- [ ] Remove OAuth prompt logic from `src/main.rs` Add handler
- [ ] Update `ClearToken` command to call `FileCredentialStore::clear()`

### Phase 3: Auth-Aware HTTP Transport
- [ ] Update HTTP upstream connection in `src/mcp/client.rs`
- [ ] Create `AuthorizationManager`, call `discover_metadata()`
- [ ] Set up `FileCredentialStore` on the manager
- [ ] If no stored credentials, trigger PKCE flow via `AuthorizationSession`
- [ ] Wrap HTTP client with `AuthClient` and create `StreamableHttpClientTransport::with_client()`
- [ ] If `NoAuthorizationSupport`, connect directly without auth

### Phase 4: PKCE Flow Helper
- [ ] Create `src/mcp/oauth/flow.rs` with `run_pkce_flow(auth_mgr)` helper
- [ ] Create `AuthorizationSession`, get authorization URL
- [ ] Open browser (`open` crate)
- [ ] Start local async callback server (`tokio::net::TcpListener` on 127.0.0.1:9876)
- [ ] Call `session.handle_callback(code, csrf_token)`
- [ ] Handle errors (timeout, denied, CSRF mismatch)

### Phase 5: TUI OAuth Integration
- [ ] Show OAuth status in Servers tab detail panel (check token file existence + read expiry)
- [ ] Add `o` key binding to trigger PKCE flow for selected server
- [ ] Add `O` key binding to clear saved token for selected server
- [ ] Handle async OAuth in background task with status polling

---

## Example Usage (After Implementation)

```bash
# Add a server — just the URL, nothing else
mt add notion https://mcp.notion.com/mcp

# Serve — first connect discovers OAuth and triggers browser flow automatically
mt serve
# → Discovering OAuth metadata from notion...
# → Opening browser for authorization...
# → Authorization successful
# → Connected to upstream: notion

# Clear token and re-auth next time
mt clear-token notion

# TUI
mt
# → Select notion server
# → Press 'o' to manually trigger OAuth
# → Press 'O' to clear saved token
```

---

## Files to Modify

| File | Change |
|------|--------|
| `src/mcp/oauth/store.rs` | **New** — `FileCredentialStore` implementing rmcp's `CredentialStore` trait |
| `src/mcp/oauth/flow.rs` | **New** — PKCE flow helper using rmcp's `AuthorizationSession` |
| `src/mcp/oauth.rs` | Re-export store and flow modules; remove old custom OAuth implementation |
| `src/mcp/oauth/providers.rs` | **Delete** — no longer needed |
| `src/mcp/oauth/discovery.rs` | **Delete** — rmcp handles discovery via `AuthorizationManager` |
| `src/cli.rs` | Remove all `--oauth-*` flags from `Add` command |
| `src/main.rs` | Remove OAuth config construction in Add handler |
| `src/config.rs` | Remove `OAuthConfig` struct and `oauth` field from `ServerConfig` |
| `src/mcp/client.rs` | Use `AuthClient` wrapper + `AuthorizationManager` for HTTP upstreams |
| `src/tui/mod.rs` | Add 'o'/'O' key handlers |
| `src/tui/servers.rs` | Show OAuth status in detail panel |

---

## Key Differences from Original Plan

| Aspect | Original Plan | Updated Plan (using rmcp auth) |
|--------|--------------|-------------------------------|
| Discovery | Custom `discover_oauth()` fetching well-known endpoint | `AuthorizationManager::discover_metadata()` (resource metadata + well-known) |
| Token storage | Custom JSON files with our own schema | Implement rmcp's `CredentialStore` trait; rmcp defines `StoredCredentials` |
| Token refresh | Custom `ensure_token()` logic | `AuthClient::get_access_token()` handles refresh automatically |
| HTTP auth injection | Manual `try_connect_with_auth()` | `AuthClient` wrapper auto-injects Bearer on every request |
| PKCE flow | Custom code challenge/verifier generation | `AuthorizationSession` handles PKCE, CSRF, code exchange |
| OAuth client config | Custom structs | rmcp's `OAuthClientConfig` + `AuthorizationMetadata` |
| Dependencies | `oauth2` crate directly | Use rmcp's built-in auth (already in Cargo.toml) |
