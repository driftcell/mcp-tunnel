use rmcp::transport::auth::{AuthError, OAuthState};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{info, warn};
use url::Url;

use crate::constants::{OAUTH_CALLBACK_ADDR, OAUTH_CALLBACK_URL};
use crate::error::AppError;

/// Result of attempting the PKCE OAuth flow.
pub enum PkceFlowResult {
    /// Authorization succeeded; contains the client_id and token response.
    Success {
        /// The OAuth client_id assigned during registration.
        client_id: String,
        /// The token response containing access and refresh tokens.
        token: rmcp::transport::auth::OAuthTokenResponse,
    },
    /// The server does not support OAuth authorization.
    NoAuthorizationSupport,
}

/// Result of attempting to refresh an OAuth token.
pub enum RefreshResult {
    /// Refresh succeeded; contains the new token response.
    Success(rmcp::transport::auth::OAuthTokenResponse),
    /// No refresh token was available.
    NoRefreshToken,
    /// The server does not support OAuth authorization.
    NoAuthorizationSupport,
}

/// Refresh an OAuth access token using a refresh token.
///
/// This uses rmcp's `AuthorizationManager` internally, which handles
/// metadata discovery and token refresh automatically.
///
/// The `client_id` must match the one used during the original PKCE authorization.
#[tracing::instrument(skip(refresh_token))]
pub async fn refresh_access_token(
    url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<RefreshResult, AppError> {
    use rmcp::transport::auth::{
        AuthorizationManager, CredentialStore, StoredCredentials,
    };

    // Build a token response from the refresh token so we can store it
    // in the AuthorizationManager's credential store.
    let mut token_response = rmcp::transport::auth::OAuthTokenResponse::new(
        oauth2::AccessToken::new("".to_string()),
        oauth2::basic::BasicTokenType::Bearer,
        rmcp::transport::auth::VendorExtraTokenFields::default(),
    );
    token_response.set_refresh_token(Some(oauth2::RefreshToken::new(refresh_token.to_string())));

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let stored = StoredCredentials::new(
        client_id.to_string(),
        Some(token_response),
        Vec::new(),
        Some(now_epoch),
    );

    // Use AuthorizationManager for metadata discovery and refresh
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let metadata = match manager.discover_metadata().await {
        Ok(m) => m,
        Err(AuthError::NoAuthorizationSupport) => return Ok(RefreshResult::NoAuthorizationSupport),
        Err(e) => return Err(AppError::OAuth(e.to_string())),
    };

    manager.set_metadata(metadata);
    manager
        .configure_client_id(client_id)
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    // Seed the credential store with our stored credentials so refresh_token() can find them
    let store = rmcp::transport::auth::InMemoryCredentialStore::new();
    CredentialStore::save(&store, stored)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;
    manager.set_credential_store(store);

    match manager.refresh_token().await {
        Ok(new_token) => {
            info!("OAuth token refreshed successfully");
            Ok(RefreshResult::Success(new_token))
        }
        Err(AuthError::TokenRefreshFailed(msg)) if msg.contains("No refresh token") => {
            Ok(RefreshResult::NoRefreshToken)
        }
        Err(AuthError::NoAuthorizationSupport) => Ok(RefreshResult::NoAuthorizationSupport),
        Err(e) => Err(AppError::OAuth(format!("token refresh failed: {}", e))),
    }
}

/// Run the full PKCE OAuth flow using rmcp's OAuthState.
/// Returns the token response on success, or `NoAuthorizationSupport` if the
/// server does not advertise OAuth authorization.
#[tracing::instrument]
pub async fn run_pkce_flow(url: &str) -> Result<PkceFlowResult, AppError> {
    let mut state = OAuthState::new(url, None)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    match state
        .start_authorization(&[], OAUTH_CALLBACK_URL, Some(env!("CARGO_PKG_NAME")))
        .await
    {
        Ok(()) => {}
        Err(AuthError::NoAuthorizationSupport) => return Ok(PkceFlowResult::NoAuthorizationSupport),
        Err(e) => return Err(AppError::OAuth(e.to_string())),
    }

    let auth_url = state
        .get_authorization_url()
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    info!("Opening browser for OAuth authorization");
    // Open browser for OAuth authorization
    if let Err(e) = open::that(&auth_url) {
        warn!("Failed to open browser: {}. Please navigate to the URL manually.", e);
        info!("Please open this URL in your browser: {}", auth_url);
    }

    // Extract the CSRF state parameter from the authorization URL for validation
    let expected_csrf = extract_state_from_url(&auth_url)
        .ok_or_else(|| AppError::OAuth("failed to extract state from authorization URL".to_string()))?;

    // Start local callback server and wait for code and csrf token (with 5-minute timeout)
    let (code, csrf_token) = match tokio::time::timeout(
        std::time::Duration::from_secs(300),
        wait_for_callback(OAUTH_CALLBACK_ADDR, expected_csrf),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            return Err(AppError::OAuth(
                "OAuth callback timed out after 5 minutes".to_string(),
            ));
        }
    };

    state
        .handle_callback(&code, &csrf_token)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let (client_id, token_response) = state
        .get_credentials()
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let token_response = token_response.ok_or_else(|| {
        AppError::OAuth("No token received after authorization".to_string())
    })?;

    info!("OAuth authorization successful (client_id: {})", client_id);
    Ok(PkceFlowResult::Success { client_id, token: token_response })
}

