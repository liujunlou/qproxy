pub mod http;
pub mod tcp;

use crate::model::TrafficRecord;
use crate::options::Options;
use crate::errors::Error;
use std::sync::Arc;
use tracing::error;

pub struct ProxyServer {
    pub options: Arc<Options>,
}

impl ProxyServer {
    pub fn new(options: Options) -> Self {
        Self {
            options: Arc::new(options),
        }
    }

    pub async fn start(&self) -> Result<(), Error> {
        let http_server = self.options.clone();
        let tcp_server = self.options.clone();

        // 启动 HTTP 和 TCP 代理服务器
        tokio::try_join!(
            self::http::start_server(http_server),
            self::tcp::start_server(tcp_server)
        ).map_err(|e| {
            error!("Server error: {}", e);
            e
        })?;

        Ok(())
    }
} 