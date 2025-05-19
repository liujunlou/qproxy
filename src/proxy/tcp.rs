use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use tracing::{error, info};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    errors::Error, get_shutdown_rx, load_tls, model::TrafficRecord, options::{Options, ProxyMode}, PLAYBACK_SERVICE
};

pub async fn start_server(options: Arc<Options>) -> Result<(), Error> {
    let addr = format!("{}:{}", options.tcp.host, options.tcp.port);
    let addr = SocketAddr::from_str(&addr).expect("invalid socket address");
    let listener = TcpListener::bind(addr).await?;
    info!("TCP proxy server listening on {}", addr);
    
    let mut shutdown_rx = get_shutdown_rx().await;
    
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        info!("New TCP connection from {}", addr);
                        let options = options.clone();

                        if let Some(tls) = &options.tcp.tls {
                            // 创建TLS接受器
                            let tls_config = load_tls(&tls.tls_cert, &tls.tls_key);
                            let acceptor = TlsAcceptor::from(tls_config);

                            tokio::spawn(async move {
                                if let Ok(upstream) = acceptor.accept(stream).await {
                                    // 透传请求流量
                                    match handle_tls_proxy(tokio_rustls::TlsStream::Server(upstream), options).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            error!("Failed to handle TCP proxy: {:?}", e);
                                        }
                                    };
                                } else {
                                    error!("failed to accept TLS connection");
                                }
                            });
                        } else {
                            tokio::spawn(async move {
                                if let Err(e) = handle_proxy(stream, options).await {
                                    error!("Error handling TCP connection: {}", e);
                                }
                            });
                        }
                    }
                    Err(e) => {
                        error!("Error accepting TCP connection: {}", e);
                        continue;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("TCP server received shutdown signal");
                break;
            }
        }
    }
    
    info!("TCP proxy server shutdown complete");
    Ok(())
}

pub async fn handle_tls_proxy<S>(inbound: tokio_rustls::TlsStream<S>, options: Arc<Options>) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ri, mut wi) = tokio::io::split(inbound);
    let mut buffer = [0u8; 8192];
    // 判断是录制的流量还是回放的流量
    match options.mode {
        ProxyMode::Record => {
            // 记录上行流量，并包装成 TrafficRecord 进行缓存
            loop {
                match ri.read(&mut buffer).await {
                    Ok(0) => continue,
                    Ok(n) => {
                        let record = TrafficRecord::new_tcp("CMP", buffer[..n].to_vec(), vec![]);
                        // 写入缓存
                        let playback_service = PLAYBACK_SERVICE.lock().await;
                        if let Some(playback_service) = playback_service.as_ref() {
                            playback_service.add_record(record).await?;
                        } else {
                            error!("Playback service not initialized");
                        }
                    }
                    Err(e) => {
                        error!("Failed to read from client: {:?}", e);
                        break;
                    }
                }
            }
        }
        ProxyMode::Playback => {
            // 双向转发数据
            loop {
                match ri.read(&mut buffer).await {
                    Ok(0) => continue,
                    Ok(n) => {
                        // 1. 将流量包装成 TrafficRecord 对象
                        let record = TrafficRecord::new_tcp("CMP", buffer[..n].to_vec(), vec![]);
                        // TODO 优化http客户端，通过客户端池进行复用
                        let downstream_addr = &options.tcp.downstream;
                        if downstream_addr.is_empty() {
                            error!("No downstream address configured");
                            return Err(Error::Config("No downstream address configured".to_string()));
                        }
                        
                        // 选择第一个下游地址，TODO 这里需要根据流量来源选择下游地址
                        let addr = &downstream_addr[0];

                        // 2. 转HTTP POST /sync请求
                        let client = reqwest::Client::new();
                        let url = format!("http://{}/sync", addr);
                        let response = client.post(&url)
                            .json(&record)
                            .send()
                            .await?;

                        // 4. 接收下游返回的响应 TrafficRecord，解析成原始流量
                        let record_bytes = response.bytes().await?;
                        let record: TrafficRecord = serde_json::from_slice(&record_bytes)?;
                        // 5. 将原始流量进行响应
                        match wi.write_all(&record.response.body).await {
                            Ok(_) => {
                                info!("Forwarded {} bytes of upstream traffic", n);
                            }
                            Err(e) => {
                                error!("Failed to write to upstream: {:?}", e);
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
        }
    }
    
    Err(Error::Config("unknown proxy mode".to_string()))
}

async fn handle_proxy(inbound: TcpStream, options: Arc<Options>) -> Result<(), Error> {
    let (mut ri, mut wi) = tokio::io::split(inbound);
    let mut buffer = [0u8; 8192];

    // 判断是录制的流量还是回放的流量
    match options.mode {
        ProxyMode::Record => {
            // 记录上行流量，并包装成 TrafficRecord 进行缓存
            loop {
                match ri.read(&mut buffer).await {
                    Ok(0) => continue,
                    Ok(n) => {
                        let record = TrafficRecord::new_tcp("CMP", buffer[..n].to_vec(), vec![]);
                        // 写入缓存
                        let playback_service = PLAYBACK_SERVICE.lock().await;
                        if let Some(playback_service) = playback_service.as_ref() {
                            playback_service.add_record(record).await?;
                        } else {
                            error!("Playback service not initialized");
                        }
                        // TODO 是否进行响应
                    }
                    Err(e) => {
                        error!("Failed to read from client: {:?}", e);
                        break;
                    }
                }
            }
        }
        ProxyMode::Playback => {
            // 双向转发数据
            loop {
                match ri.read(&mut buffer).await {
                    Ok(0) => continue,
                    Ok(n) => {
                        // 1. 将流量包装成 TrafficRecord 对象
                        let record = TrafficRecord::new_tcp("CMP", buffer[..n].to_vec(), vec![]);
                        // TODO 优化http客户端，通过客户端池进行复用
                        let downstream_addr = &options.tcp.downstream;
                        if downstream_addr.is_empty() {
                            error!("No downstream address configured");
                            return Err(Error::Config("No downstream address configured".to_string()));
                        }
                        
                        // 选择第一个下游地址，TODO 这里需要根据流量来源选择下游地址
                        let addr = &downstream_addr[0];

                        // 2. 转HTTP POST /sync请求
                        let client = reqwest::Client::new();
                        let url = format!("http://{}/sync", addr);
                        let response = client.post(&url)
                            .json(&record)
                            .send()
                            .await?;

                        // 4. 接收下游返回的响应 TrafficRecord，解析成原始流量
                        let record_bytes = response.bytes().await?;
                        let record: TrafficRecord = serde_json::from_slice(&record_bytes)?;
                        // 5. 将原始流量进行响应
                        match wi.write_all(&record.response.body).await {
                            Ok(_) => {
                                info!("Forwarded {} bytes of upstream traffic", n);
                            }
                            Err(e) => {
                                error!("Failed to write to upstream: {:?}", e);
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
        }
    }

    Err(Error::Config("unknown proxy mode".to_string()))
} 