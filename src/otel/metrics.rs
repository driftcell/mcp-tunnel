use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Histogram, Meter};

static METER: OnceLock<Meter> = OnceLock::new();

static TOOL_CALLS_TOTAL: OnceLock<Counter<u64>> = OnceLock::new();
static TOOL_CALL_DURATION: OnceLock<Histogram<u64>> = OnceLock::new();
static TOOL_CALL_ERRORS_TOTAL: OnceLock<Counter<u64>> = OnceLock::new();
static LIST_TOOLS_TOTAL: OnceLock<Counter<u64>> = OnceLock::new();
static UPSTREAM_CONNECTIONS_TOTAL: OnceLock<Counter<u64>> = OnceLock::new();
static UPSTREAM_CONNECTION_ERRORS_TOTAL: OnceLock<Counter<u64>> = OnceLock::new();

pub fn init_meter() {
    let _ = METER.set(opentelemetry::global::meter("mcp-tunnel"));
}

fn get_meter() -> &'static Meter {
    METER.get().expect("meter not initialized")
}

pub fn tool_calls_total() -> &'static Counter<u64> {
    TOOL_CALLS_TOTAL.get_or_init(|| {
        get_meter()
            .u64_counter("mcp_tunnel.tool_calls.total")
            .with_description("Total number of tool calls")
            .build()
    })
}

pub fn tool_call_duration() -> &'static Histogram<u64> {
    TOOL_CALL_DURATION.get_or_init(|| {
        get_meter()
            .u64_histogram("mcp_tunnel.tool_call.duration_ms")
            .with_description("Tool call duration in milliseconds")
            .build()
    })
}

pub fn tool_call_errors_total() -> &'static Counter<u64> {
    TOOL_CALL_ERRORS_TOTAL.get_or_init(|| {
        get_meter()
            .u64_counter("mcp_tunnel.tool_call_errors.total")
            .with_description("Total number of tool call errors")
            .build()
    })
}

pub fn list_tools_total() -> &'static Counter<u64> {
    LIST_TOOLS_TOTAL.get_or_init(|| {
        get_meter()
            .u64_counter("mcp_tunnel.list_tools.total")
            .with_description("Total number of list_tools calls")
            .build()
    })
}

pub fn upstream_connections_total() -> &'static Counter<u64> {
    UPSTREAM_CONNECTIONS_TOTAL.get_or_init(|| {
        get_meter()
            .u64_counter("mcp_tunnel.upstream_connections.total")
            .with_description("Total number of upstream connections established")
            .build()
    })
}

pub fn upstream_connection_errors_total() -> &'static Counter<u64> {
    UPSTREAM_CONNECTION_ERRORS_TOTAL.get_or_init(|| {
        get_meter()
            .u64_counter("mcp_tunnel.upstream_connection_errors.total")
            .with_description("Total number of upstream connection errors")
            .build()
    })
}
