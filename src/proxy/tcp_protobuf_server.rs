use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, error, warn};
use std::sync::Arc;
use prost::Message;

// 引入生成的 proto 代码
pub mod route {
    include!(concat!(env!("OUT_DIR"), "/route.rs"));
}

use route::{
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

pub struct TcpProtoServer {
    addr: SocketAddr,
    tx: mpsc::Sender<RouteMessage>,
}

impl TcpProtoServer {
    pub fn new(addr: SocketAddr, tx: mpsc::Sender<RouteMessage>) -> Self {
        Self { addr, tx }
    }

    async fn handle_connection(
        mut socket: tokio::net::TcpStream,
        tx: Arc<mpsc::Sender<RouteMessage>>,
    ) {
        loop {
            // 读取消息长度（4字节）
            let n = match socket.read_u32().await {
                Ok(n) => n as usize,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    info!("Client disconnected normally");
                    break;
                }
                Err(e) => {
                    error!("Failed to read message length: {}", e);
                    break;
                }
            };

            // 读取消息内容
            let mut message_buf = vec![0u8; n];
            match socket.read_exact(&mut message_buf).await {
                Ok(_) => {
                    // 尝试解析 protobuf 消息，但不关心解析结果
                    let msg = match ProtoRouteMessage::decode(&*message_buf) {
                        Ok(proto_msg) => {
                            let msg: RouteMessage = proto_msg.into();
                            info!("Received protobuf message: {:?}", msg);
                            msg
                        }
                        Err(e) => {
                            warn!("Failed to decode message: {}, using empty message", e);
                            RouteMessage {
                                message_id: "".to_string(),
                                method: "".to_string(),
                                target_id: "".to_string(),
                                app_id: 0,
                                log_id: "".to_string(),
                                from_gateway: false,
                                payload: vec![],
                                metadata: std::collections::HashMap::new(),
                            }
                        }
                    };

                    // 总是返回成功响应
                    let response = RouteResponse {
                        message_id: msg.message_id.clone(),
                        status_code: 200,
                        status_message: "Success".to_string(),
                        payload: vec![],
                    };

                    // 编码并发送响应
                    let proto_response: ProtoRouteResponse = response.into();
                    let response_buf = proto_response.encode_to_vec();
                    
                    // 发送响应长度（4字节）
                    if let Err(e) = socket.write_u32(response_buf.len() as u32).await {
                        error!("Failed to write response length: {}", e);
                        break;
                    }

                    // 发送响应内容
                    if let Err(e) = socket.write_all(&response_buf).await {
                        error!("Failed to write response: {}", e);
                        break;
                    }

                    // 尝试发送到 channel，但不关心结果
                    let _ = tx.send(msg).await;
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    info!("Client disconnected normally");
                    break;
                }
                Err(e) => {
                    error!("Failed to read message: {}", e);
                    break;
                }
            }
        }
        info!("Connection handler completed");
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(self.addr).await?;
        let tx = Arc::new(self.tx);
        info!("TCP protobuf server is starting to listen on {}", self.addr);

        let mut shutdown_rx = crate::get_shutdown_rx().await;
        
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((socket, addr)) => {
                            info!("New connection from {}", addr);
                            let tx = Arc::clone(&tx);
                            tokio::spawn(async move {
                                Self::handle_connection(socket, tx).await;
                            });
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("TCP protobuf server received shutdown signal");
                    break;
                }
            }
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
    
    // 启动服务器
    let _server_handle = tokio::spawn(async move {
        match server.run().await {
            Ok(_) => info!("TCP protobuf server completed successfully"),
            Err(e) => {
                error!("TCP protobuf server failed to start: {}", e);
                std::process::exit(1);
            }
        }
    });

    // 等待服务器启动
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