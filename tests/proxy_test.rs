use qproxy::{
    proxy::{ProxyServer, ProxyMode},
    options::Options,
};
use tokio::test;
use hyper::{Client, Request, Body};
use hyper_util::rt::TokioExecutor;
use std::{sync::Arc, time::Duration};
use http::StatusCode;

// 创建一个简单的HTTP服务器用于测试代理
async fn create_test_server() -> u16 {
    let addr = ([127, 0, 0, 1], 0).into();
    let service = hyper::service::make_service_fn(|_| async {
        Ok::<_, hyper::Error>(hyper::service::service_fn(|_req| async {
            let body = "Hello, World!";
            Ok::<_, hyper::Error>(hyper::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .body(Body::from(body))
                .unwrap())
        }))
    });

    let server = hyper::Server::bind(&addr)
        .serve(service);
    
    let port = server.local_addr().port();
    
    tokio::spawn(async move {
        if let Err(e) = server.await {
            eprintln!("server error: {}", e);
        }
    });
    
    // 短暂等待以确保服务器启动完成
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    port
}

// 创建测试配置
fn create_test_config(target_port: u16) -> Options {
    let mut options = Options::default();
    options.http.enabled = true;
    options.http.port = 8080;
    options.http.proxy_mode = ProxyMode::Forward;
    options.http.target_host = Some(format!("127.0.0.1:{}", target_port));
    options
}

#[test]
async fn test_http_proxy_forward() {
    // 创建一个测试服务器
    let target_port = create_test_server().await;
    
    // 创建测试配置
    let options = create_test_config(target_port);
    let options = Arc::new(options);
    
    // 创建代理服务器
    let proxy_server = ProxyServer::new((*options).clone());
    let (http_handle, _) = proxy_server.start().await;
    
    // 短暂等待以确保代理服务器启动完成
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // 发送请求到代理服务器
    let client = Client::builder()
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .build::<_, hyper::Body>(TokioExecutor::new());
    
    let req = Request::builder()
        .uri(format!("http://127.0.0.1:8080/"))
        .body(Body::empty())
        .unwrap();
    
    let resp = client.request(req).await.expect("Request failed");
    
    // 验证响应
    assert_eq!(resp.status(), StatusCode::OK);
    
    // 读取响应体
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let body_str = String::from_utf8_lossy(&body_bytes);
    
    assert_eq!(body_str, "Hello, World!");
    
    // 停止服务器
    proxy_server.abort(http_handle, None).await;
}

#[test]
async fn test_proxy_headers() {
    // 创建一个可以回显请求头的测试服务器
    let addr = ([127, 0, 0, 1], 0).into();
    let service = hyper::service::make_service_fn(|_| async {
        Ok::<_, hyper::Error>(hyper::service::service_fn(|req| async move {
            // 获取所有请求头
            let headers = req.headers();
            let mut headers_str = String::new();
            for (k, v) in headers.iter() {
                headers_str.push_str(&format!("{}:{}\n", k, v.to_str().unwrap_or("")));
            }
            
            Ok::<_, hyper::Error>(hyper::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .body(Body::from(headers_str))
                .unwrap())
        }))
    });

    let server = hyper::Server::bind(&addr)
        .serve(service);
    
    let target_port = server.local_addr().port();
    
    tokio::spawn(async move {
        if let Err(e) = server.await {
            eprintln!("server error: {}", e);
        }
    });
    
    // 短暂等待以确保服务器启动完成
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // 创建测试配置
    let mut options = Options::default();
    options.http.enabled = true;
    options.http.port = 8081;
    options.http.proxy_mode = ProxyMode::Forward;
    options.http.target_host = Some(format!("127.0.0.1:{}", target_port));
    let options = Arc::new(options);
    
    // 创建代理服务器
    let proxy_server = ProxyServer::new((*options).clone());
    let (http_handle, _) = proxy_server.start().await;
    
    // 短暂等待以确保代理服务器启动完成
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // 发送请求到代理服务器，带有自定义头
    let client = Client::builder()
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .build::<_, hyper::Body>(TokioExecutor::new());
    
    let req = Request::builder()
        .uri(format!("http://127.0.0.1:8081/"))
        .header("X-Custom-Header", "test-value")
        .body(Body::empty())
        .unwrap();
    
    let resp = client.request(req).await.expect("Request failed");
    
    // 验证响应
    assert_eq!(resp.status(), StatusCode::OK);
    
    // 读取响应体（应包含请求头）
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let body_str = String::from_utf8_lossy(&body_bytes);
    
    // 验证自定义头被传递
    assert!(body_str.contains("x-custom-header:test-value"));
    
    // 应该包含代理添加的头
    assert!(body_str.contains("host:127.0.0.1:"));
    
    // 停止服务器
    proxy_server.abort(http_handle, None).await;
} 