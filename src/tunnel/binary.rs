use crate::error::{AppError, Result};
use std::path::PathBuf;
use tracing::info;

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

/// Check if cloudflared exists, download automatically if not
pub async fn ensure_cloudflared() -> Result<PathBuf> {
    let path = bin_path()?;
    if path.exists() {
        return Ok(path);
    }

    download_cloudflared().await?;
    Ok(path)
}

/// Download the appropriate cloudflared binary for the current platform
async fn download_cloudflared() -> Result<()> {
    use std::fs;

    let dir = bin_dir()?;
    fs::create_dir_all(&dir)?;

    let (url, is_tgz) = get_download_url()?;

    info!("Downloading cloudflared from {}...", url);

    let resp = reqwest::get(url).await.map_err(AppError::Http)?;
    let bytes = resp.bytes().await.map_err(AppError::Http)?;

    if is_tgz {
        // macOS ARM64 is .tgz format, needs extraction
        let tar_path = dir.join("cloudflared.tgz");
        let _bin = bin_path()?;
        fs::write(&tar_path, &bytes)?;

        let tar_gz = flate2::read::GzDecoder::new(std::fs::File::open(&tar_path)?);
        let mut archive = tar::Archive::new(tar_gz);
        archive.unpack(&dir)?;

        fs::remove_file(tar_path)?;
    } else {
        // Linux and others are direct binary files
        fs::write(bin_path()?, &bytes)?;
    }

    // Set executable permission (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bp = bin_path()?;
        let mut perms = fs::metadata(&bp)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bp, perms)?;
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
