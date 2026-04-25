use std::path::PathBuf;

use rmcp::transport::auth::OAuthTokenResponse;

pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub fn new(server_name: &str) -> Self {
        let path = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mcp-tunnel")
            .join("oauth")
            .join(format!("{}.json", server_name));
        Self { path }
    }

    /// Load stored OAuth token response from file.
    pub async fn load(&self) -> crate::error::Result<Option<OAuthTokenResponse>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&self.path)
            .await
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;
        let token: OAuthTokenResponse = serde_json::from_str(&json)
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;
        Ok(Some(token))
    }

    /// Save OAuth token response to file.
    pub async fn save(&self, token: &OAuthTokenResponse) -> crate::error::Result<()> {
        if let Some(parent) = self.path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let json = serde_json::to_string_pretty(token)
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;
        tokio::fs::write(&self.path, json)
            .await
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;
        Ok(())
    }

    /// Clear stored credentials.
    pub async fn clear(&self) -> crate::error::Result<()> {
        let _ = tokio::fs::remove_file(&self.path).await;
        Ok(())
    }
}
