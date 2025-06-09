// 引入生成的 proto 代码
pub mod route {
    include!(concat!(env!("OUT_DIR"), "/route.rs"));
}

use prost::Message;
use route::route_service_server::{RouteService, RouteServiceServer};
use route::{RouteMessage, RouteResponse};
use tokio::sync::RwLock;
use tonic::transport::Server;
use tonic::{Code, Request, Response, Status};
use tracing::{error, info};

use crate::errors::Error;
use crate::model::TrafficRecord;
use crate::options::{Options, ProxyMode};
use crate::PLAYBACK_SERVICE;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
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
                    return Err(Status::new(Code::Internal, "Playback service not initialized".to_string()));
                }
            }
            // 将消息透传到公安网服务节点
            ProxyMode::Playback => {
                // TODO 优化http客户端，通过客户端池进行复用
                let downstream_addr = &self.options.tcp.downstream;
                if downstream_addr.is_empty() {
                    error!("No downstream address configured");
                    return Err(Status::new(
                        Code::NotFound,
                        "No downstream address configured".to_string(),
                    ));
                }

                // 选择第一个下游地址，这里创建一个共享池来提供http客户端访问下游地址
                let index = self.index.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % downstream_addr.len();
                let addr = &downstream_addr[index];
                let client = {
                    let r = self.http_client_pool.read().await;
                    if let Some(client) = r.get(addr) {
                        client.clone()
                    } else {
                        let mut pool = self.http_client_pool.write().await;
                        pool.entry(addr.clone()).or_insert_with(|| reqwest::Client::new()).clone()
                    }
                };

                // 2. 转HTTP POST /sync请求
                let url = format!("http://{}/sync", addr);
                let response = match client.post(&url).json(&record).send().await {
                    Ok(response) => response,
                    Err(e) => {
                        error!("Failed to send request: {}", e);
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
