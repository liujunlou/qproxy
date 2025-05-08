use std::{net::SocketAddr, str::FromStr, sync::Arc};
use std::collections::HashMap;

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
                
                let records = playback_service.get_records_for_sync(peer_id, shard_id).await?;
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

    match options.mode {
        ProxyMode::Record => {
            // Save request info before consuming
            let method = req.method().clone();
            let uri = req.uri().clone();
            let headers = req.headers().clone();
            let params = uri.query()
                .map(|s| s.split('&')
                    .filter_map(|param| {
                        let parts: Vec<&str> = param.split('=').collect();
                        if parts.len() == 2 {
                            Some((parts[0].to_string(), parts[1].to_string()))
                        } else {
                            None
                        }
                    })
                    .collect());
            
            // 获取请求体
            let body_bytes = req.collect().await?.to_bytes();
            
            // 构建转发请求
            let forwarded_req = Request::builder()
                .method(method.clone())
                .uri(format!("{}{}", options.http.downstream, uri.path_and_query().map(|x| x.as_str()).unwrap_or("")))
                .body(Full::new(body_bytes.clone()))
                .unwrap();

            // 发送请求到下游服务器并获取响应
            let client = Builder::new((*executor).clone())
                .pool_idle_timeout(std::time::Duration::from_secs(30))
                .build_http();
            let resp = client.request(forwarded_req).await?;
            
            // 获取响应体
            let (parts, body) = resp.into_parts();
            let body_bytes = body.collect().await?.to_bytes();
            
            // 创建流量记录
            let record = TrafficRecord::new_http(
                method.to_string(),
                uri.to_string(),
                params,
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect(),
                body_bytes.to_vec(),
                parts.status.as_u16(),
                parts.headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect(),
                body_bytes.to_vec(),
            );

            // 保存记录
            get_playback_service().await?.add_record(record).await?;
            
            // 构建响应
            let response = Response::from_parts(parts, Full::new(body_bytes));
            
            Ok(response)
        }
        ProxyMode::Playback => {
            Ok(get_playback_service().await?.playback(req).await)
        }
        ProxyMode::Forward => {
            // Save request info before consuming
            let method = req.method().clone();
            let uri = req.uri().clone();
            
            // 获取请求体
            let body_bytes = req.collect().await?.to_bytes();
            
            // 构建转发请求
            let forwarded_req = Request::builder()
                .method(method)
                .uri(format!("{}{}", options.http.downstream, uri.path_and_query().map(|x| x.as_str()).unwrap_or("")))
                .body(Full::new(body_bytes.clone()))
                .unwrap();

            // 发送请求到下游服务器并获取响应
            let client = Builder::new((*executor).clone())
                .pool_idle_timeout(std::time::Duration::from_secs(30))
                .build_http();
            let resp = client.request(forwarded_req).await?;
            
            // 获取响应体
            let (parts, body) = resp.into_parts();
            let body_bytes = body.collect().await?.to_bytes();
            
            // 构建响应
            let response = Response::from_parts(parts, Full::new(body_bytes));
            
            Ok(response)
        }
    }
}
