# mcp-tunnel 架构与实现计划

## 1. 项目概述

mcp-tunnel 是一个 Rust 编写的 MCP (Model Context Protocol) 服务聚合与隧道工具。

它允许用户：
- 聚合多个上游 MCP 服务（HTTP 远程服务或本地 stdio 进程）
- 通过 Cloudflare Tunnel 将聚合后的 MCP 服务暴露到公网
- 通过交互式 TUI 管理服务、工具和隧道
- 精细化控制每个上游 MCP 的工具启用/禁用

### 1.1 核心概念

| 概念 | 说明 |
|------|------|
| **上游 (Upstream)** | 被聚合的 MCP 服务，可以是 HTTP 远程服务或本地 stdio 进程 |
| **隧道 (Tunnel)** | Cloudflare Tunnel，将本地 MCP 服务暴露到公网 |
| **工具过滤** | 对每个上游服务，可以独立启用/禁用其提供的 tools |
| **配置文件** | `config.toml`，存储所有服务配置和工具过滤规则 |

---

## 2. 技术选型

| 功能 | 依赖 |
|------|------|
| MCP 协议 | `rmcp` (官方 Rust SDK)，启用 `client`, `auth` features |
| 异步运行时 | `tokio` (full feature) |
| TUI 框架 | `ratatui` + `crossterm` |
| CLI 解析 | `clap` (derive feature) |
| 配置管理 | `serde` + `toml` |
| HTTP 客户端 | `reqwest` (用于 OAuth、下载 cloudflared) |
| 日志 | `tracing` + `tracing-subscriber` |
| 错误处理 | `thiserror` + `anyhow` |
| 目录管理 | `dirs` (获取系统配置目录) |

---

## 3. 模块架构

```
src/
├── main.rs              # 入口，CLI 解析，分发到各子命令
├── cli.rs               # clap derive 定义
├── config.rs            # 配置结构体、读写 config.toml
├── app.rs               # TUI 应用状态管理 (ratatui App)
├── tui/
│   ├── mod.rs           # TUI 启动和事件循环
│   ├── layout.rs        # 布局定义 (List + Detail + Status 面板)
│   ├── servers.rs       # MCP server 列表渲染与交互
│   ├── tools.rs         # 工具管理界面 (enable/disable)
│   └── tunnel.rs        # 隧道状态面板
├── mcp/
│   ├── mod.rs           # MCP 客户端管理
│   ├── upstream.rs      # 上游定义 (HTTP vs stdio)
│   ├── client.rs        # rmcp 客户端封装
│   ├── tool_filter.rs   # 工具过滤逻辑
│   └── oauth/
│       ├── mod.rs       # OAuth 模块入口
│       ├── store.rs     # FileCredentialStore 实现 rmcp CredentialStore
│       └── flow.rs      # PKCE 授权流程封装
├── server/
│   ├── mod.rs           # 聚合 MCP server 启动
│   ├── router.rs        # 请求路由到各上游
│   └── audit.rs         # 审计日志：拦截和记录所有 MCP 调用
├── tunnel/
│   ├── mod.rs           # Tunnel 管理入口
│   ├── binary.rs        # cloudflared 自动下载
│   ├── quick.rs         # Quick tunnel (TryCloudflare)
│   └── named.rs         # 具名 tunnel 管理
└── error.rs             # 错误类型定义
```

---

## 4. 配置文件设计

文件位置：当前目录下的 `config.toml`

```toml
# 上游 MCP 服务列表
[[servers]]
name = "notion"
type = "http"
url = "https://mcp.notion.com/mcp"
# 工具过滤
enabled_tools = []        # 空表示全部启用
disabled_tools = []       # 显式禁用的工具名

[[servers]]
name = "filesystem"
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/docs"]
enabled_tools = []
disabled_tools = []

# 隧道配置
[tunnel]
mode = "quick"      # "quick" | "named" | "disabled"
# named 模式下：
# name = "my-tunnel"

# 工具状态缓存（由 TUI 自动填充）
# 记录每个 server 发现到的 tools，用于离线展示
[[tool_cache]]
server = "notion"
tools = [
    { name = "search_pages", description = "...", enabled = true },
    { name = "create_page", description = "...", enabled = false },
]
```

---

## 5. CLI 设计

### 5.1 命令结构

