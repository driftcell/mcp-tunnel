use crate::error::{AppError, Result};
use std::path::PathBuf;
use tracing::{info, warn};

/// Directory where the cloudflared binary is stored
pub fn bin_dir() -> crate::error::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| crate::error::AppError::Config("Could not determine data directory".to_string()))?
        .join("mcp-tunnel");
    Ok(dir)
}

/// Path to the cloudflared binary
pub fn bin_path() -> crate::error::Result<PathBuf> {
    Ok(bin_dir()?.join("cloudflared"))
}

/// Validate that the cloudflared binary is actually executable.
async fn validate_cloudflared(path: &PathBuf) -> Result<()> {
    let path = path.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&path)
            .arg("--version")
            .output()
    })
    .await
    .map_err(|e| AppError::Tunnel(format!("failed to run validation: {}", e)))?
    .map_err(|e| AppError::Tunnel(format!("cloudflared --version failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Tunnel(format!(
            "cloudflared --version exited with error: {}",
            stderr
        )));
    }
    Ok(())
}

/// Check if cloudflared exists, download automatically if not
pub async fn ensure_cloudflared() -> Result<PathBuf> {
    let path = bin_path()?;
    if path.exists() {
        if let Err(e) = validate_cloudflared(&path).await {
            warn!("cloudflared binary is invalid ({}), removing and re-downloading", e);
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| AppError::Tunnel(format!("failed to remove invalid binary: {}", e)))?;
        } else {
            return Ok(path);
        }
    }

    download_cloudflared().await?;
    Ok(path)
}

/// Download the appropriate cloudflared binary for the current platform
async fn download_cloudflared() -> Result<()> {
    let dir = bin_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::Tunnel(format!("failed to create bin dir: {}", e)))?;

    let (url, is_tgz) = get_download_url()?;

    info!("Downloading cloudflared from {}...", url);

    let resp = reqwest::get(url).await.map_err(AppError::Http)?;
    let bytes = resp.bytes().await.map_err(AppError::Http)?;

    // Perform blocking extraction on a dedicated thread pool
    let dir_clone = dir.clone();
    tokio::task::spawn_blocking(move || {
        let result: Result<()> = (|| {
            if is_tgz {
                let tar_path = dir_clone.join("cloudflared.tgz");
                std::fs::write(&tar_path, &bytes)
                    .map_err(|e| AppError::Tunnel(format!("failed to write tar: {}", e)))?;

                let tar_gz = flate2::read::GzDecoder::new(
                    std::fs::File::open(&tar_path)
                        .map_err(|e| AppError::Tunnel(format!("failed to open tar: {}", e)))?,
                );
                let mut archive = tar::Archive::new(tar_gz);
                archive
                    .unpack(&dir_clone)
                    .map_err(|e| AppError::Tunnel(format!("failed to unpack tar: {}", e)))?;

                std::fs::remove_file(&tar_path)
                    .map_err(|e| AppError::Tunnel(format!("failed to remove tar: {}", e)))?;
            } else {
                std::fs::write(dir_clone.join("cloudflared"), &bytes)
                    .map_err(|e| AppError::Tunnel(format!("failed to write binary: {}", e)))?;
            }
            Ok(())
        })();
        result
    })
    .await
    .map_err(|e| AppError::Tunnel(format!("extraction task panicked: {}", e)))??;

    // Set executable permission (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bp = bin_path()?;
        let mut perms = std::fs::metadata(&bp)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bp, perms)?;
    }

    info!("cloudflared downloaded to {:?}", bin_path()?);
    Ok(())
}

/// Return the download URL for the current platform
fn get_download_url() -> Result<(&'static str, bool)> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("macos", "aarch64") => Ok((
            "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz",
            true,
        )),
        ("macos", "x86_64") => Ok((
            "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-amd64.tgz",
            true,
        )),
        ("linux", "x86_64") => Ok((
            "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64",
            false,
        )),
        ("linux", "aarch64") => Ok((
            "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64",
            false,
        )),
        _ => Err(AppError::Tunnel(format!("Unsupported platform: {}-{}", os, arch))),
    }
}
