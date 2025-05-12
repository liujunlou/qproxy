//! QProxy库 - 支持流量录制和回放的代理服务器
//! 
//! 本库实现了一个功能完整的代理服务器，支持以下特性：
//! - HTTP/HTTPS 代理
//! - TCP 代理（可选 TLS）
//! - 流量录制
//! - 流量回放
//! - 跨可用区部署
//! - MQTT客户端支持

pub mod api;
pub mod errors;
pub mod logger;
pub mod model;
pub mod options;
pub mod playback;
pub mod proxy;
pub mod record;
pub mod service_discovery;
pub mod sync;
pub mod mqtt_client {
    pub mod message;
    pub mod client;
    pub mod codec;
    pub mod crypto;
}

use once_cell::sync::Lazy;
use playback::PlaybackService;
use rustls::ServerConfig;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio::sync::Mutex;

// 重新导出main.rs中的静态变量和函数
pub static PLAYBACK_SERVICE: Lazy<Arc<Mutex<Option<Arc<PlaybackService>>>>> = 
    Lazy::new(|| Arc::new(Mutex::new(None)));

pub async fn get_playback_service() -> Result<Arc<PlaybackService>, errors::Error> {
    let service = PLAYBACK_SERVICE.lock().await;
    match service.as_ref() {
        Some(service) => Ok(service.clone()),
        None => Err(errors::Error::ServiceError("Playback service not initialized".to_string())),
    }
}

pub fn load_tls(cert_path: &str, key_path: &str) -> Arc<ServerConfig> {
    let cert_file = File::open(cert_path).expect("Failed to open certificate file");
    let key_file = File::open(key_path).expect("Failed to open private key file");
    
    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);
    
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .expect("Failed to parse certificate");
        
    let key = rustls_pemfile::private_key(&mut key_reader)
        .expect("Failed to parse private key")
        .expect("No private key found");
        
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("Failed to configure TLS server");
        
    Arc::new(server_config)
} 