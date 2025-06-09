use qproxy::{
    model::{Protocol, RequestData, ResponseData, TrafficRecord},
    playback::{PlaybackService, SyncStatus},
    service_discovery::ServiceInstance, SERVICE_REGISTRY,
};
use std::collections::HashMap;

#[tokio::test]
async fn test_playback_service_creation() {
    let redis_url = "redis://localhost:6379";
    let service = PlaybackService::new_with_url(redis_url).await;
    assert!(service.is_ok());
}

#[tokio::test]
async fn test_add_and_get_record() {
    let redis_url = "redis://localhost:6379";
    let service = PlaybackService::new_with_url(redis_url).await.unwrap();
    
    // 创建测试记录
    let record = TrafficRecord {
        id: "test-1".to_string(),
        peer_id: "test-peer".to_string(),
        protocol: Protocol::HTTP,
        codec: None,
        timestamp: chrono::Utc::now().timestamp_millis() as u128,
        request: RequestData {
            method: Some("GET".to_string()),
            service_name: Some("/api/test".to_string()),
            params: Some(vec![("key".to_string(), "value".to_string())]),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: vec![],
        },
        response: ResponseData {
            status: Some(200),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: vec![],
        },
    };

    // 添加记录
    assert!(service.add_record(record.clone()).await.is_ok());

    // 获取记录
    let records = service.get_recent_records("test-peer", "default", None).await.unwrap();
    assert!(!records.is_empty());
    assert_eq!(records[0].id, record.id);
}

#[tokio::test]
async fn test_trigger_replay() {
    let redis_url = "redis://localhost:6379";
    let service = PlaybackService::new_with_url(redis_url).await.unwrap();
    
    // 注册测试服务
    let instance = ServiceInstance {
        name: "test-service".to_string(),
        host: "localhost".to_string(),
        port: 8080,
        metadata: HashMap::new(),
    };
    SERVICE_REGISTRY.write().await.register(instance).await;

    // 创建测试记录
    let record = TrafficRecord {
        id: "test-2".to_string(),
        peer_id: "test-peer".to_string(),
        protocol: Protocol::HTTP,
        codec: None,
        timestamp: chrono::Utc::now().timestamp_millis() as u128,
        request: RequestData {
            method: Some("GET".to_string()),
            service_name: Some("test-service".to_string()),
            params: Some(vec![]),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: vec![],
        },
        response: ResponseData {
            status: Some(200),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: vec![],
        },
    };

    // 触发回放
    service.trigger_replay(&record).await;
}

#[tokio::test]
async fn test_sync_operations() {
    let redis_url = "redis://localhost:6379";
    let service = PlaybackService::new_with_url(redis_url).await.unwrap();
    
    // 测试获取同步锁
    let acquired = service.acquire_sync_lock("test-peer", "default").await.unwrap();
    assert!(acquired);

    // 测试更新checkpoint
    let checkpoint = qproxy::playback::CheckpointInfo {
        peer_id: "test-peer".to_string(),
        shard_id: "default".to_string(),
        last_sync_time: chrono::Utc::now().timestamp_millis() as u128,
        last_record_id: "test-3".to_string(),
        status: SyncStatus::Completed,
        retry_count: 0,
        error_message: None,
    };
    assert!(service.update_checkpoint(&checkpoint).await.is_ok());

    // 测试释放同步锁
    assert!(service.release_sync_lock("test-peer", "default").await.is_ok());
}

#[tokio::test]
async fn test_local_records_management() {
    let redis_url = "redis://localhost:6379";
    let service = PlaybackService::new_with_url(redis_url).await.unwrap();
    
    // 创建测试记录
    let record = TrafficRecord {
        id: "test-4".to_string(),
        peer_id: "test-peer".to_string(),
        protocol: Protocol::HTTP,
        codec: None,
        timestamp: chrono::Utc::now().timestamp_millis() as u128,
        request: RequestData {
            method: Some("GET".to_string()),
            service_name: Some("/api/test".to_string()),
            params: Some(vec![]),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: vec![],
        },
        response: ResponseData {
            status: Some(200),
            headers: Some(vec![("Content-Type".to_string(), "application/json".to_string())]),
            body: vec![],
        },
    };

    // 添加本地记录
    assert!(service.add_local_record(record.clone()).await.is_ok());

    // 清理本地记录
    assert!(service.clear_local_records(record.clone()).await.is_ok());
} 