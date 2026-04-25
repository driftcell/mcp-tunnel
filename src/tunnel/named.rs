use crate::error::{AppError, Result};
use tokio::process::Command;

/// Cloudflare 登录
/// 调用 `cloudflared tunnel login`，会在浏览器中打开 OAuth 页面
/// 凭证保存到 ~/.cloudflared/cert.pem
pub async fn login() -> Result<()> {
    let bin = super::binary::ensure_cloudflared().await?;

    println!("Opening browser for Cloudflare login...");
    println!("Please complete the authentication in your browser.");

    let status = Command::new(bin)
        .args(["tunnel", "login"])
        .status()
        .await
        .map_err(|e| AppError::Tunnel(format!("failed to run cloudflared login: {}", e)))?;

    if !status.success() {
        return Err(AppError::Tunnel("Cloudflare login failed".to_string()));
    }

    println!("Cloudflare login successful!");
    Ok(())
}

/// 创建具名 tunnel
/// 调用 `cloudflared tunnel create <name>`
/// 凭证保存到 ~/.cloudflared/<uuid>.json
pub async fn create_tunnel(name: &str) -> Result<()> {
    let bin = super::binary::ensure_cloudflared().await?;

    println!("Creating tunnel '{}'...", name);

    let status = Command::new(bin)
        .args(["tunnel", "create", name])
        .status()
        .await
        .map_err(|e| AppError::Tunnel(format!("failed to create tunnel: {}", e)))?;

    if !status.success() {
        return Err(AppError::Tunnel(format!("failed to create tunnel '{}'", name)));
    }

    println!("Tunnel '{}' created successfully.", name);
    Ok(())
}

/// 删除具名 tunnel
pub async fn delete_tunnel(name: &str) -> Result<()> {
    let bin = super::binary::ensure_cloudflared().await?;

    println!("Deleting tunnel '{}'...", name);

    let status = Command::new(bin)
        .args(["tunnel", "delete", name])
        .status()
        .await
        .map_err(|e| AppError::Tunnel(format!("failed to delete tunnel: {}", e)))?;

    if !status.success() {
        return Err(AppError::Tunnel(format!("failed to delete tunnel '{}'", name)));
    }

    println!("Tunnel '{}' deleted successfully.", name);
    Ok(())
}

/// 列出所有 tunnel
pub async fn list_tunnels() -> Result<()> {
    let bin = super::binary::ensure_cloudflared().await?;

    let output = Command::new(bin)
        .args(["tunnel", "list"])
        .output()
        .await
        .map_err(|e| AppError::Tunnel(format!("failed to list tunnels: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Tunnel(format!("failed to list tunnels: {}", stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);
    Ok(())
}

/// 启动具名 tunnel
/// name: tunnel 名称
/// local_url: 本地服务地址
#[allow(dead_code)]
pub async fn run_tunnel(name: &str, local_url: &str) -> Result<()> {
    let bin = super::binary::ensure_cloudflared().await?;

    println!(
        "Starting named tunnel '{}' pointing to {}...",
        name, local_url
    );

    let status = Command::new(bin)
        .args(["tunnel", "run", name])
        .env("TUNNEL_URL", local_url)
        .status()
        .await
        .map_err(|e| AppError::Tunnel(format!("failed to run tunnel: {}", e)))?;

    if !status.success() {
        return Err(AppError::Tunnel(format!(
            "tunnel '{}' exited with error",
            name
        )));
    }

    Ok(())
}
