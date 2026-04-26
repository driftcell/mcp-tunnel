use crate::error::{AppError, Result};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tracing::{info, warn};

/// Quick tunnel manager
pub struct QuickTunnel {
    child: Option<Child>,
}

impl QuickTunnel {
    pub fn new() -> Self {
        Self { child: None }
    }

    /// Start a quick tunnel
    /// local_url: local service URL, e.g. "http://localhost:3000"
    /// Returns the generated public URL
    #[tracing::instrument(skip(self))]
    pub async fn start(&mut self, local_url: &str) -> Result<String> {
        let bin = super::binary::ensure_cloudflared().await?;

        let mut child = Command::new(bin)
            .args(["tunnel", "--url", local_url])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Tunnel(format!("failed to start cloudflared: {}", e)))?;

        // Read stderr output and extract trycloudflare.com URL
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Tunnel("failed to capture cloudflared stderr".to_string()))?;

        use tokio::io::{AsyncBufReadExt, BufReader};
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        let mut url = None;
        let timeout = tokio::time::Duration::from_secs(30);

        let result = tokio::time::timeout(timeout, async {
            while let Ok(Some(line)) = lines.next_line().await {
                info!("[cloudflared] {}", line);
                // Extract URL, format: https://xxx.trycloudflare.com
                if line.to_lowercase().contains("trycloudflare.com")
                    && let Some(u) = extract_url(&line)
                {
                    url = Some(u);
                    break;
                }
            }
        })
        .await;

        if result.is_err() {
            if let Err(e) = child.kill().await {
                warn!("Failed to kill cloudflared process: {}", e);
            }
            // Wait for process to exit to avoid zombie processes
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_secs(5),
                child.wait(),
            )
            .await;
            return Err(AppError::Tunnel(
                "timeout waiting for tunnel URL".to_string(),
            ));
        }

        // Spawn a background task to drain remaining stderr so the pipe doesn't block
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut buf = String::new();
                while let Ok(n) = reader.read_line(&mut buf).await {
                    if n == 0 { break; }
                    buf.clear();
                }
            });
        }

        let url = url.ok_or_else(|| {
            AppError::Tunnel("could not extract tunnel URL from output".to_string())
        })?;

        info!("Quick tunnel started: {}", url);
        self.child = Some(child);
        Ok(url)
    }

    /// Stop the tunnel
    #[tracing::instrument(skip(self))]
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            if let Err(e) = child.kill().await {
                warn!("Failed to kill cloudflared process: {}", e);
            }
            // Wait for process to exit to avoid zombie processes
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_secs(5),
                child.wait(),
            )
            .await;
            info!("Quick tunnel stopped");
        }
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }
}

/// Extract trycloudflare.com URL from cloudflared output
fn extract_url(line: &str) -> Option<String> {
    // Find the start of https://
    if let Some(start) = line.find("https://") {
        let rest = &line[start..];
        // Take until whitespace
        let end = rest
            .find(|c: char| c.is_whitespace())
            .unwrap_or(rest.len());
        let candidate = &rest[..end];
        // Trim trailing punctuation
        let url = candidate.trim_end_matches(|c: char| {
            matches!(c, '.' | ',' | ')' | ']' | '"' | '\'' | '>' | '<' | '{' | '}' | ';' | ':' | '|')
        });
        if url.to_lowercase().contains("trycloudflare.com") && url::Url::parse(url).is_ok() {
            return Some(url.to_string());
        }
    }
    None
}
