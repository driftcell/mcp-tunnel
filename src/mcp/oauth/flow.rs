use rmcp::transport::auth::OAuthState;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{info, warn};
use url::Url;

use crate::error::AppError;
use crate::mcp::oauth::{OAUTH_CALLBACK_ADDR, OAUTH_CALLBACK_URL};

/// Run the full PKCE OAuth flow using rmcp's OAuthState.
/// Returns the token response on success.
#[tracing::instrument]
pub async fn run_pkce_flow(url: &str) -> Result<rmcp::transport::auth::OAuthTokenResponse, AppError> {
    let mut state = OAuthState::new(url, None)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    state
        .start_authorization(&[], OAUTH_CALLBACK_URL, Some(env!("CARGO_PKG_NAME")))
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let auth_url = state
        .get_authorization_url()
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    info!("Opening browser for OAuth authorization");
    // Open browser for OAuth authorization
    if let Err(e) = open::that(&auth_url) {
        warn!("Failed to open browser: {}. Please navigate to the URL manually.", e);
        println!("Please open this URL in your browser: {}", auth_url);
    }

    // Extract the CSRF state parameter from the authorization URL for validation
    let expected_csrf = extract_state_from_url(&auth_url)
        .ok_or_else(|| AppError::OAuth("failed to extract state from authorization URL".to_string()))?;

    // Start local callback server and wait for code and csrf token
    let (code, csrf_token) = wait_for_callback(OAUTH_CALLBACK_ADDR, expected_csrf).await?;

    state
        .handle_callback(&code, &csrf_token)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let (_, token_response) = state
        .get_credentials()
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let token_response = token_response.ok_or_else(|| {
        AppError::OAuth("No token received after authorization".to_string())
    })?;

    info!("OAuth authorization successful");
    Ok(token_response)
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
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| AppError::OAuth(format!("failed to read request: {}", e)))?;

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(AppError::OAuth("invalid HTTP request".to_string()));
    }

    let path = parts[1];

    // Read and discard remaining headers
    let mut line = String::new();
    while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
        if line == "\r\n" || line == "\n" {
            break;
        }
        line.clear();
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
