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
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info};

use crate::errors::Error;
use crate::model::TrafficRecord;
use crate::monitor::{
    GRPC_ACTIVE_CONNECTIONS, GRPC_ERRORS_TOTAL, GRPC_REQUESTS_TOTAL, GRPC_REQUEST_DURATION,
    GRPC_REQUEST_SIZE, GRPC_RESPONSE_SIZE,
};
use crate::options::{Options, ProxyMode};
use crate::{get_shutdown_rx, PLAYBACK_SERVICE};
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

    info!("gRPC server starting on {}", addr);

    // 通过反射开启通用 invoke 能力（支持 grpcurl 等动态调用）
    // let reflection_service = {
    //     // 由 build.rs 生成的描述符文件名
    //     static DESCRIPTOR_BYTES: &[u8] =
    //         include_bytes!(concat!(env!("OUT_DIR"), "/route_descriptor.bin"));
    //     ReflectionBuilder::configure()
    //         .register_encoded_file_descriptor_set(DESCRIPTOR_BYTES)
    //         .build()
    //         .expect("build reflection service")
    // };

    // 创建 gRPC 服务器，添加超时和连接限制，并注册反射服务
    let server = Server::builder()
        .timeout(std::time::Duration::from_secs(30))
        .concurrency_limit_per_connection(1024)
        // .add_service(reflection_service)
        .add_service(RouteServiceServer::new(route_service));

    // 获取关闭信号接收器
    let mut shutdown_rx = get_shutdown_rx().await;

    // 启动服务器
    let server_handle = tokio::spawn(async move { server.serve(addr).await });

    info!("gRPC server listening on {}", addr);

    // 等待服务器停止或收到关闭信号
    tokio::select! {
        server_result = server_handle => {
            match server_result {
                Ok(Ok(())) => {
                    info!("gRPC server stopped normally");
                    Ok(())
                }
                Ok(Err(e)) => {
                    error!("gRPC server error: {}", e);
                    Err(Error::Proxy(format!("gRPC server error: {}", e)))
                }
                Err(e) => {
                    error!("gRPC server task error: {}", e);
                    Err(Error::Proxy(format!("gRPC server task error: {}", e)))
                }
            }
        }
        _ = shutdown_rx.recv() => {
            info!("gRPC server received shutdown signal, starting graceful shutdown");

            // 由于server_handle已经在select!中被消费，这里直接返回成功
            // tonic服务器会自动处理优雅关闭
            info!("gRPC server shutdown complete");
            Ok(())
        }
    }
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
        info!("Received gRPC message: {:?}", message);

        // 将RouteMessage整个消息拼接封装grpc请求
        let payload = message.encode_to_vec();
        let record = TrafficRecord::new_grpc("default", payload, vec![]);

        // 根据当前节点的ProxyMode处理消息
        match self.options.mode {
            ProxyMode::Record => self.handle_record_mode(record).await,
            ProxyMode::Playback => self.handle_playback_mode(record).await,
        }
    }

    /// 处理录制模式：将消息缓存到队列
    async fn handle_record_mode(
        &self,
        record: TrafficRecord,
    ) -> Result<Response<RouteResponse>, Status> {
        let playback_service = PLAYBACK_SERVICE.read().await;
        let playback_service = playback_service.as_ref().ok_or_else(|| {
            error!("Playback service not initialized");
            Status::new(Code::Internal, "Playback service not initialized")
        })?;

        playback_service.add_record(record).await.map_err(|e| {
            error!("Failed to add record: {}", e);
            Status::new(Code::Internal, e.to_string())
        })?;

        Ok(Response::new(RouteResponse {
            message_id: None,
            status_code: Some(200),
            status_message: Some("record success".to_string()),
            payload: None,
        }))
    }

    /// 处理回放模式：将消息透传到下游服务节点
    async fn handle_playback_mode(
        &self,
        record: TrafficRecord,
    ) -> Result<Response<RouteResponse>, Status> {
        let downstream_addr = self.get_downstream_addresses()?;

        if downstream_addr.is_empty() {
            return Err(Status::new(
                Code::NotFound,
                "No grpc downstream address configured".to_string(),
            ));
        }

        // 选择下游地址（轮询负载均衡）
        let index = self
            .index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % downstream_addr.len();
        let addr = &downstream_addr[index];

        // 获取或创建HTTP客户端
        let client = self.get_or_create_client(addr).await;

        // 发送HTTP POST请求到下游服务
        let url = format!("http://{}/sync", addr);
        let response = client.post(&url).json(&record).send().await.map_err(|e| {
            error!("Failed to send request to target service {}: {}", addr, e);
            Status::new(Code::Unavailable, e.to_string())
        })?;

        // 检查HTTP响应状态
        info!("Response {:?}", response);
        if !response.status().is_success() {
            let status_text = response.status().to_string();
            error!(
                "Downstream service {} returned error status: {}",
                addr, status_text
            );
            return Err(Status::new(
                Code::Unavailable,
                format!("Downstream error: {}", status_text),
            ));
        }

        // 解析响应
        let record_bytes = response.bytes().await.map_err(|e| {
            error!("Failed to read response from {}: {}", addr, e);
            Status::new(Code::Unavailable, e.to_string())
        })?;

        let record: TrafficRecord = serde_json::from_slice(&record_bytes).map_err(|e| {
            error!("Failed to parse response from {}: {}", addr, e);
            Status::new(Code::Unavailable, e.to_string())
        })?;

        // 返回gRPC响应
        Ok(Response::new(RouteResponse {
            message_id: None,
            status_code: Some(200),
            status_message: Some("OK".to_string()),
            payload: Some(record.response.body),
        }))
    }

    /// 获取下游地址列表
    fn get_downstream_addresses(&self) -> Result<Vec<String>, Status> {
        self.options
            .grpc
            .as_ref()
            .map(|grpc_opts| grpc_opts.downstream.clone())
            .ok_or_else(|| {
                Status::new(
                    Code::NotFound,
                    "No grpc downstream address configured".to_string(),
                )
            })
    }

    /// 获取或创建HTTP客户端
    async fn get_or_create_client(&self, addr: &str) -> reqwest::Client {
        let pool = self.http_client_pool.read().await;
        if let Some(client) = pool.get(addr) {
            client.clone()
        } else {
            drop(pool); // 释放读锁
            let mut pool = self.http_client_pool.write().await;
            pool.entry(addr.to_string())
                .or_insert_with(|| {
                    reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(30))
                        .build()
                        .unwrap_or_else(|_| reqwest::Client::new())
                })
                .clone()
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
