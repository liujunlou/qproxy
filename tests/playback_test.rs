use qproxy::{
    model::{TrafficRecord, Protocol, RequestData, ResponseData},
    playback::PlaybackService,
};
use std::time::SystemTime;
use redis::{Client, AsyncCommands};
use tokio::test;

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

fn create_sample_record() -> TrafficRecord {
    TrafficRecord {
        id: uuid::Uuid::new_v4().to_string(),
        protocol: Protocol::HTTP,
        timestamp: SystemTime::now(),
        request: RequestData {
            method: Some("GET".to_string()),
            service_name: Some("test-service".to_string()),
            params: Some(vec![("param1".to_string(), "value1".to_string())]),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: b"test request body".to_vec(),
        },
        response: ResponseData {
            status: Some(200),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: b"test response body".to_vec(),
        },
    }
}

#[test]
async fn test_save_and_get_record() {
    // 初始化模拟Redis
    let mock_redis = MockRedis::new().await;
    mock_redis.cleanup("test:").await.expect("Failed to cleanup Redis");
    
    // 创建PlaybackService
    let playback_service = PlaybackService::new_with_connection(mock_redis.conn_manager.clone())
        .await
        .expect("Failed to create playback service");
    
    // 创建测试记录
    let record = create_sample_record();
    let record_id = record.id.clone();
    
    // 保存记录
    playback_service.save_record(&record).await.expect("Failed to save record");
    
    // 获取记录
    let retrieved_record = playback_service.get_record(&record_id)
        .await
        .expect("Failed to get record");
    
    // 验证记录是否匹配
    assert_eq!(retrieved_record.id, record.id);
    assert_eq!(retrieved_record.protocol, record.protocol);
    assert_eq!(retrieved_record.request.service_name, record.request.service_name);
    
    // 清理
    mock_redis.cleanup("test:").await.expect("Failed to cleanup Redis");
}

#[test]
async fn test_list_records() {
    // 初始化模拟Redis
    let mock_redis = MockRedis::new().await;
    mock_redis.cleanup("test:").await.expect("Failed to cleanup Redis");
    
    // 创建PlaybackService
    let playback_service = PlaybackService::new_with_connection(mock_redis.conn_manager.clone())
        .await
        .expect("Failed to create playback service");
    
    // 创建多个测试记录
    let record1 = create_sample_record();
    let record2 = create_sample_record();
    
    // 保存记录
    playback_service.save_record(&record1).await.expect("Failed to save record");
    playback_service.save_record(&record2).await.expect("Failed to save record");
    
    // 获取记录列表
    let records = playback_service.list_records(Some("test-service".to_string()), None)
        .await
        .expect("Failed to list records");
    
    // 验证记录列表不为空
    assert!(!records.is_empty());
    assert!(records.len() >= 2);
    
    // 清理
    mock_redis.cleanup("test:").await.expect("Failed to cleanup Redis");
}

#[test]
async fn test_delete_record() {
    // 初始化模拟Redis
    let mock_redis = MockRedis::new().await;
    mock_redis.cleanup("test:").await.expect("Failed to cleanup Redis");
    
    // 创建PlaybackService
    let playback_service = PlaybackService::new_with_connection(mock_redis.conn_manager.clone())
        .await
        .expect("Failed to create playback service");
    
    // 创建测试记录
    let record = create_sample_record();
    let record_id = record.id.clone();
    
    // 保存记录
    playback_service.save_record(&record).await.expect("Failed to save record");
    
    // 删除记录
    playback_service.delete_record(&record_id).await.expect("Failed to delete record");
    
    // 尝试获取已删除的记录
    let result = playback_service.get_record(&record_id).await;
    assert!(result.is_err());
    
    // 清理
    mock_redis.cleanup("test:").await.expect("Failed to cleanup Redis");
} 