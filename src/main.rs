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
use std::time::Duration;

use clap::Parser;
use qproxy::{
    errors::Error, logger, options::Options, send_shutdown_signal, start_qproxy,
};
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use tokio::signal;
use tokio::time::timeout;
use tracing::{error, info, warn};

/// QProxy - 支持流量录制和回放的代理服务器
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 配置文件路径
    #[arg(short, long, default_value = "config.json")]
    config: String,
    
    /// 日志级别
    #[arg(short, long, default_value = "info")]
    log_level: String,
    
    /// 显示详细输出
    #[arg(short, long)]
    verbose: bool,
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
    // 解析命令行参数
    let args = Args::parse();
    
    // 设置环境变量以传递配置文件路径
    std::env::set_var("CONFIG_PATH", &args.config);
    
    // 如果指定了详细输出，设置更详细的日志级别
    if args.verbose {
        std::env::set_var("RUST_LOG", "debug");
    } else {
        std::env::set_var("RUST_LOG", &args.log_level);
    }
    
    info!("Starting QProxy with config: {}", args.config);
    info!("Log level: {}", args.log_level);

    // 加载配置
    let options = match Options::new() {
        Ok(opts) => opts,
        Err(e) => {
            error!("Failed to load configuration from {}: {}", args.config, e);
            return Err(e);
        }
    };
    info!("Loaded configuration: {:?}", options.clone());

    // 初始化日志
    logger::init_logger(&options.logging)?;

    // 启动qproxy
    let handles = start_qproxy(&options).await?;

    // 等待关闭信号
    shutdown_signal().await;

    // 优雅关闭服务
    graceful_shutdown(handles).await;

    Ok(())
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
        _ = ctrl_c => {
            info!("ctrl+c signal received, starting graceful shutdown");
            let _ = send_shutdown_signal().await;
        },
        _ = terminate => {
            info!("terminate signal received, starting graceful shutdown");
            let _ = send_shutdown_signal().await;
        },
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

/// 优雅关闭服务
async fn graceful_shutdown(handles: Vec<tokio::task::JoinHandle<()>>) {
    info!("Starting graceful shutdown...");

    // 设置关闭超时时间
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

    // 发送关闭信号
    send_shutdown_signal().await;

    // 按顺序关闭各个服务
    for (i, handle) in handles.into_iter().enumerate() {
        info!("Shutting down service {}...", i + 1);

        // 获取任务的 abort_handle, 用于在超时后强制关闭任务，这里使用abort_handle而不是handle来终止任务，是为了解决所有权问题
        let abort_handle = handle.abort_handle();
        // 等待服务关闭，带超时控制
        match timeout(SHUTDOWN_TIMEOUT, handle).await {
            Ok(Ok(_)) => info!("Service {} shutdown completed", i + 1),
            Ok(Err(e)) => warn!("Service {} shutdown with error: {}", i + 1, e),
            Err(_) => {
                warn!("Service {} shutdown timed out, forcing shutdown", i + 1);
                abort_handle.abort();
            }
        }
    }

    info!("All services shutdown completed");
}
