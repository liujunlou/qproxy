pub mod http;
pub mod tcp;

use crate::options::Options;
use crate::errors::Error;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub struct ProxyServer {
    pub options: Arc<Options>,
}

impl ProxyServer {
    pub fn new(options: Options) -> Self {
        Self {
            options: Arc::new(options),
        }
    }

    pub async fn start(&self) -> (JoinHandle<Result<(), Error>>, JoinHandle<Result<(), Error>>) {
        let http_server = self.options.clone();
        let tcp_server = self.options.clone();

        // 启动 HTTP 和 TCP 代理服务器
        let http_handle = tokio::spawn(self::http::start_server(http_server));
        let tcp_handle = tokio::spawn(self::tcp::start_server(tcp_server));

        (http_handle, tcp_handle)
    }

    pub async fn abort(&self, http_handle: JoinHandle<Result<(), Error>>, tcp_handle: JoinHandle<Result<(), Error>>) {
        // 中止 HTTP 服务
        http_handle.abort_handle().abort();

        // 中止 TCP 服务
        tcp_handle.abort_handle().abort();
    }
} 