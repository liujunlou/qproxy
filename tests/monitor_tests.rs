use qproxy::{
    monitor::{MetricsCollector, HTTP_REQUESTS_TOTAL, GRPC_REQUESTS_TOTAL},
    options::ProxyMode,
};

#[tokio::test]
async fn test_metrics_collector() {
    let collector = MetricsCollector::new(ProxyMode::Record);
    
    // 测试记录 HTTP 请求
    HTTP_REQUESTS_TOTAL.inc();
    
    // 测试记录 gRPC 请求
    GRPC_REQUESTS_TOTAL.inc();
    
    // 验证健康状态
    let health_status = collector.get_health_status().await;
    assert_eq!(health_status.status, qproxy::monitor::ServiceStatus::Up);
    assert_eq!(health_status.mode, ProxyMode::Record);
    
    // 验证组件状态
    assert_eq!(health_status.components.redis, qproxy::monitor::ServiceStatus::Up);
    assert_eq!(health_status.components.recording, qproxy::monitor::ServiceStatus::Up);
    assert_eq!(health_status.components.system, qproxy::monitor::ServiceStatus::Up);
}

#[tokio::test]
async fn test_metrics_recording() {
    // 测试 HTTP 指标记录
    HTTP_REQUESTS_TOTAL.inc();
    HTTP_REQUESTS_TOTAL.inc();
    
    // 测试 gRPC 指标记录
    GRPC_REQUESTS_TOTAL.inc();
    
    // 验证指标被正确记录（通过检查计数器值）
    // 注意：由于是静态变量，这些值会在测试之间累积
    assert!(HTTP_REQUESTS_TOTAL.get() >= 2.0);
    assert!(GRPC_REQUESTS_TOTAL.get() >= 1.0);
}

#[tokio::test]
async fn test_health_status_updates() {
    let collector = MetricsCollector::new(ProxyMode::Playback);
    
    // 初始状态应该是 Up
    let initial_status = collector.get_health_status().await;
    assert_eq!(initial_status.status, qproxy::monitor::ServiceStatus::Up);
    
    // 更新组件状态
    collector.update_component_status("redis", qproxy::monitor::ServiceStatus::Down).await;
    
    // 验证组件状态已更新
    let updated_status = collector.get_health_status().await;
    assert_eq!(updated_status.components.redis, qproxy::monitor::ServiceStatus::Down);
} 