// 引入生成的 proto 代码
pub mod route {
    include!(concat!(env!("OUT_DIR"), "/route.rs"));
}

use prost::Message;
use route::route_service_server::{RouteService, RouteServiceServer};
use route::{RouteMessage, RouteResponse};
use std::time::Instant;
use tokio::sync::RwLock;
use tonic::transport::Server;
use tonic::{Code, Request, Response, Status};
use tracing::{error, info};

use crate::errors::Error;
use crate::model::TrafficRecord;
use crate::monitor::{
    GRPC_ACTIVE_CONNECTIONS, GRPC_ERRORS_TOTAL, GRPC_REQUESTS_TOTAL, GRPC_REQUEST_DURATION,
    GRPC_REQUEST_SIZE, GRPC_RESPONSE_SIZE,
};
use crate::options::{Options, ProxyMode};
use crate::PLAYBACK_SERVICE;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

/// 启动 gRPC 服务器
pub async fn start_server(opts: Arc<Options>) -> Result<(), Error> {
    let addr = if let Some(grpc_opts) = &opts.grpc {
        if !grpc_opts.enabled {
            return Err(Error::Config("grpc is not enabled".to_string()));
        }
        let addr = format!("{}:{}", grpc_opts.host, grpc_opts.port);
        addr.parse::<std::net::SocketAddr>()
            .map_err(|e| Error::Config(e.to_string()))?
    } else {
        return Err(Error::Config("grpc options not found".to_string()));
    };

    let route_service = RouteServiceImpl::new(opts.clone());
    Server::builder()
        .add_service(RouteServiceServer::new(route_service))
        .serve(addr)
        .await
        .map_err(|e| Error::Proxy(e.to_string()))?;

    info!("gRPC server started at {}", addr);
    Ok(())
}

#[derive(Debug)]
pub struct RouteServiceImpl {
    options: Arc<Options>,
    // 维护一个共享的http客户端池
    http_client_pool: Arc<RwLock<HashMap<String, reqwest::Client>>>,
    index: AtomicUsize,
}

impl RouteServiceImpl {
    pub fn new(options: Arc<Options>) -> Self {
        Self {
            options,
            http_client_pool: Arc::new(RwLock::new(HashMap::new())),
            index: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl RouteService for RouteServiceImpl {
    async fn send_message(
        &self,
        request: Request<RouteMessage>,
    ) -> Result<Response<RouteResponse>, Status> {
        let start_time = Instant::now();
        let method = "send_message";

        // 记录请求开始
        GrpcMetricsMiddleware::record_request_start(&request);

        let result = self.handle_send_message(request).await;

        // 记录请求完成
        GrpcMetricsMiddleware::record_request_complete(start_time, &result);

        result
    }
}

impl RouteServiceImpl {
    async fn handle_send_message(
        &self,
        request: Request<RouteMessage>,
    ) -> Result<Response<RouteResponse>, Status> {
        let message = request.into_inner();
        info!("received message: {:?}", message);

        // 将RouteMessage整个消息拼接封装grpc请求
        let payload = message.encode_to_vec();
        let record = TrafficRecord::new_grpc("CMP", payload, vec![]);
        // 这里判断当前节点的ProxyMode，如果是Recode模式，则将消息缓存到队列；如果是Playback模式，则将消息透传到公安网服务节点；
        match self.options.mode {
            // 将消息缓存到队列
            ProxyMode::Record => {
                let playback_service = PLAYBACK_SERVICE.read().await;
                if let Some(playback_service) = playback_service.as_ref() {
                    match playback_service.add_record(record).await {
                        Ok(_) => Ok(Response::new(RouteResponse {
                            message_id: "".to_string(),
                            status_code: 200,
                            status_message: "record success".to_string(),
                            payload: vec![],
                        })),
                        Err(e) => {
                            error!("Failed to add record: {}", e);
                            return Err(Status::new(Code::Internal, e.to_string()));
                        }
                    }
                } else {
                    error!("Playback service not initialized");
                    return Err(Status::new(
                        Code::Internal,
                        "Playback service not initialized".to_string(),
                    ));
                }
            }
            // 将消息透传到公安网服务节点
            ProxyMode::Playback => {
                // TODO 优化http客户端，通过客户端池进行复用
                let downstream_addr = if let Some(grpc_opts) = &self.options.grpc {
                    grpc_opts.downstream.clone()
                } else {
                    return Err(Status::new(
                        Code::NotFound,
                        "No grpc downstream address configured".to_string(),
                    ));
                };

                // 选择第一个下游地址，这里创建一个共享池来提供http客户端访问下游地址
                let index = self
                    .index
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    % downstream_addr.len();
                let addr = &downstream_addr[index];
                let client = {
                    let r = self.http_client_pool.read().await;
                    if let Some(client) = r.get(addr) {
                        client.clone()
                    } else {
                        let mut pool = self.http_client_pool.write().await;
                        pool.entry(addr.clone())
                            .or_insert_with(|| reqwest::Client::new())
                            .clone()
                    }
                };

                // 2. 转HTTP POST /sync请求
                let url = format!("http://{}/sync", addr);
                let response = match client.post(&url).json(&record).send().await {
                    Ok(response) => response,
                    Err(e) => {
                        error!("Failed to send request to target service: {}", e);
                        return Err(Status::new(Code::Unavailable, e.to_string()));
                    }
                };

                // 4. 接收下游返回的响应 TrafficRecord，解析成原始流量
                let record_bytes = match response.bytes().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!("Failed to read response: {}", e);
                        return Err(Status::new(Code::Unavailable, e.to_string()));
                    }
                };
                let record: TrafficRecord = match serde_json::from_slice(&record_bytes) {
                    Ok(record) => record,
                    Err(e) => {
                        error!("Failed to parse response: {}", e);
                        return Err(Status::new(Code::Unavailable, e.to_string()));
                    }
                };
                // 5. 将原始响应哦解析成grpc的response进行返回
                let response = RouteResponse {
                    message_id: "".to_string(),
                    status_code: 200,
                    status_message: "OK".to_string(),
                    payload: record.response.body,
                };
                Ok(Response::new(response))
            }
        }
    }
}

/// gRPC 监控中间件
pub struct GrpcMetricsMiddleware;

impl GrpcMetricsMiddleware {
    /// 记录请求开始
    pub fn record_request_start(request: &Request<RouteMessage>) {
        // 记录请求大小
        let request_size = request.get_ref().encoded_len() as f64;
        GRPC_REQUEST_SIZE.observe(request_size);

        // 增加活跃连接数
        GRPC_ACTIVE_CONNECTIONS.inc();
    }

    /// 记录请求完成
    pub fn record_request_complete(
        start_time: Instant,
        result: &Result<Response<RouteResponse>, Status>,
    ) {
        // 记录响应大小
        if let Ok(ref response) = result {
            let response_size = response.get_ref().encoded_len() as f64;
            GRPC_RESPONSE_SIZE.observe(response_size);
        }

        // 记录请求处理时间
        let duration = start_time.elapsed().as_secs_f64();
        GRPC_REQUEST_DURATION.observe(duration);

        // 记录请求结果
        match result {
            Ok(_) => {
                GRPC_REQUESTS_TOTAL.inc();
            }
            Err(_) => {
                GRPC_REQUESTS_TOTAL.inc();
                GRPC_ERRORS_TOTAL.inc();
            }
        }

        // 减少活跃连接数
        GRPC_ACTIVE_CONNECTIONS.dec();
    }
}
