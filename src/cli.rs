use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mt")]
#[command(about = "MCP Tunnel - Aggregate and tunnel MCP services")]
#[command(version)]
pub struct Cli {
    #[arg(short, long, default_value = "config.toml")]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the aggregated MCP server (no TUI)
    Serve,

    /// Add an HTTP upstream MCP server
    Add {
        name: String,
        url: String,
    },

    /// Add a stdio upstream MCP server
    AddStdio {
        name: String,
        command: String,
        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Remove an upstream server
    Remove {
        name: String,
    },

    /// Clear saved OAuth token for a server
    ClearToken {
        name: String,
    },

    /// Manage Cloudflare tunnel
    Tunnel {
        #[command(subcommand)]
        command: TunnelCommands,
    },
}

#[derive(Subcommand)]
pub enum TunnelCommands {
    /// Login to Cloudflare
    Login,
    /// Create a named tunnel
    Create {
        name: String,
    },
    /// Delete a named tunnel
    Delete {
        name: String,
    },
    /// List all tunnels
    List,
}
