use std::collections::HashMap;
use std::{net::SocketAddr, str::FromStr, sync::Arc};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::{get_shutdown_rx, ONCE_FILTER_CHAIN};
use crate::{
    errors::Error, model::TrafficRecord, options::Options, PLAYBACK_SERVICE, METRICS_COLLECTOR
};

// 启动http服务端
pub async fn start_server(options: Arc<Options>) -> Result<(), Error> {
    let addr = format!("{}:{}", options.http.host, options.http.port);
    let addr = SocketAddr::from_str(&addr).expect("invalid socket address");
    let listener = TcpListener::bind(addr).await?;
    info!("HTTP proxy server started at {}", addr);

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
                                            handle_request(req, options).await
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

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    options: Arc<Options>,
) -> Result<Response<Full<Bytes>>, Error> {
    // 处理同步请求
    match req.uri().path() {
        "/sync" => {
            let playback_service = PLAYBACK_SERVICE.lock().await;
            if let Some(playback_service) = playback_service.as_ref() {
                match req.method() {
                    // 获取需要同步的记录
                &http::Method::GET => {
                    // 获取 GET 请求的params参数
                    let uri = req.uri().clone();
                    let query_params: HashMap<_, _> = uri.query()
                        .map(|q| q.split('&')
                            .filter_map(|param| {
                                let parts: Vec<&str> = param.split('=').collect();
                                if parts.len() == 2 {
                                    Some((parts[0], parts[1]))
                                } else {
                                    None
                                }
                            })
                            .collect())
                        .unwrap_or_default();
    
                    let peer_id = query_params.get("peer_id").unwrap_or(&"default");
                    let shard_id = query_params.get("shard_id").unwrap_or(&"default");
                    // 区分互联网端和警务网
                    let from = query_params.get("from").unwrap_or(&"internet");
                    
                    let records = playback_service.get_records_for_sync(peer_id, shard_id).await?;
                    // 判断拉取端，如果是互联网端，则需要做数据过滤
                    let records = if from.eq_ignore_ascii_case("internet") {
                        let mut filtered_records = Vec::new();
                        for record in records {
                            let filtered = if options.http.filter_fields.is_some() {
                                ONCE_FILTER_CHAIN.read().await.filter(&record)
                            } else {
                                Ok(record)
                            }?;
                            filtered_records.push(filtered);
                        }
                        filtered_records
                    } else {
                        records
                    };
    
                    let json = serde_json::to_string(&records)
                        .map_err(|e| Error::Proxy(format!("Failed to serialize records: {}", e)))?;
                    
                    return Ok(Response::builder()
                        .status(http::StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(json)))
                        .unwrap());
                }
                &http::Method::POST => {
                    // 接收并同步记录
                    let body = req.collect().await?.to_bytes();
                    let records: Vec<TrafficRecord> = serde_json::from_slice(&body)
                        .map_err(|e| Error::Proxy(format!("Failed to parse records: {}", e)))?;
                    
                    playback_service.sync_from_peer("default", "default", records).await?;
                    
                    return Ok(Response::builder()
                        .status(http::StatusCode::OK)
                        .body(Full::new(Bytes::from("Sync completed")))
                        .unwrap());
                }
                _ => {
                    return Ok(Response::builder()
                        .status(http::StatusCode::METHOD_NOT_ALLOWED)
                        .body(Full::new(Bytes::from("Method not allowed")))
                        .unwrap());
                    }
                }
            } else {
                error!("Playback service not initialized");
                return Ok(Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from("Playback service not initialized")))
                    .unwrap());
            }
        }
        "/metrics" => {     // 返回prometheus metrics
            let metrics_collector = METRICS_COLLECTOR.lock().await;
            if let Some(metrics_collector) = metrics_collector.as_ref() {
                let metrics = match metrics_collector.get_metrics().await {
                        Ok(metrics) => metrics,
                        Err(e) => e.to_string(),
                    };
                    return Ok(Response::builder()
                        .status(http::StatusCode::OK)
                        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                        .body(Full::new(Bytes::from(metrics)))
                        .unwrap());
            } else {
                error!("Metrics collector not initialized");
                return Ok(Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from("Metrics collector not initialized")))
                    .unwrap());
            }
        }
        "/health" => {
            let metrics_collector = METRICS_COLLECTOR.lock().await;
            if let Some(metrics_collector) = metrics_collector.as_ref() {
                let health = match metrics_collector.get_health().await {
                    Ok(health) => health,
                    Err(e) => e.to_string(),
                };
                return Ok(Response::builder()
                        .status(http::StatusCode::OK)
                        .body(Full::new(Bytes::from(health)))
                        .unwrap());
            } else {
                error!("Metrics collector not initialized");
                return Ok(Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from("Metrics collector not initialized")))
                    .unwrap());
            }
        }
        _ => {  // 处理其他请求
            return Ok(Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("Not found")))
                .unwrap());
        }
    }
}
