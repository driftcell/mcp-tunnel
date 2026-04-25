use rmcp::transport::auth::OAuthState;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{info, warn};
use url::Url;

use crate::error::AppError;

/// Run the full PKCE OAuth flow using rmcp's OAuthState.
/// Returns the token response on success.
pub async fn run_pkce_flow(url: &str) -> Result<rmcp::transport::auth::OAuthTokenResponse, AppError> {
    let mut state = OAuthState::new(url, None)
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    state
        .start_authorization(&[], "http://127.0.0.1:9876/callback", Some("mcp-tunnel"))
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    let auth_url = state
        .get_authorization_url()
        .await
        .map_err(|e| AppError::OAuth(e.to_string()))?;

    info!("Opening browser for OAuth authorization: {}", auth_url);
    let _ = open::that(&auth_url);

    // Start local callback server and wait for code and csrf token
    let (code, csrf_token) = wait_for_callback("127.0.0.1:9876").await?;

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

/// Start local TCP callback server, wait for browser redirect.
/// Returns (authorization_code, csrf_token) on success.
async fn wait_for_callback(addr: &str) -> Result<(String, String), AppError> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::OAuth(format!("failed to bind callback server: {}", e)))?;

    loop {
        match listener.accept().await {
            Ok((stream, _)) => match handle_callback_request(stream).await {
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
async fn handle_callback_request(mut stream: TcpStream) -> Result<(String, String), AppError> {
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

    // Extract CSRF token (state parameter)
    let csrf_token = query
        .get("state")
        .cloned()
        .ok_or_else(|| AppError::OAuth("missing 'state' in callback".to_string()))?;

    // Send success response to browser
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
        <!DOCTYPE html>\
        <html><head><title>Authorization Successful</title></head>\
        <body style='font-family:sans-serif;text-align:center;padding-top:50px;'>\
        <h1>Authorization Successful</h1>\
        <p>You can close this window and return to the application.</p>\
        </body></html>";

    let _ = stream.write_all(response.as_bytes()).await;

    Ok((code, csrf_token))
}