```
mt                          # 默认：启动交互式 TUI
mt serve                    # 根据 config.toml 启动聚合服务（无 TUI）

# 服务管理
mt add <name> <url>         # 添加 HTTP 上游服务
mt add-stdio <name> <cmd>   # 添加 stdio 上游服务
mt remove <name>            # 移除服务

# Tunnel 管理
mt tunnel login             # Cloudflare 登录 (生成 cert.pem)
mt tunnel create <name>     # 创建具名 tunnel
mt tunnel delete <name>     # 删除具名 tunnel
mt tunnel list              # 列出已创建的 tunnel

# 其他
mt --config <path>          # 指定配置文件路径（默认 ./config.toml）
mt --version
```

### 5.2 Clap 定义示意

```rust
#[derive(Parser)]
#[command(name = "mt")]
struct Cli {
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the aggregated MCP server (no TUI)
    Serve,

    /// Add an HTTP upstream MCP server
    Add { name: String, url: String },

    /// Add a stdio upstream MCP server
    AddStdio { name: String, command: String, args: Vec<String> },

    /// Remove an upstream server
    Remove { name: String },

    /// Manage Cloudflare tunnel
    Tunnel {
        #[command(subcommand)]
        command: TunnelCommands,
    },
}
```

---

## 6. TUI 设计

### 6.1 主界面布局

```
┌─────────────────────────────────────────────────────────────┐
│  mcp-tunnel v0.1.0                    Tunnel: [Quick] Ready │
├──────────────────┬──────────────────────────────────────────┤
│                  │                                          │
│  [Servers]       │  Server: notion                          │
│  [Tools]         │  Type: HTTP                              │
│  [Tunnel]        │  URL: https://mcp.notion.com/mcp         │
│  [Logs]          │  Status: Connected                       │
│                  │                                          │
│  ▶ notion        │  OAuth: Authenticated                    │
│    filesystem    │  Tools: 12 total (10 enabled, 2 disabled)│
│    slack         │                                          │
│                  │  ─────────────────────────────────────── │
│                  │  Tools                                   │
│                  │  [✓] search_pages   Search Notion pages  │
│                  │  [✓] create_page    Create a new page    │
│                  │  [✗] delete_page    Delete a page        │
│                  │                                          │
├──────────────────┴──────────────────────────────────────────┤
│  q:quit  a:add  d:delete  Enter:edit tools  s:start serve   │
└─────────────────────────────────────────────────────────────┘
```

### 6.2 交互按键

| 按键 | 功能 |
|------|------|
| `↑/↓` 或 `j/k` | 在列表中上下移动 |
| `Enter` | 进入工具管理界面 |
| `a` | 添加新服务（弹出表单） |
| `d` | 删除选中服务 |
| `s` | 启动/停止聚合服务 |
| `t` | 启动/停止隧道 |
| `Tab` | 切换面板 (Servers / Tools / Tunnel / Logs) |
| `q` | 退出 |

### 6.3 工具管理界面

进入后可对选中服务的每个 tool 单独启用/禁用：
- 按 `Space` 切换 enable/disable
- 修改自动保存到 config.toml
- 实时生效（无需重启）

### 6.4 审计日志面板 (Audit Log)

在 `mt serve` 的 TUI 界面中，实时展示所有 MCP 调用的审计日志：

```
┌─────────────────────────────────────────────────────────────┐
│  [Servers] [Tools] [Tunnel] [Audit Log]                     │
├─────────────────────────────────────────────────────────────┤
│  2025/04/25 14:32:10  [CALL]   notion → search_pages        │
│                       args: { query: "meeting" }            │
│                       result: ok (3 pages)                  │
│  2025/04/25 14:32:15  [CALL]   filesystem → read_file       │
│                       args: { path: "/docs/report.md" }     │
│                       result: ok (1240 bytes)               │
│  2025/04/25 14:32:18  [ERROR]  notion → create_page         │
│                       args: { title: "New Page" }           │
│                       error: Unauthorized (401)             │
│  2025/04/25 14:33:02  [LIST]   All servers                  │
│                       result: 42 tools from 3 servers       │
├─────────────────────────────────────────────────────────────┤
│  r:refresh  c:clear  s:save to file  d:details              │
└─────────────────────────────────────────────────────────────┘
```

审计日志记录内容：

| 字段 | 说明 |
|------|------|
| `timestamp` | ISO 8601 格式时间 |
| `direction` | `[CALL]` / `[RESPONSE]` / `[LIST]` / `[ERROR]` |
| `upstream` | 目标上游服务名 |
| `tool` | 工具名（`tools/call` 时）|
| `args` | 调用参数摘要（JSON 截断显示）|
| `result` | 响应摘要（成功/失败、数据大小等）|
| `error` | 错误信息（如有）|
| `duration_ms` | 调用耗时 |

