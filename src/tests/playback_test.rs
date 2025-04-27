use qproxy::{
    model::{Protocol, TrafficRecord, RequestData, ResponseData},
    playback::PlaybackService,
    service_discovery::{ServiceInstance, ServiceRegistry, SERVICE_REGISTRY},
};
use bytes::Bytes;
use http::{Request, Response, Method};
use http_body_util::{BodyExt, Full};
use std::{collections::HashMap, time::{SystemTime, Duration}};

#[tokio::test]
async fn test_record_storage_and_retrieval() {
    let service = PlaybackService::new();
    let record = TrafficRecord::new_http(
        "GET".to_string(),
        "http://test-service/api/test".to_string(),
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"request body".to_vec(),
        200,
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"response body".to_vec(),
    );

    service.add_record(record.clone()).await;
    
    let req = Request::builder()
        .method("GET")
        .uri("http://test-service/api/test")
        .body(Full::new(Bytes::from("")))
        .unwrap();

    let found_record = service.find_matching_record(&req).await.unwrap();
    assert_eq!(found_record.request.method, Some("GET".to_string()));
    assert_eq!(found_record.request.uri, Some("http://test-service/api/test".to_string()));
}

#[tokio::test]
async fn test_playback_with_local_service() {
    // 注册本地服务
    let instance = ServiceInstance {
        name: "test-service".to_string(),
        host: "localhost".to_string(),
        port: 8080,
        metadata: HashMap::new(),
    };
    SERVICE_REGISTRY.register(instance).await;

    let service = PlaybackService::new();
    let record = TrafficRecord::new_http(
        "GET".to_string(),
        "http://test-service/api/test".to_string(),
        vec![],
        vec![],
        200,
        vec![],
        b"recorded response".to_vec(),
    );

    service.add_record(record).await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/test")
        .body(Full::new(Bytes::from("")))
        .unwrap();

    let response = service.playback(req).await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_recent_records_filtering() {
    let service = PlaybackService::new();
    let now = SystemTime::now();
    let old_time = now - Duration::from_secs(7200); // 2小时前

    // 添加一个旧记录
    let old_record = TrafficRecord {
        id: "old".to_string(),
        protocol: Protocol::HTTP,
        timestamp: old_time,
        request: RequestData {
            method: Some("GET".to_string()),
            uri: Some("http://test/old".to_string()),
            headers: None,
            body: vec![],
        },
        response: ResponseData {
            status: Some(200),
            headers: None,
            body: vec![],
        },
    };

    // 添加一个新记录
    let new_record = TrafficRecord {
        id: "new".to_string(),
        protocol: Protocol::HTTP,
        timestamp: now,
        request: RequestData {
            method: Some("GET".to_string()),
            uri: Some("http://test/new".to_string()),
            headers: None,
            body: vec![],
        },
        response: ResponseData {
            status: Some(200),
            headers: None,
            body: vec![],
        },
    };

    service.add_record(old_record).await;
    service.add_record(new_record).await;

    // 获取最近1小时的记录
    let recent_records = service.get_recent_records(Some(now - Duration::from_secs(3600))).await;
    assert_eq!(recent_records.len(), 1);
    assert_eq!(recent_records[0].id, "new");
}

#[tokio::test]
async fn test_add_and_get_records() {
    let service = PlaybackService::new();
    
    // 创建测试记录
    let record = TrafficRecord::new_http(
        "GET".to_string(),
        "/test".to_string(),
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"request body".to_vec(),
        200,
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"response body".to_vec(),
    );

    // 添加记录
    service.add_record(record.clone()).await;

    // 获取所有记录
    let records = service.get_all_records().await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].request.method, Some("GET".to_string()));
    assert_eq!(records[0].request.uri, Some("/test".to_string()));
    assert_eq!(records[0].response.status, Some(200));
}

#[tokio::test]
async fn test_playback_matching() {
    let service = PlaybackService::new();
    
    // 添加测试记录
    let record = TrafficRecord::new_http(
        "POST".to_string(),
        "/api/data".to_string(),
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"{\"key\":\"value\"}".to_vec(),
        201,
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"{\"status\":\"created\"}".to_vec(),
    );
    service.add_record(record).await;

    // 创建匹配的请求
    let req = Request::builder()
        .method("POST")
        .uri("/api/data")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from("{\"key\":\"value\"}")))
        .unwrap();

    // 测试回放
    let resp = service.playback(req).await;
    assert_eq!(resp.status(), 201);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"{\"status\":\"created\"}");
}

#[tokio::test]
async fn test_playback_no_match() {
    let service = PlaybackService::new();
    
    // 创建不匹配的请求
    let req = Request::builder()
        .method("GET")
        .uri("/not/exist")
        .body(Full::new(Bytes::from("")))
        .unwrap();

    // 测试回放
    let resp = service.playback(req).await;
    assert_eq!(resp.status(), 404);
} 