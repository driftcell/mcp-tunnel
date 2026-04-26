use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use tracing::{info, debug};
use crate::constants::DEFAULT_BIND_ADDR;
use crate::error::Result;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
    #[serde(default)]
    pub tunnel: TunnelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: UpstreamType,
    #[serde(default)]
    pub enabled_tools: BTreeSet<String>,
    #[serde(default)]
    pub disabled_tools: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum UpstreamType {
    Http { url: String },
    Stdio { command: String, #[serde(default)] args: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    #[serde(default = "default_tunnel_mode")]
    pub mode: TunnelMode,
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
}

fn default_tunnel_mode() -> TunnelMode {
    TunnelMode::Disabled
}

fn default_bind_addr() -> String {
    DEFAULT_BIND_ADDR.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelMode {
    Disabled,
    Quick,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCache {
    pub server: String,
    pub tools: Vec<ToolInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TunnelConfig {
    fn default() -> Self {
        TunnelConfig {
            mode: default_tunnel_mode(),
            bind_addr: default_bind_addr(),
        }
    }
}

impl TunnelConfig {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.bind_addr)
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        if !path.exists() {
            info!("Config file not found at {:?}, using defaults", path);
            return Ok(Config::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::error::AppError::Config(format!("Failed to read config file: {e}")))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| crate::error::AppError::Config(format!("Failed to parse config: {e}")))?;
        let mut seen_names = HashSet::new();
        let mut duplicates = Vec::new();
        for server in &config.servers {
            if let Err(e) = server.validate() {
                tracing::warn!("Invalid server config for '{}': {}", server.name, e);
            }
            if !seen_names.insert(server.name.clone()) {
                duplicates.push(server.name.clone());
            }
        }
        if !duplicates.is_empty() {
            return Err(crate::error::AppError::Config(format!(
                "Duplicate server names found: {:?}",
                duplicates
            )));
        }
        debug!("Loaded config with {} server(s)", config.servers.len());
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| crate::error::AppError::Config(format!("Failed to serialize config: {e}")))?;
        std::fs::write(path, content)
            .map_err(|e| crate::error::AppError::Config(format!("Failed to write config file: {e}")))?;
        info!("Config saved to {:?}", path);
        Ok(())
    }
}

impl ServerConfig {
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        if self.disabled_tools.contains(tool_name) {
            return false;
        }
        if self.enabled_tools.is_empty() {
            return true;
        }
        self.enabled_tools.contains(tool_name)
    }

    /// Validate that the server config is well-formed.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.name.trim().is_empty() {
            return Err(crate::error::AppError::Config(
                "Server name cannot be empty".to_string(),
            ));
        }
        if self.name.contains("__") {
            return Err(crate::error::AppError::Config(format!(
                "Server name '{}' contains reserved delimiter '__'",
                self.name
            )));
        }
        match &self.ty {
            UpstreamType::Http { url } => {
                let trimmed = url.trim();
                if trimmed.is_empty() {
                    return Err(crate::error::AppError::Config(
                        "HTTP URL cannot be empty".to_string(),
                    ));
                }
                match url::Url::parse(trimmed) {
                    Ok(parsed) => {
                        if parsed.host().is_none() {
                            return Err(crate::error::AppError::Config(format!(
                                "URL '{}' has no host",
                                trimmed
                            )));
                        }
                        let scheme = parsed.scheme();
                        if scheme != "http" && scheme != "https" {
                            return Err(crate::error::AppError::Config(format!(
                                "URL '{}' must use http:// or https://",
                                trimmed
                            )));
                        }
                    }
                    Err(e) => {
                        return Err(crate::error::AppError::Config(format!(
                            "URL '{}' is invalid: {}",
                            trimmed, e
                        )));
                    }
                }
            }
            UpstreamType::Stdio { command, .. } => {
                if command.trim().is_empty() {
                    return Err(crate::error::AppError::Config(
                        "Stdio command cannot be empty".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}
