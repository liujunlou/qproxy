use qproxy::{
    options::{Options, HttpOptions, TcpOptions, ProxyMode},
    proxy::http::start_server,
};
use std::{net::TcpListener, sync::Arc};
use tokio::net::TcpStream;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use http_body_util::{BodyExt, Full};
use bytes::Bytes;

async fn create_test_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    
    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept() {
                let mut stream = tokio::net::TcpStream::from_std(stream).unwrap();
                let response = "HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello World!";
                let _ = stream.try_write(response.as_bytes());
            }
        }
    });

    port
}

#[tokio::test]
async fn test_proxy_forward_mode() {
    // 创建测试下游服务器
    let downstream_port = create_test_server().await;
    
    // 创建代理服务器配置
    let options = Arc::new(Options {
        http: HttpOptions {
            host: "127.0.0.1".to_string(),
            port: 0, // 随机端口
            downstream: format!("http://127.0.0.1:{}", downstream_port),
        },
        tcp: TcpOptions {
            host: "127.0.0.1".to_string(),
            port: 0,
            downstream: vec![],
            tls: None,
        },
        mode: ProxyMode::Forward,
        peer: None,
        service_discovery: Default::default(),
    });

    // 启动代理服务器
    let proxy_handle = tokio::spawn(start_server(options.clone()));
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 发送测试请求
    let client = Client::builder(TokioExecutor::new())
        .build_http();
    let req = http::Request::builder()
        .uri(format!("http://127.0.0.1:{}/test", options.http.port))
        .body(Full::new(Bytes::from("")))
        .unwrap();
    let resp = client.request(req).await.unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"Hello World!");

    proxy_handle.abort();
}

#[tokio::test]
async fn test_proxy_record_mode() {
    // 创建测试下游服务器
    let downstream_port = create_test_server().await;
    
    // 创建代理服务器配置
    let options = Arc::new(Options {
        http: HttpOptions {
            host: "127.0.0.1".to_string(),
            port: 0,
            downstream: format!("http://127.0.0.1:{}", downstream_port),
        },
        tcp: TcpOptions {
            host: "127.0.0.1".to_string(),
            port: 0,
            downstream: vec![],
            tls: None,
        },
        mode: ProxyMode::Record,
        peer: None,
        service_discovery: Default::default(),
    });

    // 启动代理服务器
    let proxy_handle = tokio::spawn(start_server(options.clone()));
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 发送测试请求
    let client = Client::builder(TokioExecutor::new())
        .build_http();
    let req = http::Request::builder()
        .uri(format!("http://127.0.0.1:{}/test", options.http.port))
        .body(Full::new(Bytes::from("")))
        .unwrap();
    let resp = client.request(req).await.unwrap();

    assert_eq!(resp.status(), 200);

    proxy_handle.abort();
}

#[tokio::test]
async fn test_proxy_playback_mode() {
    // 创建代理服务器配置
    let options = Arc::new(Options {
        http: HttpOptions {
            host: "127.0.0.1".to_string(),
            port: 0,
            downstream: "http://localhost:0".to_string(),
        },
        tcp: TcpOptions {
            host: "127.0.0.1".to_string(),
            port: 0,
            downstream: vec![],
            tls: None,
        },
        mode: ProxyMode::Playback,
        peer: None,
        service_discovery: Default::default(),
    });

    // 启动代理服务器
    let proxy_handle = tokio::spawn(start_server(options.clone()));
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 发送测试请求
    let client = Client::builder(TokioExecutor::new())
        .build_http();
    let req = http::Request::builder()
        .uri(format!("http://127.0.0.1:{}/test", options.http.port))
        .body(Full::new(Bytes::from("")))
        .unwrap();
    let resp = client.request(req).await.unwrap();

    assert_eq!(resp.status(), 404); // 因为没有匹配的记录，所以返回404

    proxy_handle.abort();
} 