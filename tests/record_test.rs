use qproxy::{
    model::{TrafficRecord, Protocol},
    record::RecordService,
};
use std::sync::Arc;
use tokio::test;
use redis::Client;

// 模拟Redis，创建测试用例
struct MockRedis {
    conn_manager: redis::aio::ConnectionManager,
}

impl MockRedis {
    async fn new() -> Self {
        // 使用本地Redis服务器进行测试
        let client = Client::open("redis://127.0.0.1:6379").expect("Failed to create Redis client");
        let conn_manager = redis::aio::ConnectionManager::new(client)
            .await
            .expect("Failed to create connection manager");
        
        Self { conn_manager }
    }

    async fn cleanup(&self, prefix: &str) -> redis::RedisResult<()> {
        let mut conn = self.conn_manager.clone();
        // 查找所有匹配前缀的键
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{}*", prefix))
            .query_async(&mut conn)
            .await?;
        
        // 如果有键存在，则删除
        if !keys.is_empty() {
            redis::cmd("DEL")
                .arg(keys)
                .query_async(&mut conn)
                .await?;
        }

        Ok(())
    }
}

#[test]
async fn test_record_http_traffic() {
    // 初始化模拟Redis
    let mock_redis = MockRedis::new().await;
    mock_redis.cleanup("record:").await.expect("Failed to cleanup Redis");
    
    // 创建RecordService
    let record_service = RecordService::new(mock_redis.conn_manager.clone());
    
    // 创建HTTP流量记录
    let record = TrafficRecord::new_http(
        "GET".to_string(),
        "test-service".to_string(),
        Some(vec![("param1".to_string(), "value1".to_string())]),
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"test request body".to_vec(),
        200,
        vec![("Content-Type".to_string(), "application/json".to_string())],
        b"test response body".to_vec(),
    );
    
    // 记录流量
    record_service.record_traffic(&record).await.expect("Failed to record traffic");
    
    // 验证记录是否保存成功
    let mut conn = mock_redis.conn_manager.clone();
    let exists: bool = redis::cmd("EXISTS")
        .arg(format!("record:traffic:{}", record.id))
        .query_async(&mut conn)
        .await
        .expect("Failed to check if record exists");
    
    assert!(exists, "Record was not saved to Redis");
    
    // 清理
    mock_redis.cleanup("record:").await.expect("Failed to cleanup Redis");
}

#[test]
async fn test_record_tcp_traffic() {
    // 初始化模拟Redis
    let mock_redis = MockRedis::new().await;
    mock_redis.cleanup("record:").await.expect("Failed to cleanup Redis");
    
    // 创建RecordService
    let record_service = RecordService::new(mock_redis.conn_manager.clone());
    
    // 创建TCP流量记录
    let record = TrafficRecord::new_tcp(
        b"test request data".to_vec(),
        b"test response data".to_vec(),
    );
    
    // 记录流量
    record_service.record_traffic(&record).await.expect("Failed to record traffic");
    
    // 验证记录是否保存成功
    let mut conn = mock_redis.conn_manager.clone();
    let exists: bool = redis::cmd("EXISTS")
        .arg(format!("record:traffic:{}", record.id))
        .query_async(&mut conn)
        .await
        .expect("Failed to check if record exists");
    
    assert!(exists, "Record was not saved to Redis");
    
    // 清理
    mock_redis.cleanup("record:").await.expect("Failed to cleanup Redis");
}

#[test]
async fn test_filter_sensitive_headers() {
    // 初始化模拟Redis
    let mock_redis = MockRedis::new().await;
    mock_redis.cleanup("record:").await.expect("Failed to cleanup Redis");
    
    // 创建RecordService
    let record_service = RecordService::new(mock_redis.conn_manager.clone());
    
    // 创建带有敏感头的HTTP流量记录
    let mut record = TrafficRecord::new_http(
        "GET".to_string(),
        "test-service".to_string(),
        Some(vec![("param1".to_string(), "value1".to_string())]),
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), "Bearer token123".to_string()),
            ("Cookie".to_string(), "session=abc123".to_string()),
        ],
        b"test request body".to_vec(),
        200,
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Set-Cookie".to_string(), "session=abc123".to_string()),
        ],
        b"test response body".to_vec(),
    );
    
    // 过滤敏感头
    record_service.filter_sensitive_headers(&mut record);
    
    // 验证敏感头是否被过滤
    let request_headers = record.request.headers.unwrap();
    let response_headers = record.response.headers.unwrap();
    
    // 请求头过滤验证
    assert!(request_headers.iter().any(|(k, _)| k == "Content-Type"));
    assert!(!request_headers.iter().any(|(k, v)| k == "Authorization" && v.contains("token123")));
    assert!(!request_headers.iter().any(|(k, v)| k == "Cookie" && v.contains("abc123")));
    
    // 响应头过滤验证
    assert!(response_headers.iter().any(|(k, _)| k == "Content-Type"));
    assert!(!response_headers.iter().any(|(k, v)| k == "Set-Cookie" && v.contains("abc123")));
    
    // 清理
    mock_redis.cleanup("record:").await.expect("Failed to cleanup Redis");
} 