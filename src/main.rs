//! QProxy - 一个支持流量录制和回放的代理服务器
//! 
//! 本项目实现了一个功能完整的代理服务器，支持以下特性：
//! - HTTP/HTTPS 代理
//! - TCP 代理（可选 TLS）
//! - 流量录制
//! - 流量回放
//! - 跨可用区部署
//! 
//! 主要模块说明：
//! - proxy: 实现代理功能
//! - playback: 实现流量回放
//! - sync: 实现节点间同步
//! - api: 提供 HTTP API
//! - service_discovery: 服务发现

use std::sync::Arc;

use bytes::Bytes;
use errors::Error;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::{client::legacy::Builder, rt::TokioExecutor};
use rustls::{pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer}, ServerConfig};
use tokio::{net::{TcpListener, TcpStream}, sync::Mutex};
use tokio_rustls::TlsAcceptor;
use tracing::{error, info};
use crate::model::TrafficRecord;
use crate::playback::PlaybackService;
use crate::options::{Options, ProxyMode};
use crate::proxy::{http, tcp};

mod options;
mod logger;
mod errors;
mod model;
mod playback;
mod proxy;
mod api;
mod sync;
mod service_discovery;

/// 全局共享的回放服务实例
/// 使用 once_cell 确保单例模式和线程安全
static PLAYBACK_SERVICE: once_cell::sync::Lazy<Arc<PlaybackService>> = once_cell::sync::Lazy::new(|| {
    Arc::new(PlaybackService::new())
});

/// 程序入口函数
/// 
/// # 错误处理
/// 
/// 返回 `Result<(), Error>` 表示可能的错误：
/// - 配置加载失败
/// - 服务器启动失败
#[tokio::main]
async fn main() -> Result<(), Error> {
    // 加载配置
    let options = match Options::new() {
        Ok(opts) => opts,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e);
        }
    };
    let options = Arc::new(options);
    info!("Loaded configuration: {:?}", options.clone());

    // 初始化日志
    logger::init_logger(&(*options).clone())?;

    // 创建同步服务
    let sync_service = sync::SyncService::new((*options).clone(), PLAYBACK_SERVICE.clone());
    sync_service.start().await;

    // 启动代理服务器
    let server = proxy::ProxyServer::new(((*options).clone()).clone());
    server.start().await;
    
    Ok(())
}

/// 流量录制处理函数
/// 
/// 将接收到的请求转发到下游服务器，同时记录请求和响应信息
/// 
/// # 参数
/// 
/// * `req` - 接收到的 HTTP 请求
/// 
/// # 返回值
/// 
/// 返回下游服务器的响应，或者错误
async fn record(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Error> {
    use http_body_util::BodyExt;
    
    // 保存请求信息
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();
    
    // 获取请求体
    let body_bytes = req.collect().await?.to_bytes();
    
    // 构建转发请求
    let forwarded_req = Request::builder()
        .method(method.clone())
        .uri(uri.clone())
        .body(Full::new(body_bytes.clone()))
        .unwrap();

    // 发送请求到下游服务器并获取响应
    let client = Builder::new(TokioExecutor::new())
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

/// 流量回放处理函数
/// 
/// 根据请求信息从回放服务中查找匹配的记录并返回
/// 
/// # 参数
/// 
/// * `req` - 接收到的 HTTP 请求
/// 
/// # 返回值
/// 
/// 返回匹配的历史响应，或者错误
async fn playback(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Error> {
    Ok(PLAYBACK_SERVICE.playback(req).await)
}

/// TCP 代理处理函数（TLS 版本）
/// 
/// 处理已经完成 TLS 握手的连接，将数据转发到下游服务器
/// 
/// # 参数
/// 
/// * `inbound` - 来自客户端的 TLS 连接
/// * `downstream` - 下游服务器地址列表
async fn handle_tcp_proxy(mut inbound: tokio_rustls::server::TlsStream<TcpStream>, downstream: Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    
    let downstream_addrs = downstream.lock().await;
    if downstream_addrs.is_empty() {
        error!("No downstream address configured");
        return;
    }
    
    // 选择第一个下游地址
    let addr = &downstream_addrs[0];
    
    // 连接下游服务器
    if let Ok(mut outbound) = TcpStream::connect(addr).await {
        let (mut ri, mut wi) = tokio::io::split(inbound);
        let (mut ro, mut wo) = tokio::io::split(outbound);

        // 双向转发数据
        let client_to_server = async {
            let mut buffer = [0u8; 8192];
            loop {
                match ri.read(&mut buffer).await {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if wo.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        let server_to_client = async {
            let mut buffer = [0u8; 8192];
            loop {
                match ro.read(&mut buffer).await {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if wi.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        // 同时处理双向数据流
        let _ = tokio::join!(client_to_server, server_to_client);
    }
}

/// TCP 代理处理函数（非 TLS 版本）
/// 
/// 处理普通 TCP 连接，将数据转发到下游服务器
/// 
/// # 参数
/// 
/// * `inbound` - 来自客户端的 TCP 连接
/// * `downstream` - 下游服务器地址列表
async fn handle_proxy(mut inbound: TcpStream, downstream: Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    
    let downstream_addrs = downstream.lock().await;
    if downstream_addrs.is_empty() {
        error!("No downstream address configured");
        return;
    }
    
    // 选择第一个下游地址
    let addr = &downstream_addrs[0];
    
    // 连接下游服务器
    if let Ok(mut outbound) = TcpStream::connect(addr).await {
        let (mut ri, mut wi) = inbound.split();
        let (mut ro, mut wo) = outbound.split();

        // 双向转发数据
        let client_to_server = async {
            let mut buffer = [0u8; 8192];
            loop {
                match ri.read(&mut buffer).await {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if wo.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        let server_to_client = async {
            let mut buffer = [0u8; 8192];
            loop {
                match ro.read(&mut buffer).await {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if wi.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        // 同时处理双向数据流
        let _ = tokio::join!(client_to_server, server_to_client);
    }
}

/// 加载 TLS 配置
/// 
/// 从指定路径加载证书和私钥，创建 TLS 服务器配置
/// 
/// # 参数
/// 
/// * `cert_path` - 证书文件路径
/// * `key_path` - 私钥文件路径
/// 
/// # 返回值
/// 
/// 返回 TLS 服务器配置
fn load_tls(cert_path: &str, key_path: &str) -> Arc<ServerConfig> {
    let certs = CertificateDer::pem_file_iter(cert_path)
        .expect("cannot read cerificate file")
        .map(|result| result.unwrap())
        .collect();
    let key = PrivateKeyDer::from_pem_file(key_path).expect("connot read private key");

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();

    Arc::new(config)
}
