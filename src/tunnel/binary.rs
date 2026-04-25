use crate::error::{AppError, Result};
use std::path::PathBuf;

/// cloudflared 二进制文件的存放目录
pub fn bin_dir() -> crate::error::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| crate::error::AppError::Config("Could not determine data directory".to_string()))?
        .join("mcp-tunnel");
    Ok(dir)
}

/// cloudflared 二进制文件路径
pub fn bin_path() -> crate::error::Result<PathBuf> {
    Ok(bin_dir()?.join("cloudflared"))
}

/// 检查 cloudflared 是否存在，不存在则自动下载
pub async fn ensure_cloudflared() -> Result<PathBuf> {
    let path = bin_path()?;
    if path.exists() {
        return Ok(path);
    }

    download_cloudflared().await?;
    Ok(path)
}

/// 根据平台下载对应的 cloudflared 二进制文件
async fn download_cloudflared() -> Result<()> {
    use std::fs;

    let dir = bin_dir()?;
    fs::create_dir_all(&dir)?;

    let (url, is_tgz) = get_download_url()?;

    println!("Downloading cloudflared from {}...", url);

    let resp = reqwest::get(url).await.map_err(AppError::Http)?;
    let bytes = resp.bytes().await.map_err(AppError::Http)?;

    if is_tgz {
        // macOS ARM64 是 .tgz 格式，需要解压
        let tar_path = dir.join("cloudflared.tgz");
        let _bin = bin_path()?;
        fs::write(&tar_path, &bytes)?;

        let tar_gz = flate2::read::GzDecoder::new(std::fs::File::open(&tar_path)?);
        let mut archive = tar::Archive::new(tar_gz);
        archive.unpack(&dir)?;

        fs::remove_file(tar_path)?;
    } else {
        // Linux 等是直接的二进制文件
        fs::write(bin_path()?, &bytes)?;
    }

    // 设置可执行权限（Unix）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bp = bin_path()?;
        let mut perms = fs::metadata(&bp)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bp, perms)?;
    }

    println!("cloudflared downloaded to {:?}", bin_path()?);
    Ok(())
}

/// 根据当前平台返回下载 URL
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