交互按键：
- `r`：刷新日志
- `c`：清空当前日志
- `s`：将日志保存到 `audit-YYYY-MM-DD.log` 文件
- `d` / `Enter`：查看某条日志的完整详情（展开完整请求/响应 JSON）
- `↑/↓`：滚动查看历史日志
- `f`：按 upstream 或 tool 名过滤日志

审计日志在内存中保留最近 1000 条，可通过配置调整。也可以选择持久化到文件。

---

## 7. Cloudflare Tunnel 设计

### 7.1 cloudflared 自动下载

首次使用 tunnel 功能时，检查 `~/.local/share/mcp-tunnel/cloudflared`：

```rust
async fn ensure_cloudflared() -> Result<PathBuf> {
    let bin_dir = dirs::data_local_dir()
        .unwrap()
        .join("mcp-tunnel");
    let bin_path = bin_dir.join("cloudflared");

    if bin_path.exists() {
        return Ok(bin_path);
    }

    // 根据平台选择下载 URL
    let url = match (os, arch) {
        ("macos", "aarch64") => ".../cloudflared-darwin-arm64.tgz",
        ("linux", "x86_64") => ".../cloudflared-linux-amd64",
        // ...
    };

    download_and_extract(url, &bin_dir).await?;
    Ok(bin_path)
}
```

### 7.2 Quick Tunnel (TryCloudflare)

无需登录，直接运行：

```bash
cloudflared tunnel --url http://localhost:<mcp-port>
```

特性：
- 自动生成 `trycloudflare.com` 临时域名
- 限制 200 并发请求
- 不支持 SSE
- 适合临时使用

### 7.3 具名 Tunnel (Named Tunnel)

需要预先登录：

```bash
# Step 1: 登录（浏览器 OAuth）
cloudflared tunnel login
# -> 在 ~/.cloudflared/cert.pem 保存凭证

# Step 2: 创建 tunnel
cloudflared tunnel create my-tunnel
# -> 在 ~/.cloudflared/<uuid>.json 保存 tunnel 凭证

# Step 3: 运行 tunnel
cloudflared tunnel run my-tunnel
# -> 需要配置 DNS 指向（可选）
```

mt 的封装：
- `mt tunnel login` -> 调用 `cloudflared tunnel login`
- `mt tunnel create <name>` -> 调用 `cloudflared tunnel create <name>`，记录 name 到 config.toml
- tunnel 运行时自动传递 `--credentials-file` 和 `--url`

### 7.4 Tunnel 状态管理

```rust
enum TunnelMode {
    Disabled,
    Quick { url: Option<String> },
    Named { name: String, url: Option<String> },
}

struct TunnelManager {
    child: Option<Child>,
    mode: TunnelMode,
}
```

---

## 8. MCP 客户端设计

### 8.1 上游类型

```rust
enum UpstreamType {
    Http { url: String },
    Stdio { command: String, args: Vec<String> },
}

struct UpstreamConfig {
    name: String,
    ty: UpstreamType,
    enabled_tools: HashSet<String>,   // 空 = 全部启用
    disabled_tools: HashSet<String>,  // 优先于 enabled_tools
}
```

### 8.2 客户端连接

HTTP 上游：
```rust
use rmcp::transport::StreamableHttpClientTransport;

let transport = StreamableHttpClientTransport::new(url);
let client = serve_client(MyClientHandler, transport).await?;
```

Stdio 上游：
```rust
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;

let cmd = Command::new("npx").args([...]);
let transport = TokioChildProcess::new(cmd)?;
let client = serve_client(MyClientHandler, transport).await?;
```

### 8.3 工具过滤

聚合服务在收到 `tools/list` 请求时：
1. 并发查询所有上游的 tools
2. 对每个上游，根据 `enabled_tools` / `disabled_tools` 过滤
3. 合并结果，添加前缀 `upstream_name__tool_name` 避免冲突
4. 返回过滤后的列表

收到 `tools/call` 请求时：
1. 解析工具名前缀，确定目标上游
2. 转发到对应上游客户端
3. 返回结果

---

## 9. OAuth 支持

### 9.1 配置方式

OAuth **无需配置**。用户只需提供服务器 URL：

```bash
mt add notion https://mcp.notion.com/mcp
```

无需 `client_id`、`client_secret`、authorize URL 或 token URL。所有 OAuth 元数据在运行时自动发现。

### 9.2 授权流程（使用 rmcp auth feature）

rmcp 内置 `auth` feature，提供完整的 OAuth 2.0 支持：

