use std::path::PathBuf;

use chrono::{DateTime, Utc};
use oauth2::TokenResponse;
use rmcp::transport::auth::OAuthTokenResponse;

/// Wrapper around OAuthTokenResponse that also stores the issue time
/// so we can check token expiration without relying on server 401 responses.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredToken {
    #[serde(flatten)]
    pub token: OAuthTokenResponse,
    pub issue_time: DateTime<Utc>,
}

impl StoredToken {
    /// Check if the token has expired based on its `expires_in` field.
    /// Returns `true` if the token has expired or if expiration info is unavailable.
    pub fn is_expired(&self) -> bool {
        match self.token.expires_in() {
            None => false, // No expiration info, assume valid
            Some(duration) => {
                let elapsed = Utc::now().signed_duration_since(self.issue_time);
                elapsed.to_std().map_or(true, |d| d >= duration)
            }
        }
    }
}

pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub fn new(server_name: &str) -> crate::error::Result<Self> {
        let path = dirs::data_local_dir()
            .ok_or_else(|| crate::error::AppError::Config("Could not determine data directory".to_string()))?
            .join("mcp-tunnel")
            .join("oauth")
            .join(format!("{}.json", server_name));
        Ok(Self { path })
    }

    /// Load stored OAuth token response from file.
    /// Returns `None` if no token exists. Returns an error for invalid or expired tokens.
    pub async fn load(&self) -> crate::error::Result<Option<StoredToken>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&self.path)
            .await
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;

        // Try loading as the new format (with issue_time)
        match serde_json::from_str::<StoredToken>(&json) {
            Ok(stored) => {
                if stored.is_expired() {
                    tracing::warn!("Stored OAuth token has expired");
                    return Ok(None);
                }
                Ok(Some(stored))
            }
            Err(_) => {
                // Fallback: try loading as the old format (raw OAuthTokenResponse)
                // Treat old-format tokens as potentially expired (conservative)
                tracing::warn!("Token file is in old format (no issue_time), treating as expired");
                Ok(None)
            }
        }
    }

    /// Save OAuth token response to file with issue time tracking.
    pub async fn save(&self, token: &OAuthTokenResponse) -> crate::error::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| crate::error::AppError::OAuth(format!("failed to create directory: {}", e)))?;
        }
        let stored = StoredToken {
            token: token.clone(),
            issue_time: Utc::now(),
        };
        let json = serde_json::to_string_pretty(&stored)
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;
        tokio::fs::write(&self.path, json)
            .await
            .map_err(|e| crate::error::AppError::OAuth(e.to_string()))?;

        // Set restrictive permissions on Unix (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&self.path, perms)
                .map_err(|e| crate::error::AppError::OAuth(format!("failed to set file permissions: {}", e)))?;
        }

        Ok(())
    }

    /// Clear stored credentials.
    pub async fn clear(&self) -> crate::error::Result<()> {
        if self.path.exists() {
            tokio::fs::remove_file(&self.path)
                .await
                .map_err(|e| crate::error::AppError::OAuth(format!("failed to remove token file: {}", e)))?;
        }
        Ok(())
    }
}
