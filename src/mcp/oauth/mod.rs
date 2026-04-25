pub mod store;
pub mod flow;

pub use store::FileCredentialStore;
pub use flow::run_pkce_flow;

/// 本地 OAuth 回调监听地址（host:port）。
pub const OAUTH_CALLBACK_ADDR: &str = "127.0.0.1:9876";
/// 本地 OAuth 回调完整 URL，用于 redirect_uri。
pub const OAUTH_CALLBACK_URL: &str = "http://127.0.0.1:9876/callback";