/// Extract the `state` query parameter from an authorization URL.
fn extract_state_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
}

/// Start local TCP callback server, wait for browser redirect.
/// Validates the CSRF `state` parameter against the expected token.
/// Returns (authorization_code, csrf_token) on success.
#[tracing::instrument]
async fn wait_for_callback(addr: &str, expected_csrf: String) -> Result<(String, String), AppError> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::OAuth(format!("failed to bind callback server: {}", e)))?;

    loop {
        match listener.accept().await {
            Ok((stream, _)) => match handle_callback_request(stream, &expected_csrf).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    warn!("Callback request handling error: {}", e);
                }
            },
            Err(e) => {
                return Err(AppError::OAuth(format!(
                    "callback server accept error: {}",
                    e
                )));
            }
        }
    }
}


/// Handle a single HTTP callback request, extracting the authorization code and CSRF token.
/// Validates the CSRF `state` parameter against the expected token.
async fn handle_callback_request(mut stream: TcpStream, expected_csrf: &str) -> Result<(String, String), AppError> {
    // Set a connection timeout for reading the request
    let read_timeout = std::time::Duration::from_secs(30);

    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();

    let read_result = tokio::time::timeout(read_timeout, reader.read_line(&mut request_line))
        .await
        .map_err(|_| AppError::OAuth("callback request read timed out".to_string()))?;

    read_result
        .map_err(|e| AppError::OAuth(format!("failed to read request: {}", e)))?;

    // Limit request line size to prevent memory exhaustion
    if request_line.len() > 4096 {
        return Err(AppError::OAuth("request line too long".to_string()));
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(AppError::OAuth("invalid HTTP request".to_string()));
    }

    // Validate HTTP method is GET
    if parts[0] != "GET" {
        return Err(AppError::OAuth(format!(
            "unsupported HTTP method: {}",
            parts[0]
        )));
    }

    let path = parts[1];

    // Read and discard remaining headers (with timeout)
    let mut line = String::new();
    loop {
        line.clear();
        match tokio::time::timeout(read_timeout, reader.read_line(&mut line)).await {
            Ok(Ok(0)) | Ok(Ok(_)) if line == "\r\n" || line == "\n" => break,
            Ok(Ok(n)) if n > 0 => continue,
            Ok(Ok(_)) => break,
            Ok(Err(_)) => break,
            Err(_) => return Err(AppError::OAuth("header read timed out".to_string())),
        }
    }

    // Parse query parameters
    let url = Url::parse(&format!("http://localhost{}", path))
        .map_err(|e| AppError::OAuth(format!("failed to parse callback URL: {}", e)))?;

    let query: std::collections::HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // Check for error
    if let Some(error) = query.get("error") {
        let error_desc = query.get("error_description").cloned().unwrap_or_default();
        return Err(AppError::OAuth(format!(
            "OAuth authorization error: {} - {}",
            error, error_desc
        )));
    }

    // Extract code
    let code = query
        .get("code")
        .cloned()
        .ok_or_else(|| AppError::OAuth("missing 'code' in callback".to_string()))?;

    // Extract CSRF token (state parameter) and validate it
    let csrf_token = query
        .get("state")
        .cloned()
        .ok_or_else(|| AppError::OAuth("missing 'state' in callback".to_string()))?;

    if csrf_token != expected_csrf {
        // Send error response to browser
        let error_response = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
            <!DOCTYPE html>\
            <html><head><title>CSRF Validation Failed</title></head>\
            <body style='font-family:sans-serif;text-align:center;padding-top:50px;'>\
            <h1 style='color:red;'>CSRF Validation Failed</h1>\
            <p>The authorization request could not be validated. Please try again.</p>\
            </body></html>";
        let _ = stream.write_all(error_response.as_bytes()).await;
        let _ = stream.flush().await;
        let _ = stream.shutdown().await;
        return Err(AppError::OAuth(
            "CSRF token validation failed: possible attack".to_string(),
        ));
    }

    // Send success response to browser
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
        <!DOCTYPE html>\
        <html><head><title>Authorization Successful</title></head>\
        <body style='font-family:sans-serif;text-align:center;padding-top:50px;'>\
        <h1>Authorization Successful</h1>\
        <p>You can close this window and return to the application.</p>\
        </body></html>";

    if let Err(e) = stream.write_all(response.as_bytes()).await {
        warn!("Failed to write OAuth callback response: {}", e);
    }
    if let Err(e) = stream.flush().await {
        warn!("Failed to flush OAuth callback stream: {}", e);
    }
    if let Err(e) = stream.shutdown().await {
        warn!("Failed to shutdown OAuth callback stream: {}", e);
    }

    Ok((code, csrf_token))
}
