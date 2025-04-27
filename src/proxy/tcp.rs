use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use tracing::{error, info};
use uuid::Uuid;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use bytes::Bytes;
use http_body_util::Full;

use crate::{
    options::{Options, ProxyMode}, 
    errors::Error,
    model::TrafficRecord,
    playback::PlaybackService,
};

static PLAYBACK_SERVICE: once_cell::sync::Lazy<Arc<PlaybackService>> = once_cell::sync::Lazy::new(|| {
    Arc::new(PlaybackService::new())
});

pub async fn start_server(options: Arc<Options>) -> Result<(), Error> {
    let addr = format!("{}:{}", options.tcp.host, options.tcp.port);
    let addr = SocketAddr::from_str(&addr).map_err(|e| Error::Config(e.to_string()))?;
    let listener = TcpListener::bind(addr).await?;
    info!("TCP proxy server started at {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let options = options.clone();

        if let Some(tls) = &options.tcp.tls {
            let tls_config = crate::load_tls(&tls.tls_cert, &tls.tls_key);
            let tls_acceptor = TlsAcceptor::from(tls_config);
            
            tokio::task::spawn(async move {
                if let Ok(upstream) = tls_acceptor.accept(stream).await {
                    // 透传请求流量
                    handle_tcp_proxy(upstream, options).await;
                } else {
                    error!("failed to accept TLS connection");
                }
            });
        } else {
            tokio::task::spawn(async move {
                // 透传请求流量
                handle_proxy(stream, options).await;
            });
        }
    }
}

