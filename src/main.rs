mod app;
mod cli;
mod config;
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
            config.servers.push(config::ServerConfig {
                name,
                ty: config::UpstreamType::Http { url },
                enabled_tools: Default::default(),
                disabled_tools: Default::default(),
            });
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
            config.servers.push(config::ServerConfig {
                name,
                ty: config::UpstreamType::Stdio { command, args },
                enabled_tools: Default::default(),
                disabled_tools: Default::default(),
            });
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
            let store = mcp::oauth::FileCredentialStore::new(&name);
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

fn init_file_logger() -> anyhow::Result<PathBuf> {
    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mcp-tunnel");
    std::fs::create_dir_all(&log_dir)?;

    let log_path = log_dir.join("mcp-tunnel.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(move || log_file.try_clone().expect("failed to clone log file"))
        .init();

    Ok(log_path)
}
