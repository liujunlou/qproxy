use crate::options::ProxyMode;
use prometheus::{register_counter, register_gauge, register_histogram, Counter, Gauge, Histogram};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub mod collector;

lazy_static::lazy_static! {
    // 用于记录HTTP请求处理时间的计时器
    pub static ref HTTP_REQUEST_TIMER: Histogram = register_histogram!(
        "qproxy_http_request_timer",
        "The total time taken to process HTTP requests"
    ).expect("Failed to register HTTP request timer");

    // 用于记录TCP请求处理时间的计时器
    pub static ref TCP_REQUEST_TIMER: Histogram = register_histogram!(
        "qproxy_tcp_request_timer",
        "The total time taken to process TCP requests"
    ).expect("Failed to register TCP request timer");

    // 用于记录总流量大小的计数器
    pub static ref TRAFFIC_SIZE_COUNTER: Gauge = register_gauge!(
        "qproxy_traffic_size_counter",
        "The total size of traffic recorded by qproxy"
    ).expect("Failed to register traffic size counter");

    // 系统指标：CPU使用率
    pub static ref CPU_USAGE_GAUGE: Gauge = register_gauge!(
        "qproxy_cpu_usage_gauge",
        "The current CPU usage of qproxy"
    ).expect("Failed to register CPU usage gauge");

    // 系统指标：内存使用率
    pub static ref MEMORY_USAGE_GAUGE: Gauge = register_gauge!(
        "qproxy_memory_usage_gauge",
        "The current memory usage of qproxy"
    ).expect("Failed to register memory usage gauge");

    // 系统指标：网络IO
    pub static ref NETWORK_IO_GAUGE: Gauge = register_gauge!(
        "qproxy_network_io_gauge",
        "The current network IO of qproxy"
    ).expect("Failed to register network IO gauge");

    pub static ref NETWORK_CONNECTIONS_GAUGE: Gauge = register_gauge!(
        "qproxy_network_connections_gauge",
        "The current number of network connections of qproxy"
    ).expect("Failed to register network connections gauge");

    // HTTP 服务端监控指标
    pub static ref HTTP_REQUESTS_TOTAL: Counter = register_counter!(
        "http_requests_total",
        "Total number of HTTP requests"
    ).expect("Failed to register HTTP requests counter");

    pub static ref HTTP_REQUEST_DURATION: Histogram = register_histogram!(
        "http_request_duration_seconds",
        "HTTP request duration in seconds"
    ).expect("Failed to register HTTP request duration histogram");

    pub static ref HTTP_ACTIVE_CONNECTIONS: Gauge = register_gauge!(
        "http_active_connections",
        "Number of active HTTP connections"
    ).expect("Failed to register HTTP active connections gauge");

    pub static ref HTTP_REQUEST_SIZE: Histogram = register_histogram!(
        "http_request_size_bytes",
        "HTTP request size in bytes"
    ).expect("Failed to register HTTP request size histogram");

    pub static ref HTTP_RESPONSE_SIZE: Histogram = register_histogram!(
        "http_response_size_bytes",
        "HTTP response size in bytes"
    ).expect("Failed to register HTTP response size histogram");

    pub static ref HTTP_ERRORS_TOTAL: Counter = register_counter!(
        "http_errors_total",
        "Total number of HTTP errors"
    ).expect("Failed to register HTTP errors counter");

    // gRPC 监控指标
    pub static ref GRPC_REQUESTS_TOTAL: Counter = register_counter!(
        "grpc_requests_total",
        "Total number of gRPC requests"
    ).expect("Failed to register gRPC requests counter");

    pub static ref GRPC_REQUEST_DURATION: Histogram = register_histogram!(
        "grpc_request_duration_seconds",
        "gRPC request duration in seconds"
    ).expect("Failed to register gRPC request duration histogram");

    pub static ref GRPC_ACTIVE_CONNECTIONS: Gauge = register_gauge!(
        "grpc_active_connections",
        "Number of active gRPC connections"
    ).expect("Failed to register gRPC active connections gauge");

    pub static ref GRPC_REQUEST_SIZE: Histogram = register_histogram!(
        "grpc_request_size_bytes",
        "gRPC request size in bytes"
    ).expect("Failed to register gRPC request size histogram");

    pub static ref GRPC_RESPONSE_SIZE: Histogram = register_histogram!(
        "grpc_response_size_bytes",
        "gRPC response size in bytes"
    ).expect("Failed to register gRPC response size histogram");

    pub static ref GRPC_ERRORS_TOTAL: Counter = register_counter!(
        "grpc_errors_total",
        "Total number of gRPC errors"
    ).expect("Failed to register gRPC errors counter");
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HealthStatus {
    pub status: ServiceStatus,
    pub mode: ProxyMode,
    pub components: ComponentStatus,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum ServiceStatus {
    Up,
    Down,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ComponentStatus {
    pub redis: ServiceStatus,
    pub recording: ServiceStatus,
    pub system: ServiceStatus,
}

pub struct MetricsCollector {
    health: Arc<RwLock<HealthStatus>>,
}

impl MetricsCollector {
    pub fn new(mode: ProxyMode) -> Self {
        Self {
            health: Arc::new(RwLock::new(HealthStatus {
                status: ServiceStatus::Up,
                mode,
                components: ComponentStatus {
                    redis: ServiceStatus::Up,
                    recording: ServiceStatus::Up,
                    system: ServiceStatus::Up,
                },
                last_check: chrono::Utc::now(),
                error: None,
            })),
        }
    }

    pub async fn update_health_status(&self, status: ServiceStatus, error: Option<String>) {
        let mut health = self.health.write().await;
        health.status = status;
        health.last_check = chrono::Utc::now();
        health.error = error;
    }

    pub async fn update_component_status(&self, component: &str, status: ServiceStatus) {
        let mut health = self.health.write().await;
        match component {
            "redis" => health.components.redis = status,
            "recording" => health.components.recording = status,
            "system" => health.components.system = status,
            _ => {}
        }
    }

    pub async fn get_health_status(&self) -> HealthStatus {
        self.health.read().await.clone()
    }
}

// 告警阈值常量
pub const ALERT_THRESHOLDS: AlertThresholds = AlertThresholds {
    tps: 1000.0,
    response_time: Duration::from_secs(1),
    error_rate: 0.01,
    recording_success_rate: 0.99,
    playback_success_rate: 0.95,
    sync_delay: Duration::from_secs(5),
    cpu_usage: 0.8,
    memory_usage: 0.85,
    disk_usage: 0.9,
};

pub struct AlertThresholds {
    pub tps: f64,
    pub response_time: Duration,
    pub error_rate: f64,
    pub recording_success_rate: f64,
    pub playback_success_rate: f64,
    pub sync_delay: Duration,
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub disk_usage: f64,
}
