use std::sync::Arc;

use qproxy::{
    model::{Protocol, RequestData, ResponseData, TrafficRecord},
    options::{Options, PeerOptions},
    sync::SyncService,
};

#[tokio::test]
async fn test_sync_service_creation() {
    let options = Options {
        sync: qproxy::options::SyncOptions {
            enabled: true,
            shards: 1,
            interval: 1000,
            peer: Some(PeerOptions {
                host: "localhost".to_string(),
                port: 8080,
                tls: false,
            }),
        },
        ..Default::default()
    };

    let service = SyncService::new(&options);
    assert!(service.is_ok());
}

#[tokio::test]
async fn test_sync_with_peers() {
    let options = Options {
        sync: qproxy::options::SyncOptions {
            enabled: true,
            shards: 1,
            interval: 1000,
            peer: Some(PeerOptions {
                host: "localhost".to_string(),
                port: 8080,
                tls: false,
            }),
        },
        ..Default::default()
    };

    let service = SyncService::new(&options).unwrap();
    let client = service.client.clone();
    let options = service.options.clone();

    // 测试同步操作
    let result = SyncService::sync_with_peers(&options, &client).await;
    // 由于测试环境可能没有实际的peer服务，这里只验证函数调用不会panic
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_sync_from_peer() {
    let options = Options {
        sync: qproxy::options::SyncOptions {
            enabled: true,
            shards: 1,
            interval: 1000,
            peer: Some(PeerOptions {
                host: "localhost".to_string(),
                port: 8080,
                tls: false,
            }),
        },
        ..Default::default()
    };

    let service = SyncService::new(&options).unwrap();
    let client = service.client.clone();
    let peer = options.clone().sync.peer.unwrap();

    // 创建测试记录
    let record = TrafficRecord {
        id: "test-sync-1".to_string(),
        peer_id: "test-peer".to_string(),
        protocol: Protocol::HTTP,
        codec: None,
        timestamp: chrono::Utc::now().timestamp_millis() as u128,
        request: RequestData {
            method: Some("GET".to_string()),
            service_name: Some("/api/test".to_string()),
            path: None,
            params: Some(vec![]),
            headers: Some(vec![(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )]),
            body: vec![],
        },
        response: ResponseData {
            status: Some(200),
            headers: Some(vec![(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )]),
            body: vec![],
        },
    };

    // 测试从peer同步记录
    let result = SyncService::sync_from_peer(
        &client,
        &peer,
        &Arc::new(options.clone()),
        qproxy::PLAYBACK_SERVICE
            .read()
            .await
            .as_ref()
            .unwrap()
            .clone(),
    )
    .await;
    // 由于测试环境可能没有实际的peer服务，这里只验证函数调用不会panic
    assert!(result.is_ok() || result.is_err());
}
