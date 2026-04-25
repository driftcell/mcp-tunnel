mod app;
mod cli;
mod config;
mod constants;
mod error;
mod mcp;
mod server;
mod tui;
mod tunnel;

use clap::Parser;
use cli::{Cli, Commands, TunnelCommands};
use config::Config;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let log_path = init_file_logger()?;
    eprintln!("Logs written to: {}", log_path.display());

    let cli = Cli::parse();
    let config_path = &cli.config;

    match cli.command {
        None => {
            let config = Config::load(config_path)?;
            tui::run_tui(config, config_path.clone()).await?;
            Ok(())
        }
        Some(Commands::Serve) => {
            info!("Starting MCP Tunnel server");
            let config = Config::load(config_path)?;
            server::router::start_server(&config).await?;
            Ok(())
        }
        Some(Commands::Add { name, url }) => {
            info!("Adding HTTP upstream: {} -> {}", name, url);
            let mut config = Config::load(config_path)?;
            let server = config::ServerConfig {
                name,
                ty: config::UpstreamType::Http { url },
                enabled_tools: Default::default(),
                disabled_tools: Default::default(),
            };
            server.validate()
                .map_err(|e| anyhow::anyhow!("Invalid server config: {}", e))?;
            config.servers.push(server);
            config.save(config_path)?;
            println!("Added HTTP upstream.");
            Ok(())
        }
        Some(Commands::AddStdio {
            name,
            command,
            args,
        }) => {
            info!("Adding stdio upstream: {} -> {} {:?}", name, command, args);
            let mut config = Config::load(config_path)?;
            let server = config::ServerConfig {
                name,
                ty: config::UpstreamType::Stdio { command, args },
                enabled_tools: Default::default(),
                disabled_tools: Default::default(),
            };
            server.validate()
                .map_err(|e| anyhow::anyhow!("Invalid server config: {}", e))?;
            config.servers.push(server);
            config.save(config_path)?;
            println!("Added stdio upstream.");
            Ok(())
        }
        Some(Commands::Remove { name }) => {
            info!("Removing upstream: {}", name);
            let mut config = Config::load(config_path)?;
            config.servers.retain(|s| s.name != name);
            config.save(config_path)?;
            println!("Removed upstream '{name}'.");
            Ok(())
        }
        Some(Commands::ClearToken { name }) => {
            info!("Clearing OAuth token for server: {}", name);
            let store = mcp::oauth::FileCredentialStore::new(&name)
                .map_err(|e| anyhow::anyhow!("Failed to create credential store: {}", e))?;
            store
                .clear()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to clear token: {}", e))?;
            println!("Cleared OAuth token for server '{}'.", name);
            Ok(())
        }
        Some(Commands::Tunnel { command }) => {
            match command {
                TunnelCommands::Login => {
                    info!("Tunnel login");
                    tunnel::named::login().await?;
                }
                TunnelCommands::Create { name } => {
                    info!("Creating tunnel: {}", name);
                    tunnel::named::create_tunnel(&name).await?;
                }
                TunnelCommands::Delete { name } => {
                    info!("Deleting tunnel: {}", name);
                    tunnel::named::delete_tunnel(&name).await?;
                }
                TunnelCommands::List => {
                    info!("Listing tunnels");
                    tunnel::named::list_tunnels().await?;
                }
            }
            Ok(())
        }
    }
}

fn data_dir() -> anyhow::Result<PathBuf> {
    dirs::data_local_dir()
        .map(|p| p.join("mcp-tunnel"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))
}

fn init_file_logger() -> anyhow::Result<PathBuf> {
    let log_dir = data_dir()?;
    std::fs::create_dir_all(&log_dir)?;

    let log_path = log_dir.join("mcp-tunnel.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let log_path_for_closure = log_path.clone();
    let shared_log = std::sync::Arc::new(std::sync::Mutex::new(log_file));

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(move || {
            let guard = shared_log.lock().unwrap();
            guard.try_clone().unwrap_or_else(|_| {
                // Fallback: reopen the log file if clone fails (e.g., FD limit hit)
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path_for_closure)
                    .unwrap()
            })
        })
        .init();

    Ok(log_path)
}
