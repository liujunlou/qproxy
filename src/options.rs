use serde::{Deserialize, Serialize};
use std::fs;
use std::collections::HashMap;

use crate::errors::Error;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Options {
    pub http: HttpOptions,
    pub tcp: TcpOptions,
    pub mode: ProxyMode,
    pub peer: Option<PeerOptions>,
    pub service_discovery: ServiceDiscoveryOptions,
    pub logging: LoggingOptions,
    pub redis: RedisOptions,
    pub sync: SyncOptions,
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
    Record,
    Playback,
    Forward,
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