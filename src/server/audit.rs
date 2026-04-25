use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum AuditDirection {
    Call,
    Response,
    List,
    Error,
}

#[derive(Debug, Clone)]
pub struct AuditLog {
    pub timestamp: DateTime<Utc>,
    pub direction: AuditDirection,
    pub upstream: String,
    pub tool: Option<String>,
    pub args: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// 审计通道：用于发送和接收审计日志
pub struct AuditChannel {
    pub sender: mpsc::Sender<AuditLog>,
    pub receiver: mpsc::Receiver<AuditLog>,
}

impl AuditChannel {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::channel(capacity);
        Self { sender, receiver }
    }
}

/// 审计记录器：提供便捷的日志记录方法
#[derive(Clone)]
pub struct AuditLogger {
    sender: mpsc::Sender<AuditLog>,
}

impl AuditLogger {
    pub fn new(sender: mpsc::Sender<AuditLog>) -> Self {
        Self { sender }
    }

    pub async fn log_call(
        &self,
        upstream: String,
        tool: Option<String>,
        args: Option<Value>,
    ) {
        let log = AuditLog {
            timestamp: Utc::now(),
            direction: AuditDirection::Call,
            upstream,
            tool,
            args,
            result: None,
            error: None,
            duration_ms: 0,
        };
        if self.sender.send(log).await.is_err() {
            tracing::warn!("Audit log channel closed; log entry dropped");
        }
    }

    pub async fn log_response(
        &self,
        upstream: String,
        tool: Option<String>,
        result: Option<Value>,
        duration_ms: u64,
    ) {
        let log = AuditLog {
            timestamp: Utc::now(),
            direction: AuditDirection::Response,
            upstream,
            tool,
            args: None,
            result,
            error: None,
            duration_ms,
        };
        if self.sender.send(log).await.is_err() {
            tracing::warn!("Audit log channel closed; log entry dropped");
        }
    }

    pub async fn log_error(
        &self,
        upstream: String,
        tool: Option<String>,
        error: String,
        duration_ms: u64,
    ) {
        let log = AuditLog {
            timestamp: Utc::now(),
            direction: AuditDirection::Error,
            upstream,
            tool,
            args: None,
            result: None,
            error: Some(error),
            duration_ms,
        };
        if self.sender.send(log).await.is_err() {
            tracing::warn!("Audit log channel closed; log entry dropped");
        }
    }

    pub async fn log_list(
        &self,
        upstream: String,
        tool_count: usize,
    ) {
        let log = AuditLog {
            timestamp: Utc::now(),
            direction: AuditDirection::List,
            upstream,
            tool: Some(format!("{} tools", tool_count)),
            args: None,
            result: None,
            error: None,
            duration_ms: 0,
        };
        if self.sender.send(log).await.is_err() {
            tracing::warn!("Audit log channel closed; log entry dropped");
        }
    }
}
