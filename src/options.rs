use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

use crate::errors::Error;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Options {
    // 节点的角色
    pub mode: ProxyMode,
    // http代理节点
    pub http: HttpOptions,
    // tcp代理节点
    pub tcp: TcpOptions,
    // 流量回放服务节点
    pub peer: Option<PeerOptions>,
    // 服务发现
    pub service_discovery: ServiceDiscoveryOptions,
    // 同步
    pub sync: SyncOptions,
    // redis
    pub redis: RedisOptions,
    // 日志
    pub logging: LoggingOptions,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncOptions {
    pub enabled: bool,
    pub shards: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HttpOptions {
    pub host: String,
    pub port: u16,
    pub downstream: String,
    pub filter_fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TcpOptions {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub downstream: Vec<String>,
    pub tls: Option<TlsOptions>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TlsOptions {
    pub tls_cert: String,
    pub tls_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeerOptions {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ProxyMode {
    // 流量录制
    Record,
    // 流量回放
    Playback,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServiceDiscoveryOptions {
    pub provider: ServiceDiscoveryProvider,
    pub config: ServiceDiscoveryConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ServiceDiscoveryProvider {
    #[serde(rename = "static")]
    Static,
    #[serde(rename = "zookeeper")]
    Zookeeper,
    #[serde(rename = "kubernetes")]
    Kubernetes,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServiceDiscoveryConfig {
    pub static_services: Option<Vec<ServiceConfig>>,
    pub zookeeper: Option<ZookeeperConfig>, 
    pub kubernetes: Option<KubernetesConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZookeeperConfig {
    pub hosts: Vec<String>,
    pub base_path: String,
    pub timeout: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KubernetesConfig {
    pub namespace: String,
    pub service_account_token_path: Option<String>,
    pub api_server: Option<String>,
    pub label_selector: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoggingOptions {
    pub level: String,
    pub directory: String,
    pub file_name_pattern: String,
    pub rotation: LogRotationOptions,
    pub format: LogFormatOptions,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogRotationOptions {
    pub max_size_mb: u64,
    pub max_files: u32,
    pub compress: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogFormatOptions {
    pub timestamp: bool,
    pub level: bool,
    pub target: bool,
    pub thread_id: bool,
    pub file: bool,
    pub line_number: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RedisOptions {
    pub url: String,
    pub pool_size: Option<u32>,
    pub connection_timeout: Option<u64>,
    pub retry_count: Option<u32>,
}

impl Options {
    // 加载配置
    pub fn new() -> Result<Self, Error> {
        let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.json".to_string());
        let config_str = fs::read_to_string(config_path)?;
        serde_json::from_str(&config_str)
            .map_err(|e|Error::Config(e.to_string()))
    }
} 

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: ProxyMode::Record,
            http: HttpOptions {
                host: "0.0.0.0".to_string(),
                port: 8080,
                downstream: "http://localhost:8080".to_string(),
                filter_fields: None,
            },
            tcp: TcpOptions {
                enabled: false,
                host: "0.0.0.0".to_string(),
                port: 8080,
                downstream: vec![],
                tls: None,
            },
            peer: None,
            service_discovery: ServiceDiscoveryOptions {
                provider: ServiceDiscoveryProvider::Static,
                config: ServiceDiscoveryConfig {
                    static_services: None,
                    zookeeper: None,
                    kubernetes: None,
                },
            },
            sync: SyncOptions {
                enabled: false,
                shards: 1,
            },
            redis: RedisOptions {
                url: "redis://localhost:6379".to_string(),
                pool_size: None,
                connection_timeout: None,
                retry_count: None,
            },
            logging: LoggingOptions {
                level: "info".to_string(),
                directory: "logs".to_string(),
                file_name_pattern: "log-%Y-%m-%d.log".to_string(),
                rotation: LogRotationOptions {
                    max_size_mb: 100,
                    max_files: 10,
                    compress: true,
                },
                format: LogFormatOptions {
                    timestamp: true,
                    level: true,
                    target: true,
                    thread_id: true,
                    file: true,
                    line_number: true,
                },
            },
        }
    }
}