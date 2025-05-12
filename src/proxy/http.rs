use std::collections::HashMap;
use std::{net::SocketAddr, str::FromStr, sync::Arc};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::{client::legacy::Builder, rt::{TokioExecutor, TokioIo}};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::{
    errors::Error, get_playback_service, model::TrafficRecord, options::{Options, ProxyMode}
};

use super::filter::ONCE_FILTER_CHAIN;

// 启动http服务端
pub async fn start_server(options: Arc<Options>) -> Result<(), Error> {
    let addr = format!("{}:{}", options.http.host, options.http.port);
    let addr = SocketAddr::from_str(&addr).expect("invalid socket address");
    let listener = TcpListener::bind(addr).await?;
    let executor = Arc::new(TokioExecutor::new());
    info!("HTTP proxy server started at {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let options = options.clone();
        let executor = executor.clone();

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(
                    io,
                    service_fn(|req| handle_request(req, options.clone(), executor.clone())),
                )
                .with_upgrades()
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    options: Arc<Options>,
    executor: Arc<TokioExecutor>,
) -> Result<Response<Full<Bytes>>, Error> {
    // 处理同步请求
    if req.uri().path() == "/sync" {
        let playback_service = get_playback_service().await?;
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
                let from = query_params.get("from").unwrap_or(&"default");
                
                let records = playback_service.get_records_for_sync(peer_id, shard_id).await?;
                // 判断拉取端，如果是互联网端，则需要做数据过滤
                let records = if from.eq_ignore_ascii_case("internet") {
                    let mut filtered_records = Vec::new();
                    for record in records {
                        let filtered = if options.http.filter_fields.is_some() {
                            ONCE_FILTER_CHAIN.lock().await.filter(&record)
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
    }
    // 处理其他请求
    return Ok(Response::builder()
        .status(http::StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from("Not found")))
        .unwrap());
}
