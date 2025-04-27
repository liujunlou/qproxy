use qproxy::{
    model::TrafficRecord,
    record::RecordService,
};
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;

#[tokio::test]
async fn test_record_request_response() {
    let service = RecordService::new();
    
    // 创建测试请求
    let req = Request::builder()
        .method("POST")
        .uri("/api/test")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from("{\"test\":\"data\"}")))
        .unwrap();

    // 创建测试响应
    let resp = Response::builder()
        .status(201)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from("{\"status\":\"success\"}")))
        .unwrap();

    // 记录请求和响应
    service.record_request(&req).await;
    service.record_response(&resp).await;

    // 获取记录并验证
    let records = service.get_all_records().await;
    assert_eq!(records.len(), 1);
    
    let record = &records[0];
    assert_eq!(record.request.method, Some("POST".to_string()));
    assert_eq!(record.request.uri, Some("/api/test".to_string()));
    assert_eq!(record.request.headers.get("Content-Type").unwrap(), "application/json");
    assert_eq!(record.request.body, b"{\"test\":\"data\"}".to_vec());
    
    assert_eq!(record.response.status, Some(201));
    assert_eq!(record.response.headers.get("Content-Type").unwrap(), "application/json");
    assert_eq!(record.response.body, b"{\"status\":\"success\"}".to_vec());
}

#[tokio::test]
async fn test_clear_records() {
    let service = RecordService::new();
    
    // 创建并记录一个简单的请求和响应
    let req = Request::builder()
        .method("GET")
        .uri("/test")
        .body(Full::new(Bytes::from("")))
        .unwrap();
    let resp = Response::builder()
        .status(200)
        .body(Full::new(Bytes::from("")))
        .unwrap();

    service.record_request(&req).await;
    service.record_response(&resp).await;

    // 验证记录已添加
    assert_eq!(service.get_all_records().await.len(), 1);

    // 清除记录
    service.clear_records().await;

    // 验证记录已清除
    assert_eq!(service.get_all_records().await.len(), 0);
}

#[tokio::test]
async fn test_multiple_records() {
    let service = RecordService::new();
    
    // 创建多个请求和响应
    for i in 0..3 {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/test/{}", i))
            .body(Full::new(Bytes::from("")))
            .unwrap();
        let resp = Response::builder()
            .status(200)
            .body(Full::new(Bytes::from(format!("response {}", i))))
            .unwrap();

        service.record_request(&req).await;
        service.record_response(&resp).await;
    }

    // 验证所有记录
    let records = service.get_all_records().await;
    assert_eq!(records.len(), 3);
    
    for (i, record) in records.iter().enumerate() {
        assert_eq!(record.request.uri, Some(format!("/test/{}", i)));
        assert_eq!(record.response.body, format!("response {}", i).as_bytes());
    }
} 