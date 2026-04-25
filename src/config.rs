use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use tracing::{info, debug};
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
    pub name: Option<String>,
}

fn default_tunnel_mode() -> TunnelMode {
    TunnelMode::Disabled
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelMode {
    Disabled,
    Quick,
    Named,
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
            name: None,
        }
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
        for server in &config.servers {
            if let Err(e) = server.validate() {
                tracing::warn!("Invalid server config for '{}': {}", server.name, e);
            }
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
                if url.trim().is_empty() {
                    return Err(crate::error::AppError::Config(
                        "HTTP URL cannot be empty".to_string(),
                    ));
                }
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err(crate::error::AppError::Config(format!(
                        "URL '{}' must start with http:// or https://",
                        url
                    )));
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
