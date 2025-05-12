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

use crate::options::Options;
use crate::playback::PlaybackService;
use errors::Error;
use rustls::{pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer}, ServerConfig};
use tokio::signal;
use tracing::{error, info};

mod options;
mod logger;
mod errors;
mod model;
mod playback;
mod proxy;
mod api;
mod sync;
mod service_discovery;
mod mqtt_client;

/// 全局共享的回放服务实例
/// 使用 once_cell 确保单例模式和线程安全
static PLAYBACK_SERVICE: once_cell::sync::Lazy<Arc<tokio::sync::Mutex<Option<Arc<PlaybackService>>>>> = 
    once_cell::sync::Lazy::new(|| {
        Arc::new(tokio::sync::Mutex::new(None))
    });

/// 全局共享的服务注册表实例
/// 使用 once_cell 确保单例模式和线程安全
static SERVICE_REGISTRY: once_cell::sync::Lazy<Arc<tokio::sync::RwLock<service_discovery::ServiceRegistry>>> = 
    once_cell::sync::Lazy::new(|| {
        Arc::new(tokio::sync::RwLock::new(service_discovery::ServiceRegistry::new()))
    });

/// 初始化全局回放服务
/// 
/// # 参数
/// * `options` - 全局配置选项
/// 
/// # 错误
/// 如果Redis连接失败，返回错误
pub async fn init_playback_service(options: &Options) -> Result<(), Error> {
    let mut service = PLAYBACK_SERVICE.lock().await;
    if service.is_none() {
        let redis_options = &options.redis;
        let redis_url = redis_options.url.clone();
        
        // 创建Redis客户端
        let client = redis::Client::open(redis_url.as_str())?;
        
        // 创建连接管理器
        match redis::aio::ConnectionManager::new(client).await {
            Ok(conn_manager) => {
                match PlaybackService::new(conn_manager).await {
                    Ok(playback_service) => {
                        *service = Some(Arc::new(playback_service));
                        info!("Successfully initialized playback service with Redis at {}", redis_url);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to create playback service: {}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect to Redis at {}: {}", redis_url, e);
                Err(Error::Redis(e))
            }
        }
    } else {
        Ok(())
    }
}

/// 获取全局回放服务实例
pub async fn get_playback_service() -> Result<Arc<PlaybackService>, Error> {
    let service = PLAYBACK_SERVICE.lock().await;
    service.clone().ok_or_else(|| Error::Config("Playback service not initialized".to_string()))
}

/// 等待关闭信号
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received, starting graceful shutdown");
}

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
    logger::init_logger(&options.logging)?;
    
    // 初始化服务注册表
    SERVICE_REGISTRY.write().await.init(&options).await?;
    
    // 创建同步服务
    let mut sync_handle = None;
    if options.sync.clone().enabled {
        let sync_service = sync::SyncService::new((*options).clone())?;
        sync_handle = Some(sync_service.start().await);
    }

    // 初始化回放服务
    init_playback_service(&options).await?;
    
    // 启动代理服务器
    let server = proxy::ProxyServer::new((*options).clone());
    let (http_handle, tcp_handle) = server.start().await;

    // 等待关闭信号
    shutdown_signal().await;

    // 优雅关闭服务
    info!("shutting down services...");
    
    // 关闭同步服务
    if options.sync.clone().enabled {
        if let Some(handle) = sync_handle {
            sync::SyncService::abort(handle).await;
        }
    }

    // 关闭代理服务器
    server.abort(http_handle, tcp_handle).await;

    // 等待所有连接处理完成
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    info!("shutdown complete");
    
    Ok(())
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
