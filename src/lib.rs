//! QProxy库 - 支持流量录制和回放的代理服务器
//!
//! 本库实现了一个功能完整的代理服务器，支持以下特性：
//! - HTTP/HTTPS 代理
//! - TCP 代理（可选 TLS）
//! - 流量录制
//! - 流量回放
//! - 跨可用区部署
//! - MQTT客户端支持

pub mod api;
pub mod errors;
pub mod filter;
pub mod logger;
pub mod model;
pub mod options;
pub mod playback;
pub mod proxy;
pub mod record;
pub mod service_discovery;
pub mod sync;
pub mod client;
pub mod mqtt_client {
    pub mod client;
    pub mod codec;
}
pub mod monitor;

use errors::Error;
use filter::FilterChain;
use monitor::collector::MetricsCollectorTask;
use once_cell::sync::Lazy;
use options::Options;
use playback::PlaybackService;
use proxy::ProxyServer;
use rustls::ServerConfig;
use service_discovery::ServiceRegistry;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info};

/// 全局共享的回放服务实例
/// 使用 once_cell 确保单例模式和线程安全
pub static PLAYBACK_SERVICE: Lazy<Arc<RwLock<Option<Arc<PlaybackService>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));

/// 全局共享的服务注册表实例
/// 使用 once_cell 确保单例模式和线程安全
pub static SERVICE_REGISTRY: Lazy<Arc<RwLock<ServiceRegistry>>> =
    Lazy::new(|| Arc::new(RwLock::new(ServiceRegistry::new())));

/// 全局共享的指标监控实例
/// 使用 once_cell 确保单例模式和线程安全
pub static METRICS_COLLECTOR: Lazy<Arc<RwLock<Option<Arc<MetricsCollectorTask>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));

// 提供静态的 FilterChain 实例，支持多线程下使用
pub static ONCE_FILTER_CHAIN: Lazy<Arc<RwLock<FilterChain>>> =
    Lazy::new(|| Arc::new(RwLock::new(FilterChain::new())));

// 全局关闭信号发送器
static SHUTDOWN_TX: once_cell::sync::Lazy<Arc<Mutex<broadcast::Sender<()>>>> =
    once_cell::sync::Lazy::new(|| {
        let (tx, _) = broadcast::channel(1);
        Arc::new(Mutex::new(tx))
    });

/// 获取关闭信号接收器
pub async fn get_shutdown_rx() -> broadcast::Receiver<()> {
    let tx = SHUTDOWN_TX.lock().await;
    tx.subscribe()
}

/// 发送关闭信号
pub async fn send_shutdown_signal() {
    let tx = SHUTDOWN_TX.lock().await;
    let _ = tx.send(());
}

// 初始化qproxy
pub async fn start_qproxy(options: &Options) -> Result<Vec<JoinHandle<()>>, Error> {
    // 初始化监控
    start_metrics_collector(&options).await?;

    // 初始化服务注册表
    SERVICE_REGISTRY.write().await.init(&options).await?;

    // 创建同步服务
    let mut sync_handle = None;
    if options.sync.clone().enabled {
        let sync_service = sync::SyncService::new(&options)?;
        sync_handle = Some(sync_service.start().await);
    }

    // 初始化回放服务
    init_playback_service(&options).await?;

    // 启动代理服务器
    let server = ProxyServer::new((*options).clone());
    let mut handlers = server.start().await;

    if let Some(sync_handle) = sync_handle {
        handlers.push(sync_handle);
    }

    Ok(handlers)
}

/// 启动指标监控
///
/// 启动指标监控任务，并将其添加到任务列表中
///
/// # 参数
/// * `opts` - 配置选项
///
/// # 返回值
///
/// 返回指标监控任务
async fn start_metrics_collector(opts: &Options) -> Result<(), Error> {
    let mut collector = METRICS_COLLECTOR.write().await;
    if collector.is_none() {
        let metrics_collector_task = MetricsCollectorTask::new(opts);
        *collector = Some(Arc::new(metrics_collector_task));
    }
    Ok(())
}

/// 初始化全局回放服务
///
/// # 参数
/// * `options` - 全局配置选项
///
/// # 错误
/// 如果Redis连接失败，返回错误
async fn init_playback_service(options: &Options) -> Result<(), Error> {
    let mut service = PLAYBACK_SERVICE.write().await;
    if service.is_none() {
        let redis_options = &options.redis;
        let redis_url = redis_options.url.clone();

        // 创建Redis客户端
        let client = redis::Client::open(redis_url.as_str())?;

        // 创建连接管理器
        match redis::aio::ConnectionManager::new(client).await {
            Ok(conn_manager) => match PlaybackService::new(conn_manager).await {
                Ok(playback_service) => {
                    *service = Some(Arc::new(playback_service));
                    info!(
                        "Successfully initialized playback service with Redis at {}",
                        redis_url
                    );
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to create playback service: {}", e);
                    Err(e)
                }
            },
            Err(e) => {
                error!("Failed to connect to Redis at {}: {}", redis_url, e);
                Err(Error::Redis(e))
            }
        }
    } else {
        Ok(())
    }
}

/// 加载TLS证书和私钥
pub fn load_tls(cert_path: &str, key_path: &str) -> Arc<ServerConfig> {
    let cert_file = File::open(cert_path).expect("Failed to open certificate file");
    let key_file = File::open(key_path).expect("Failed to open private key file");

    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);

    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .expect("Failed to parse certificate");

    let key = rustls_pemfile::private_key(&mut key_reader)
        .expect("Failed to parse private key")
        .expect("No private key found");

    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("Failed to configure TLS server");

    Arc::new(server_config)
}
