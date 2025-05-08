use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use tracing::{error, info};

use crate::{
    errors::Error, get_playback_service, model::TrafficRecord, options::{Options, ProxyMode}
};

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
                    match handle_tcp_proxy(upstream, options).await {
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
            tokio::task::spawn(async move {
                // 透传请求流量
                match handle_proxy(stream, options).await {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Failed to handle TCP proxy: {:?}", e);
                    }
                };
            });
        }
    }
}

async fn handle_tcp_proxy(inbound: TlsStream<TcpStream>, options: Arc<Options>) -> Result<(), Error> {
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
                        let playback_service = get_playback_service().await?;
                        playback_service.add_record(record).await?;
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
        ProxyMode::Playback => {
            // TCP 不需要回放
            return Err(Error::Config("Playback mode is not supported for TCP proxy".to_string()));
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
                        let playback_service = get_playback_service().await?;
                        playback_service.add_record(record).await?;
                        // TODO 是否进行响应
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
        ProxyMode::Playback => {
            // TCP 不需要回放
            return Err(Error::Config("Playback mode is not supported for TCP proxy".to_string()));
        }
    }

    Err(Error::Config("unknown proxy mode".to_string()))
} 