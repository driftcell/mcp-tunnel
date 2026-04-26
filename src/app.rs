use crate::config::{Config, ServerConfig};
use crate::constants::TUI_MESSAGE_DURATION_SECS;
use crate::error::Result;
use crate::server::audit::AuditLog;
use crate::tunnel::quick::QuickTunnel;
use ratatui::widgets::ListState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tab {
    Servers,
    Tunnel,
    AuditLog,
}

pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub current_tab: Tab,
    pub should_quit: bool,

    // Servers tab
    pub server_list_state: ListState,
    pub selected_server: usize,

    // Tools overlay (shown in server detail panel)
    pub show_tools: bool,
    pub tool_list_state: ListState,
    pub selected_tool: usize,
    pub tools_for_server: Option<String>, // Server name currently displaying tools
    pub tools_folded: bool,               // 'f' toggles brief (folded) vs full view

    // Audit log
    pub audit_logs: Vec<AuditLog>,
    pub audit_scroll: usize,

    // Tunnel
    pub tunnel_url: Option<String>,
    pub quick_tunnel_running: bool,
    pub quick_tunnel: Option<QuickTunnel>,

    // Serve mode
    pub serve_running: bool,
    pub serve_cancel: Option<CancellationToken>,
    pub serve_audit_buffer: Arc<Mutex<Vec<AuditLog>>>,

    // Tool cache (in-memory only, not persisted)
    pub tool_cache: Vec<crate::config::ToolCache>,

    // Messages (transient bottom notification)
    pub message: Option<String>,
    pub message_time: Option<Instant>,

    // Add dialog
    pub show_add_dialog: bool,
    pub add_dialog_type: AddDialogType,
    pub add_dialog_fields: Vec<String>,
    pub add_dialog_focus: usize,

    // Edit mode flag (reuses add dialog UI)
    pub is_edit_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddDialogType {
    Http,
    Stdio,
}

impl App {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        let mut app = Self {
            config,
            config_path,
            current_tab: Tab::Servers,
            should_quit: false,
            server_list_state: ListState::default(),
            selected_server: 0,
            show_tools: false,
            tool_list_state: ListState::default(),
            selected_tool: 0,
            tools_for_server: None,
            tools_folded: false,
            audit_logs: Vec::new(),
            audit_scroll: 0,
            tunnel_url: None,
            quick_tunnel_running: false,
            quick_tunnel: None,
            serve_running: false,
            serve_cancel: None,
            serve_audit_buffer: Arc::new(Mutex::new(Vec::new())),
            tool_cache: Vec::new(),
            message: None,
            message_time: None,
            show_add_dialog: false,
            add_dialog_type: AddDialogType::Http,
            add_dialog_fields: vec![String::new(), String::new()],
            add_dialog_focus: 0,
            is_edit_mode: false,
        };
        if !app.config.servers.is_empty() {
            app.server_list_state.select(Some(0));
        }
        app
    }

    pub fn save_config(&mut self) -> Result<()> {
        self.config.save(&self.config_path)
    }

    pub fn next_server(&mut self) {
        if self.config.servers.is_empty() { return; }
        self.selected_server = (self.selected_server + 1) % self.config.servers.len();
        self.server_list_state.select(Some(self.selected_server));
    }

    pub fn prev_server(&mut self) {
        if self.config.servers.is_empty() { return; }
        if self.selected_server == 0 {
            self.selected_server = self.config.servers.len() - 1;
        } else {
            self.selected_server -= 1;
        }
        self.server_list_state.select(Some(self.selected_server));
    }

    pub fn next_tool(&mut self) {
        let tools_count = self.current_tools_count();
        if tools_count == 0 { return; }
        self.selected_tool = (self.selected_tool + 1) % tools_count;
        self.tool_list_state.select(Some(self.selected_tool));
    }

    pub fn prev_tool(&mut self) {
        let tools_count = self.current_tools_count();
        if tools_count == 0 { return; }
        if self.selected_tool == 0 {
            self.selected_tool = tools_count - 1;
        } else {
            self.selected_tool -= 1;
        }
        self.tool_list_state.select(Some(self.selected_tool));
    }

    fn current_tools_count(&self) -> usize {
        // Look up the tool count for the current server from tool_cache
        self.tools_for_server.as_ref().and_then(|name| {
            self.tool_cache.iter().find(|c| c.server == *name)
                .map(|c| c.tools.len())
        }).unwrap_or(0)
    }

    pub fn selected_server_config(&self) -> Option<&ServerConfig> {
        self.config.servers.get(self.selected_server)
    }

    pub fn set_message(&mut self, msg: String) {
        self.message = Some(msg);
        self.message_time = Some(Instant::now());
    }

    pub fn clear_message_if_expired(&mut self) {
        if let Some(time) = self.message_time
            && time.elapsed().as_secs() > TUI_MESSAGE_DURATION_SECS
        {
            self.message = None;
            self.message_time = None;
        }
    }

    pub fn next_tab(&mut self) {
        self.show_tools = false;
        self.current_tab = match self.current_tab {
            Tab::Servers => Tab::Tunnel,
            Tab::Tunnel => Tab::AuditLog,
            Tab::AuditLog => Tab::Servers,
        };
    }

    pub fn prev_tab(&mut self) {
        self.show_tools = false;
        self.current_tab = match self.current_tab {
            Tab::Servers => Tab::AuditLog,
            Tab::Tunnel => Tab::Servers,
            Tab::AuditLog => Tab::Tunnel,
        };
    }

    pub fn remove_selected_server(&mut self) -> Result<()> {
        if self.selected_server < self.config.servers.len() {
            self.config.servers.remove(self.selected_server);
            if self.config.servers.is_empty() {
                self.selected_server = 0;
                self.server_list_state.select(None);
            } else if self.selected_server >= self.config.servers.len() {
                self.selected_server = self.config.servers.len() - 1;
                self.server_list_state.select(Some(self.selected_server));
            }
            self.save_config()?;
        }
        Ok(())
    }

    pub fn toggle_selected_tool(&mut self) -> Result<()> {
        if let Some(server_name) = &self.tools_for_server {
            if let Some(config) = self.config.servers.iter_mut().find(|s| s.name == *server_name) {
                if let Some(cache) = self.tool_cache.iter().find(|c| c.server == *server_name) {
                    if let Some(tool) = cache.tools.get(self.selected_tool) {
                        if config.disabled_tools.contains(&tool.name) {
                            config.disabled_tools.remove(&tool.name);
                        } else {
                            config.disabled_tools.insert(tool.name.clone());
                        }
                        return self.save_config();
                    }
                }
            }
        }
        Ok(())
    }

    pub fn enter_tools_view(&mut self) {
        if let Some(server) = self.selected_server_config() {
            self.tools_for_server = Some(server.name.clone());
            self.show_tools = true;
            self.selected_tool = 0;
            self.tool_list_state.select(Some(0));
        }
    }
}