1. **发现（Discovery）**：`AuthorizationManager::discover_metadata()` 自动从服务器获取 OAuth 元数据（authorization_endpoint、token_endpoint、scopes 等）

2. **首次连接**：
   - 创建 `AuthorizationSession` 启动 PKCE 流程
   - 自动生成 code challenge/verifier 和 CSRF token
   - 打开浏览器到授权 URL
   - 启动本地 HTTP 回调服务器（如 localhost:9876）
   - 用户授权后，回调携带 code
   - `session.handle_callback()` 用 code 换取 access_token
   - 通过 `CredentialStore` 保存 token 到本地

3. **后续连接**：
   - `AuthClient` 包装 HTTP 客户端，自动注入 `Authorization: Bearer <token>`
   - token 过期时自动刷新（通过 `get_access_token()`）

4. **实现细节**：
   - 实现 `CredentialStore` trait 用于文件持久化（`~/.local/share/mcp-tunnel/oauth/<server_name>.json`）
   - 使用 `AuthClient<C>` 包装底层 HTTP 客户端，`StreamableHttpClientTransport::with_client()` 创建带认证的传输层
   - 公共客户端模式（无 client_secret），使用 PKCE（RFC 8252）

---

## 10. 聚合 Server 实现

### 10.1 架构

```
┌──────────────────────────────────────┐
│          mcp-tunnel server           │
│  ┌────────────────────────────────┐  │
│  │     HTTP Server (axum)         │  │
│  │    /mcp -> SSE endpoint        │  │
│  └────────────┬───────────────────┘  │
│               │                      │
│  ┌────────────▼───────────────────┐  │
│  │      MCP Server (rmcp)         │  │
│  │   - tools/list (聚合+过滤)      │  │
│  │   - tools/call (路由)           │  │
│  └────────────┬───────────────────┘  │
│               │                      │
│  ┌────────────▼───────────────────┐  │
│  │    Upstream Client Pool        │  │
│  │  ┌────────┐ ┌────────┐        │  │
│  │  │ Notion │ │ Filesys│ ...    │  │
│  │  │ (HTTP) │ │ (stdio)│        │  │
│  │  └────────┘ └────────┘        │  │
│  └────────────────────────────────┘  │
└──────────────────────────────────────┘
```

### 10.2 审计日志拦截

在聚合 Server 的请求处理链路中插入审计层：

```
Client Request
      │
      ▼
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Audit     │────▶│   Filter    │────▶│  Upstream   │
│  Logger     │     │   / Route   │     │   Client    │
└─────────────┘     └─────────────┘     └──────┬──────┘
      │                                        │
      │         ┌─────────────┐                │
      └────────▶│  Response   │◀───────────────┘
                │   Return    │
                └─────────────┘
```

审计日志通过 `tokio::sync::broadcast` 或 `mpsc` 通道发送到 TUI，实现实时展示。

```rust
struct AuditLog {
    timestamp: DateTime<Utc>,
    direction: AuditDirection,    // Call / Response / Error / List
    upstream: String,
    tool: Option<String>,
    args: Option<Value>,          // 请求参数
    result: Option<Value>,        // 响应结果
    error: Option<String>,        // 错误信息
    duration_ms: u64,
}

// 在 server/router.rs 中，每个 tools/call 和 tools/list 都经过审计
async fn handle_tool_call(
    audit: &AuditSender,
    upstream: &str,
    tool: &str,
    args: Value,
    client: &UpstreamClient,
) -> Result<Value> {
    let start = Instant::now();
    let log_id = audit.begin(AuditDirection::Call, upstream, tool, args.clone());

    match client.call_tool(tool, args).await {
        Ok(result) => {
            let duration = start.elapsed().as_millis() as u64;
            audit.end(log_id, Ok(&result), duration);
            Ok(result)
        }
        Err(e) => {
            let duration = start.elapsed().as_millis() as u64;
            audit.end(log_id, Err(&e), duration);
            Err(e)
        }
    }
}
```

### 10.3 启动流程

```
mt serve
  │
  ├─> 读取 config.toml
  ├─> 初始化所有上游客户端
  │    ├─> HTTP: 建立 StreamableHttpClientTransport 连接
  │    └─> stdio: 启动 TokioChildProcess
  ├─> 启动审计日志通道
  ├─> 启动本地 HTTP server（默认 127.0.0.1:0 或配置端口）
  ├─> 输出本地服务地址
  └─> 如果 tunnel 已配置：启动 tunnel，输出公网地址
```

---

## 11. 实现阶段

