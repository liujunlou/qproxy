pub mod http;
pub mod tcp;
pub mod tcp_protobuf_server;

use crate::filter::response_filter::ResponseFilter;
use crate::options::Options;
use crate::ONCE_FILTER_CHAIN;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::error;
use serde::{Deserialize, Serialize};

pub struct ProxyServer {
    options: Arc<Options>,
}

impl ProxyServer {
    pub fn new(options: Options) -> Self {
        Self {
            options: Arc::new(options),
        }
    }

    pub async fn start(&self) -> (JoinHandle<()>, JoinHandle<()>) {
        let http_server = self.options.clone();
        let tcp_server = self.options.clone();
        
        // 启动 HTTP 和 TCP 代理服务器
        let http_handle = tokio::spawn(async move {
            if let Err(e) = self::http::start_server(http_server.clone()).await {
                error!("HTTP server failed to start: {}", e);
                std::process::exit(1);
            }
        });
        let tcp_handle = tokio::spawn(async move {
            if let Err(e) = self::tcp::start_server(tcp_server.clone()).await {
                error!("TCP server failed to start: {}", e);
                std::process::exit(1);
            }
        });

        // 添加 HTTP 代理服务器的过滤器
        ONCE_FILTER_CHAIN.write().await.add_filter(Box::new(ResponseFilter::new(self.options.http.filter_fields.clone())));

        (http_handle, tcp_handle)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Response {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn new(code: i32, message: String, data: Option<serde_json::Value>) -> Self {
        Self { code, message, data }
    }

    pub fn success(&self) -> bool {
        self.code == 200
    }

    pub fn success_with_message(message: &str) -> Self {
        Self { code: 200, message: message.to_string(), data: None }
    }

    pub fn success_with_data(data: serde_json::Value) -> Self {
        Self { code: 200, message: String::new(), data: Some(data) }
    }

    pub fn failed(message: &str) -> Self {
        Self { code: 500, message: message.to_string(), data: None }
    }
}
