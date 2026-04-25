use crate::error::{AppError, Result};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tracing::{info, warn};

/// Quick tunnel 管理器
pub struct QuickTunnel {
    child: Option<Child>,
    url: Option<String>,
}

impl QuickTunnel {
    pub fn new() -> Self {
        Self {
            child: None,
            url: None,
        }
    }

    /// 启动 quick tunnel
    /// local_url: 本地服务地址，如 "http://localhost:3000"
    /// 返回生成的公网 URL
    #[tracing::instrument(skip(self))]
    pub async fn start(&mut self, local_url: &str) -> Result<String> {
        let bin = super::binary::ensure_cloudflared().await?;

        let mut child = Command::new(bin)
            .args(["tunnel", "--url", local_url])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Tunnel(format!("failed to start cloudflared: {}", e)))?;

        // 读取 stderr 输出，提取 trycloudflare.com URL
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
                // 提取 URL，格式如: https://xxx.trycloudflare.com
                if line.contains("trycloudflare.com")
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

        let url = url.ok_or_else(|| {
            AppError::Tunnel("could not extract tunnel URL from output".to_string())
        })?;

        info!("Quick tunnel started: {}", url);
        self.child = Some(child);
        self.url = Some(url.clone());
        Ok(url)
    }

    /// 停止 tunnel
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
        self.url = None;
        Ok(())
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }
}

/// 从 cloudflared 输出中提取 trycloudflare.com URL
fn extract_url(line: &str) -> Option<String> {
    // 简单提取包含 trycloudflare.com 的 URL
    for word in line.split_whitespace() {
        if word.contains("trycloudflare.com") {
            let url = word.trim_matches(|c: char| {
                !c.is_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '-'
            });
            if url.starts_with("http") {
                return Some(url.to_string());
            }
        }
    }
    None
}
