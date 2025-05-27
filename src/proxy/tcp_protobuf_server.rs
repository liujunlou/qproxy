use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, error, warn};
use std::sync::Arc;
use tonic::{transport::Server, Request, Response, Status, async_trait};

// 引入生成的 proto 代码
pub mod route {
    tonic::include_proto!("route");
}

use route::{
    route_service_server::{RouteService, RouteServiceServer},
    RouteMessage as ProtoRouteMessage,
    RouteResponse as ProtoRouteResponse,
};

// 定义 protobuf 消息和服务
#[derive(Debug, Clone)]
pub struct RouteMessage {
    pub message_id: String,
    pub method: String,
    pub target_id: String,
    pub app_id: i64,
    pub log_id: String,
    pub from_gateway: bool,
    pub payload: Vec<u8>,
    pub metadata: std::collections::HashMap<String, String>,
}

impl From<ProtoRouteMessage> for RouteMessage {
    fn from(msg: ProtoRouteMessage) -> Self {
        Self {
            message_id: msg.message_id,
            method: msg.method,
            target_id: msg.target_id,
            app_id: msg.app_id,
            log_id: msg.log_id,
            from_gateway: msg.from_gateway,
            payload: msg.payload,
            metadata: msg.metadata,
        }
    }
}

impl From<RouteMessage> for ProtoRouteMessage {
    fn from(msg: RouteMessage) -> Self {
        Self {
            message_id: msg.message_id,
            method: msg.method,
            target_id: msg.target_id,
            app_id: msg.app_id,
            log_id: msg.log_id,
            from_gateway: msg.from_gateway,
            payload: msg.payload,
            metadata: msg.metadata,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteResponse {
    pub message_id: String,
    pub status_code: i32,
    pub status_message: String,
    pub payload: Vec<u8>,
}

impl From<ProtoRouteResponse> for RouteResponse {
    fn from(resp: ProtoRouteResponse) -> Self {
        Self {
            message_id: resp.message_id,
            status_code: resp.status_code,
            status_message: resp.status_message,
            payload: resp.payload,
        }
    }
}

impl From<RouteResponse> for ProtoRouteResponse {
    fn from(resp: RouteResponse) -> Self {
        Self {
            message_id: resp.message_id,
            status_code: resp.status_code,
            status_message: resp.status_message,
            payload: resp.payload,
        }
    }
}

pub struct RouteServiceImpl {
    tx: Arc<mpsc::Sender<RouteMessage>>,
}

impl Clone for RouteServiceImpl {
    fn clone(&self) -> Self {
        Self {
            tx: Arc::clone(&self.tx),
        }
    }
}

impl RouteServiceImpl {
    pub fn new(tx: Arc<mpsc::Sender<RouteMessage>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl RouteService for RouteServiceImpl {
    async fn send_message(
        &self,
        request: Request<ProtoRouteMessage>,
    ) -> Result<Response<ProtoRouteResponse>, Status> {
        let msg: RouteMessage = request.into_inner().into();
        let message_id = msg.message_id.clone();
        info!("Received protobuf message: {:?}", msg);
        
        // 转发消息到通道
        if let Err(e) = self.tx.send(msg.clone()).await {
            error!("Failed to forward message: {}", e);
            return Err(Status::internal("Failed to process message"));
        }

        // 创建响应
        let response = RouteResponse {
            message_id: msg.message_id,
            status_code: 200,
            status_message: "Message received".to_string(),
            payload: vec![],
        };

        info!("Sending response for message: {}", message_id);
        Ok(Response::new(response.into()))
    }
}

pub struct TcpProtoServer {
    addr: SocketAddr,
    tx: mpsc::Sender<RouteMessage>,
}

impl TcpProtoServer {
    pub fn new(addr: SocketAddr, tx: mpsc::Sender<RouteMessage>) -> Self {
        Self { addr, tx }
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let service = RouteServiceImpl::new(Arc::new(self.tx));
        info!("TCP protobuf server is starting to listen on {}", self.addr);
        
        let server = Server::builder()
            .tcp_keepalive(Some(std::time::Duration::from_secs(30)))
            .tcp_nodelay(true)
            .add_service(RouteServiceServer::new(service));
        
        let mut shutdown_rx = crate::get_shutdown_rx().await;
        tokio::select! {
            result = server.serve(self.addr) => {
                if let Err(e) = result {
                    error!("TCP protobuf server error: {}", e);
                    return Err(e.into());
                }
                info!("TCP protobuf server completed successfully");
            }
            _ = shutdown_rx.recv() => info!("TCP protobuf server received shutdown signal"),
        }
        info!("TCP protobuf server is shutting down");
        Ok(())
    }
}

// 启动 TCP protobuf 服务器的辅助函数
pub async fn start_tcp_proto_server(addr: SocketAddr) -> Result<mpsc::Receiver<RouteMessage>, Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel(100);
    let server = TcpProtoServer::new(addr, tx);
    
    info!("Attempting to start TCP protobuf server on {}", addr);
    
    // 先尝试绑定端口，确保端口可用
    let listener = tokio::net::TcpListener::bind(addr).await?;
    drop(listener); // 释放端口，让服务器使用
    
    // 启动服务器并等待它开始监听
    let _server_handle = tokio::spawn(async move {
        match server.run().await {
            Ok(_) => info!("TCP protobuf server completed successfully"),
            Err(e) => {
                error!("TCP protobuf server failed to start: {}", e);
                std::process::exit(1);
            }
        }
    });

    // 等待服务器启动，增加重试次数和等待时间
    let max_retries = 10;
    let mut retry_count = 0;
    let mut connected = false;

    while retry_count < max_retries && !connected {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => {
                connected = true;
                info!("Successfully connected to TCP protobuf server at {}", addr);
            }
            Err(e) => {
                warn!("Attempt {}: Failed to connect to TCP protobuf server at {}: {}", 
                    retry_count + 1, addr, e);
                retry_count += 1;
            }
        }
    }

    if !connected {
        error!("Failed to start TCP protobuf server after {} attempts", max_retries);
        return Err(format!("Failed to start TCP protobuf server: Could not connect to {}", addr).into());
    }

    info!("TCP protobuf server is running and listening on {}", addr);
    Ok(rx)
} 