### Phase 1: 基础骨架
- [ ] 完善 Cargo.toml，添加所有依赖
- [ ] 实现 `config.rs`：定义配置结构体，读写 config.toml
- [ ] 实现 `cli.rs`：clap derive 定义
- [ ] 实现 `error.rs`：自定义错误类型
- [ ] `main.rs` 命令分发框架

### Phase 2: MCP 客户端
- [ ] 实现 `mcp/upstream.rs`：上游定义
- [ ] 实现 `mcp/client.rs`：基于 rmcp 的客户端封装
- [ ] 实现 `mcp/tool_filter.rs`：工具过滤逻辑
- [ ] 实现 `mt add` / `mt remove` 命令

### Phase 3: 聚合 Server
- [ ] 实现 `server/mod.rs`：基于 rmcp 的聚合 MCP server
- [ ] 实现 `server/router.rs`：tools/list 聚合与过滤、tools/call 路由
- [ ] 实现 `server/audit.rs`：审计日志拦截和记录
- [ ] 实现 `mt serve` 命令

### Phase 4: Cloudflare Tunnel
- [ ] 实现 `tunnel/binary.rs`：自动下载 cloudflared
- [ ] 实现 `tunnel/quick.rs`：Quick tunnel
- [ ] 实现 `tunnel/named.rs`：具名 tunnel + login
- [ ] 实现 `mt tunnel` 子命令

### Phase 5: TUI
- [ ] 实现 `app.rs`：TUI 状态管理
- [ ] 实现 `tui/mod.rs`：事件循环和渲染
- [ ] 实现 `tui/layout.rs`：界面布局
- [ ] 实现 `tui/servers.rs`：服务列表面板
- [ ] 实现 `tui/tools.rs`：工具管理面板
- [ ] 实现 `tui/tunnel.rs`：隧道状态面板
- [ ] 实现 `tui/audit_log.rs`：审计日志实时展示面板
- [ ] 默认 `mt` 启动 TUI

### Phase 6: OAuth（使用 rmcp auth feature）
- [ ] 实现 `mcp/oauth/store.rs`：`FileCredentialStore` 实现 rmcp 的 `CredentialStore` trait
- [ ] 实现 `mcp/oauth/flow.rs`：PKCE 流程封装，使用 `AuthorizationSession`
- [ ] 更新 `mcp/client.rs`：HTTP 上游使用 `AuthClient` 包装 + `AuthorizationManager`
- [ ] 自动发现 OAuth 元数据，无配置时触发浏览器授权
- [ ] Token 自动刷新和持久化

### Phase 7: 完善与测试
- [ ] 端到端测试：添加服务 -> 启动 serve -> 隧道 -> 工具调用
- [ ] 错误处理和边界情况
- [ ] 文档和示例

---

## 12. 关键依赖版本

```toml
[dependencies]
# 异步
async-trait = "0.1"
futures = "0.3"
tokio = { version = "1", features = ["full"] }

# MCP
rmcp = { version = "0.3", features = ["client", "server", "auth"] }

# TUI
ratatui = "0.29"
crossterm = "0.28"

# CLI
clap = { version = "4", features = ["derive"] }

# 序列化/配置
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# HTTP
reqwest = { version = "0.12", features = ["json", "stream"] }
axum = "0.8"

# 浏览器打开（OAuth 授权）
open = "5"

# 日志/错误
tracing = "0.1"
tracing-subscriber = "0.3"
thiserror = "2"
anyhow = "1"

# 工具
dirs = "6"
which = "7"
tempfile = "3"
tar = "0.4"        # 解压 cloudflared
flate2 = "1"       # gzip 解压
```

---

## 13. 运行示例

```bash
# 1. 添加一个 HTTP 上游
mt add notion https://mcp.notion.com/mcp

# 2. 添加一个 stdio 上游
mt add-stdio filesystem npx -y @modelcontextprotocol/server-filesystem ~/docs

# 3. 启动 TUI（默认命令 mt）
mt
# 在 TUI 中：
#   - 选中 filesystem，按 Enter 进入工具管理
#   - 禁用某些工具（按 Space）
#   - 按 s 启动聚合服务
#   - 按 t 启动 Quick tunnel
#   - 观察公网 URL

# 4. 无 TUI 直接启动服务
mt serve

# 5. Cloudflare 登录（用于具名 tunnel）
mt tunnel login
mt tunnel create my-tunnel

# 6. 编辑 config.toml 设置 tunnel mode = "named"
# 然后 TUI 中启动 tunnel 将使用具名 tunnel
```
