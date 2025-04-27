use std::{net::SocketAddr, str::FromStr, sync::Arc};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::{client::legacy::Builder, rt::{TokioExecutor, TokioIo}};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::{
    api,
    errors::Error,
    model::TrafficRecord,
    options::{Options, ProxyMode},
    playback::PlaybackService,
};

static PLAYBACK_SERVICE: once_cell::sync::Lazy<Arc<PlaybackService>> = once_cell::sync::Lazy::new(|| {
    Arc::new(PlaybackService::new())
});

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
    match options.mode {
        ProxyMode::Record => {
            // Save request info before consuming
            let method = req.method().clone();
            let uri = req.uri().clone();
            let headers = req.headers().clone();
            
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
            PLAYBACK_SERVICE.add_record(record).await;
            
            // 构建响应
            let response = Response::from_parts(parts, Full::new(body_bytes));
            
            Ok(response)
        }
        ProxyMode::Playback => {
            Ok(PLAYBACK_SERVICE.playback(req).await)
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