async fn handle_tcp_proxy(mut inbound: TlsStream<TcpStream>, options: Arc<Options>) {
    let downstream_addrs = &options.tcp.downstream;
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

        match options.mode {
            ProxyMode::Record => {
                // 记录上行流量，并包装成 TrafficRecord 透传到另一个可用区
                let client_to_server = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ri.read(&mut buffer).await {
                            Ok(0) => continue, // EOF
                            Ok(n) => {
                                match wo.write_all(&buffer[..n]).await {
                                    Ok(_) => {
                                        // 记录上行流量
                                        let record = TrafficRecord::new_tcp(
                                            buffer[..n].to_vec(),
                                            vec![],
                                        );
                                        PLAYBACK_SERVICE.add_record(record.clone()).await;
                                        info!("Recorded {} bytes of upstream traffic", n);

                                        // 如果配置了peer，则转发到peer
                                        if let Some(peer) = &options.peer {
                                            let scheme = if peer.tls { "https" } else { "http" };
                                            let url = format!("{}://{}:{}/sync", scheme, peer.host, peer.port);
                                            
                                            let client = reqwest::Client::new();
                                            if let Err(e) = client.post(&url)
                                                .json(&record)
                                                .send()
                                                .await
                                            {
                                                error!("Failed to forward traffic record to peer: {}", e);
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        error!("failed to write to downstream {:?}", e);
                                        // 关闭连接
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from client: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                let server_to_client = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ro.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                match wi.write_all(&buffer[..n]).await {
                                    Ok(_) => {
                                        // 记录下行流量
                                        let record = TrafficRecord::new_tcp(
                                            vec![],
                                            buffer[..n].to_vec(),
                                        );
                                        // todo 检查返回的响应处理
                                        PLAYBACK_SERVICE.add_record(record.clone()).await;
                                        info!("Recorded {} bytes of downstream traffic", n);

                                        // 如果配置了peer，则转发到peer
                                        if let Some(peer) = &options.peer {
                                            let scheme = if peer.tls { "https" } else { "http" };
                                            let url = format!("{}://{}:{}/sync", scheme, peer.host, peer.port);
                                            
                                            let client = reqwest::Client::new();
                                            if let Err(e) = client.post(&url)
                                                .json(&record)
                                                .send()
                                                .await
                                            {
                                                error!("Failed to forward traffic record to peer: {}", e);
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        error!("failed to write to downstream {:?}", e);
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from downstream: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                // 同时处理双向数据流
                let _ = tokio::join!(client_to_server, server_to_client);
            }
            ProxyMode::Playback => {
                // 从本地回放服务获取记录并回放
                let mut buffer = [0u8; 8192];
                loop {
                    match ri.read(&mut buffer).await {
                        Ok(0) => continue,
                        Ok(n) => {
                            // 查找匹配的记录
                            let records = PLAYBACK_SERVICE.get_recent_records(None).await;
                            for record in records {
                                if record.protocol == crate::model::Protocol::TCP {
                                    // 回放请求
                                    if !record.request.body.is_empty() {
                                        if let Err(e) = wo.write_all(&record.request.body).await {
                                            error!("Failed to playback request: {}", e);
                                            break;
                                        }
                                    }
                                    // 回放响应
                                    if !record.response.body.is_empty() {
                                        if let Err(e) = wi.write_all(&record.response.body).await {
                                            error!("Failed to playback response: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from client: {:?}", e);
                            break;
                        }
                    }
                }
            }
            ProxyMode::Forward => {
                // 双向转发数据
                let client_to_server = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ri.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                if let Err(e) = wo.write_all(&buffer[..n]).await {
                                    error!("Failed to write to downstream: {:?}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from client: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                let server_to_client = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ro.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                if let Err(e) = wi.write_all(&buffer[..n]).await {
                                    error!("Failed to write to client: {:?}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from downstream: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                // 同时处理双向数据流
                let _ = tokio::join!(client_to_server, server_to_client);
            }
        }
    }
}

async fn handle_proxy(mut inbound: TcpStream, options: Arc<Options>) {
    let downstream_addrs = &options.tcp.downstream;
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

        match options.mode {
            ProxyMode::Record => {
                // 双向转发数据并记录
                let client_to_server = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ri.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                match wo.write_all(&buffer[..n]).await {
                                    Ok(_) => {
                                        // 记录上行流量
                                        let record = TrafficRecord::new_tcp(
                                            buffer[..n].to_vec(),
                                            vec![],
                                        );
                                        PLAYBACK_SERVICE.add_record(record.clone()).await;
                                        info!("Recorded {} bytes of upstream traffic", n);

                                        // 如果配置了peer，则转发到peer
                                        if let Some(peer) = &options.peer {
                                            let scheme = if peer.tls { "https" } else { "http" };
                                            let url = format!("{}://{}:{}/sync", scheme, peer.host, peer.port);
                                            
                                            let client = reqwest::Client::new();
                                            if let Err(e) = client.post(&url)
                                                .json(&record)
                                                .send()
                                                .await
                                            {
                                                error!("Failed to forward traffic record to peer: {}", e);
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        error!("failed to write to downstream {:?}", e);
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from client: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                let server_to_client = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ro.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                match wi.write_all(&buffer[..n]).await {
                                    Ok(_) => {
                                        // 记录下行流量
                                        let record = TrafficRecord::new_tcp(
                                            vec![],
                                            buffer[..n].to_vec(),
                                        );
                                        PLAYBACK_SERVICE.add_record(record.clone()).await;
                                        info!("Recorded {} bytes of downstream traffic", n);

                                        // 如果配置了peer，则转发到peer
                                        if let Some(peer) = &options.peer {
                                            let scheme = if peer.tls { "https" } else { "http" };
                                            let url = format!("{}://{}:{}/sync", scheme, peer.host, peer.port);
                                            
                                            let client = reqwest::Client::new();
                                            if let Err(e) = client.post(&url)
                                                .json(&record)
                                                .send()
                                                .await
                                            {
                                                error!("Failed to forward traffic record to peer: {}", e);
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        error!("failed to write to downstream {:?}", e);
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from downstream: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                // 同时处理双向数据流
                let _ = tokio::join!(client_to_server, server_to_client);
            }
            ProxyMode::Playback => {
                // 从本地回放服务获取记录并回放
                let mut buffer = [0u8; 8192];
                loop {
                    match ri.read(&mut buffer).await {
                        Ok(0) => continue,
                        Ok(n) => {
                            // 查找匹配的记录
                            let records = PLAYBACK_SERVICE.get_recent_records(None).await;
                            for record in records {
                                if record.protocol == crate::model::Protocol::TCP {
                                    // 回放请求
                                    if !record.request.body.is_empty() {
                                        if let Err(e) = wo.write_all(&record.request.body).await {
                                            error!("Failed to playback request: {}", e);
                                            break;
                                        }
                                    }
                                    // 回放响应
                                    if !record.response.body.is_empty() {
                                        if let Err(e) = wi.write_all(&record.response.body).await {
                                            error!("Failed to playback response: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from client: {:?}", e);
                            break;
                        }
                    }
                }
            }
            ProxyMode::Forward => {
                // 双向转发数据
                let client_to_server = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ri.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                if let Err(e) = wo.write_all(&buffer[..n]).await {
                                    error!("Failed to write to downstream: {:?}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from client: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                let server_to_client = async {
                    let mut buffer = [0u8; 8192];
                    loop {
                        match ro.read(&mut buffer).await {
                            Ok(0) => continue,
                            Ok(n) => {
                                if let Err(e) = wi.write_all(&buffer[..n]).await {
                                    error!("Failed to write to client: {:?}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Failed to read from downstream: {:?}", e);
                                break;
                            }
                        }
                    }
                };

                // 同时处理双向数据流
                let _ = tokio::join!(client_to_server, server_to_client);
            }
        }
    }
} 