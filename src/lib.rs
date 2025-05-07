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
} 