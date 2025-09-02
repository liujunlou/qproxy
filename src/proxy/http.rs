use std::{net::SocketAddr, sync::Arc};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::monitor::{
    HTTP_ACTIVE_CONNECTIONS, HTTP_ERRORS_TOTAL, HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION,
    HTTP_REQUEST_SIZE, HTTP_RESPONSE_SIZE,
};
use crate::{api, get_shutdown_rx};
use crate::{errors::Error, options::Options};
use std::time::Instant;

// 启动http服务端
pub async fn start_server(options: Arc<Options>) -> Result<(), Error> {
    let addr = format!("{}:{}", options.http.host, options.http.port)
        .parse::<SocketAddr>()
        .map_err(|e| Error::Config(e.to_string()))?;
    let listener = TcpListener::bind(addr).await?;
    info!("HTTP proxy server listening on {}", addr);

    let mut shutdown_rx = get_shutdown_rx().await;

    loop {
        let options = options.clone();

        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);
                        let options = options.clone();

                        tokio::task::spawn(async move {
                            if let Err(err) = http1::Builder::new()
                                .preserve_header_case(true)
                                .title_case_headers(true)
                                .serve_connection(
                                    io,
                                    service_fn(move |req| {
                                        let options = options.clone();
                                        async move {
                                            let start_time = Instant::now();
                                            info!("Received request: {:?}", req);
                                            let result = api::handle_api_request(req, options).await;

                                            // 记录请求完成
                                            HttpMetricsMiddleware::record_request_complete(start_time, &result);

                                            result
                                        }
                                    }),
                                )
                                .with_upgrades()
                                .await
                            {
                                error!("Error serving connection: {:?}", err);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Error accepting connection: {}", e);
                        continue;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("HTTP server received shutdown signal");
                // 等待所有连接处理完成
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                break;
            }
        }
    }

    info!("HTTP server shutdown complete");
    Ok(())
}

/// HTTP 监控中间件
pub struct HttpMetricsMiddleware;

impl HttpMetricsMiddleware {
    /// 记录请求开始
    pub fn record_request_start(req: &Request<hyper::body::Incoming>) {
        // 增加活跃连接数
        HTTP_ACTIVE_CONNECTIONS.inc();

        // 记录请求大小（如果有 Content-Length 头）
        if let Some(content_length) = req.headers().get("content-length") {
            if let Ok(size) = content_length.to_str().unwrap_or("0").parse::<f64>() {
                HTTP_REQUEST_SIZE.observe(size);
            }
        }
    }

    /// 记录请求完成
    pub fn record_request_complete(
        start_time: Instant,
        result: &Result<Response<Full<Bytes>>, Error>,
    ) {
        // 记录请求处理时间
        let duration = start_time.elapsed().as_secs_f64();
        HTTP_REQUEST_DURATION.observe(duration);

        // 记录请求结果
        match result {
            Ok(response) => {
                HTTP_REQUESTS_TOTAL.inc();

                // 记录响应大小
                if let Some(content_length) = response.headers().get("content-length") {
                    if let Ok(size) = content_length.to_str().unwrap_or("0").parse::<f64>() {
                        HTTP_RESPONSE_SIZE.observe(size);
                    }
                }
            }
            Err(_) => {
                HTTP_REQUESTS_TOTAL.inc();
                HTTP_ERRORS_TOTAL.inc();
            }
        }

        // 减少活跃连接数
        HTTP_ACTIVE_CONNECTIONS.dec();
    }
}
