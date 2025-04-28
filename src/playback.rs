use crate::{model::{Protocol, TrafficRecord}, service_discovery::SERVICE_REGISTRY};
use bytes::Bytes;
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use hyper_util::{client::legacy::{Builder, Client}, rt::TokioExecutor};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{SystemTime, Duration, UNIX_EPOCH};
use tracing::{info, warn, error};
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use redis::{aio::ConnectionManager, AsyncCommands};
use serde::{Serialize, Deserialize};

const CHECKPOINT_KEY_PREFIX: &str = "qproxy:checkpoint:";
const SYNC_LOCK_PREFIX: &str = "qproxy:sync_lock:";
const LOCK_EXPIRE_SECS: u64 = 300; // 5分钟锁过期时间

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub peer_id: String,
    pub shard_id: String,
    pub last_sync_time: u64,
    pub last_record_id: String,
    pub status: SyncStatus,
    pub retry_count: u32,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SyncStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// 流量回放服务
/// 
/// 该服务负责:
/// 1. 存储和管理流量记录
/// 2. 查找匹配的流量记录
/// 3. 回放流量到本地服务
/// 4. 管理最近的流量记录
/// 5. 跟踪同步位点，确保数据完整性
#[derive(Clone)]
pub struct PlaybackService {
    /// 存储流量记录的哈希表，使用 RwLock 保证并发安全
    records: Arc<RwLock<HashMap<String, TrafficRecord>>>,
    /// Tokio 执行器，用于异步任务调度
    executor: Arc<TokioExecutor>,
    /// Redis 连接管理器
    redis: ConnectionManager,
}

impl PlaybackService {
    /// 使用Redis URL创建服务实例
    pub async fn new(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let redis = ConnectionManager::new(client).await?;
        Self::new_with_connection(redis).await
    }

    /// 使用已有的Redis连接创建服务实例
    pub async fn new_with_connection(redis: ConnectionManager) -> Result<Self, redis::RedisError> {
        Ok(Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            executor: Arc::new(TokioExecutor::new()),
            redis,
        })
    }

    /// 获取checkpoint的Redis key
    fn get_checkpoint_key(&self, peer_id: &str, shard_id: &str) -> String {
        format!("{}{}:{}", CHECKPOINT_KEY_PREFIX, peer_id, shard_id)
    }

    /// 获取同步锁的Redis key
    fn get_lock_key(&self, peer_id: &str, shard_id: &str) -> String {
        format!("{}{}:{}", SYNC_LOCK_PREFIX, peer_id, shard_id)
    }

    /// 尝试获取同步锁
    async fn try_acquire_sync_lock(&self, peer_id: &str, shard_id: &str) -> Result<bool, redis::RedisError> {
        let lock_key = self.get_lock_key(peer_id, shard_id);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let expire_at = now + LOCK_EXPIRE_SECS;

        // 使用 SETNX 设置锁
        let acquired: bool = self.redis.clone()
            .set_nx(&lock_key, expire_at.to_string()).await?;

        if acquired {
            // 设置过期时间
            self.redis.clone()
                .expire(&lock_key, LOCK_EXPIRE_SECS as i64).await?;
        }

        Ok(acquired)
    }

    /// 释放同步锁
    async fn release_sync_lock(&self, peer_id: &str, shard_id: &str) -> Result<(), redis::RedisError> {
        let lock_key = self.get_lock_key(peer_id, shard_id);
        self.redis.clone().del(&lock_key).await?;
        Ok(())
    }

    /// 更新同步位点
    pub async fn update_checkpoint(&self, checkpoint: CheckpointInfo) -> Result<(), redis::RedisError> {
        let key = self.get_checkpoint_key(&checkpoint.peer_id, &checkpoint.shard_id);
        let json = serde_json::to_string(&checkpoint).unwrap();
        self.redis.clone().set(&key, json).await?;
        Ok(())
    }

    /// 获取同步位点
    pub async fn get_checkpoint(&self, peer_id: &str, shard_id: &str) -> Result<Option<CheckpointInfo>, redis::RedisError> {
        let key = self.get_checkpoint_key(peer_id, shard_id);
        let json: Option<String> = self.redis.clone().get(&key).await?;
        
        Ok(json.map(|j| serde_json::from_str(&j).unwrap()))
    }

    /// 开始同步流程
    pub async fn start_sync(&self, peer_id: &str, shard_id: &str) -> Result<bool, redis::RedisError> {
        // 尝试获取锁
        if !self.try_acquire_sync_lock(peer_id, shard_id).await? {
            return Ok(false);
        }

        // 获取或创建checkpoint
        let mut checkpoint = self.get_checkpoint(peer_id, shard_id).await?
            .unwrap_or_else(|| CheckpointInfo {
                peer_id: peer_id.to_string(),
                shard_id: shard_id.to_string(),
                last_sync_time: 0,
                last_record_id: String::new(),
                status: SyncStatus::Pending,
                retry_count: 0,
                error_message: None,
            });

        // 更新状态为进行中
        checkpoint.status = SyncStatus::InProgress;
        self.update_checkpoint(checkpoint).await?;

        Ok(true)
    }

    /// 完成同步流程
    pub async fn complete_sync(&self, peer_id: &str, shard_id: &str, success: bool, error_msg: Option<String>) -> Result<(), redis::RedisError> {
        let mut checkpoint = match self.get_checkpoint(peer_id, shard_id).await? {
            Some(cp) => cp,
            None => {
                error!("No checkpoint found for peer: {}, shard: {}", peer_id, shard_id);
                return Ok(());
            }
        };

        // 更新状态
        if success {
            checkpoint.status = SyncStatus::Completed;
            checkpoint.retry_count = 0;
            checkpoint.error_message = None;
        } else {
            checkpoint.status = SyncStatus::Failed;
            checkpoint.retry_count += 1;
            checkpoint.error_message = error_msg;
        }

        // 更新checkpoint
        self.update_checkpoint(checkpoint).await?;

        // 释放锁
        self.release_sync_lock(peer_id, shard_id).await?;

        Ok(())
    }

    /// 获取需要同步的记录
    /// 
    /// # 参数
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    /// 
    /// # 返回值
    /// 返回需要同步的记录列表
    pub async fn get_records_for_sync(&self, peer_id: &str, shard_id: &str) -> Result<Vec<TrafficRecord>, redis::RedisError> {
        let checkpoint = self.get_checkpoint(peer_id, shard_id).await?;
        let records = self.records.read().await;
        
        let since = checkpoint.as_ref()
            .map(|cp| UNIX_EPOCH + Duration::from_secs(cp.last_sync_time))
            .unwrap_or_else(|| SystemTime::now() - Duration::from_secs(3600));

        let mut new_records: Vec<TrafficRecord> = records.values()
            .filter(|record| {
                if let Some(cp) = &checkpoint {
                    record.timestamp > since && record.id != cp.last_record_id
                } else {
                    record.timestamp > since
                }
            })
            .cloned()
            .collect();

        // 按时间戳排序
        new_records.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(new_records)
    }

    /// 添加新的流量记录
    /// 
    /// # 参数
    /// * `record` - 要添加的流量记录
    pub async fn add_record(&self, record: TrafficRecord) {
        let record_clone = record.clone();
        let mut records = self.records.write().await;
        records.insert(record.id.clone(), record);
        
        // 触发回放
        self.trigger_replay(&record_clone).await;
    }

    /// 清除所有流量记录
    pub async fn clear_records(&self) {
        let mut records = self.records.write().await;
        records.clear();
    }

    /// 查找匹配的流量记录
    /// 
    /// # 参数
    /// * `req` - HTTP 请求
    /// 
    /// # 返回值
    /// 返回匹配的流量记录（如果找到）
    pub async fn find_matching_record<B>(&self, req: &Request<B>) -> Option<TrafficRecord>
    where
        B: Body,
    {
        let records = self.records.read().await;
        
        records.values()
            .find(|record| {
                matches!(record.protocol, Protocol::HTTP | Protocol::HTTPS)
                    && record.request.method.as_ref() == Some(&req.method().to_string())
                    && record.request.service_name.as_ref() == Some(&req.uri().to_string())
            })
            .cloned()
    }

    /// 回放流量
    /// 
    /// 该方法会:
    /// 1. 查找匹配的流量记录
    /// 2. 根据协议类型选择对应的回放方式
    /// 3. 尝试连接本地服务并回放流量
    /// 4. 如果本地服务不可用，则返回记录的响应
    /// 
    /// # 参数
    /// * `req` - HTTP 请求
    /// 
    /// # 返回值
    /// 返回 HTTP 响应
    pub async fn playback<B>(&self, req: Request<B>) -> Response<Full<Bytes>>
    where
        B: Body,
    {
        if let Some(record) = self.find_matching_record(&req).await {
            // 尝试查找本地服务
            if let Some(service_name) = record.request.service_name.as_ref() {
                match SERVICE_REGISTRY.read().await.find_local_service(service_name).await {
                    Ok(Some(service)) => {
                        // 构建本地服务连接地址
                        let addr = format!("{}:{}", service.host, service.port);
                        info!("Replaying traffic to local service: {}", addr);

                        // 根据协议类型选择回放方式
                        match record.protocol {
                            Protocol::TCP => {
                                // TCP 协议回放
                                match TcpStream::connect(&addr).await {
                                    Ok(mut stream) => {
                                        // 发送请求数据到本地服务
                                        if let Err(e) = stream.write_all(&record.request.body).await {
                                            warn!("Failed to write request to local service: {}", e);
                                        } else {
                                            // 读取本地服务的响应数据
                                            let mut response_data = Vec::new();
                                            let mut buffer = [0u8; 8192];
                                            
                                            // 循环读取响应数据直到 EOF 或错误发生
                                            loop {
                                                match stream.read(&mut buffer).await {
                                                    Ok(0) => break, // EOF
                                                    Ok(n) => response_data.extend_from_slice(&buffer[..n]),
                                                    Err(e) => {
                                                        warn!("Failed to read response from local service: {}", e);
                                                        break;
                                                    }
                                                }
                                            }

                                            // 如果成功读取到响应数据，则返回
                                            if !response_data.is_empty() {
                                                info!("Successfully replayed TCP traffic to local service");
                                                return Response::new(Full::new(Bytes::from(response_data)));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to establish TCP connection: {}, falling back to recorded response", e);
                                    }
                                }
                            }
                            Protocol::HTTP | Protocol::HTTPS => {
                                // HTTP/HTTPS 协议回放
                                let scheme = if record.protocol == Protocol::HTTPS { "https" } else { "http" };
                                let local_url = format!("{}://{}:{}{}", 
                                    scheme,
                                    service.host, 
                                    service.port,
                                    req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("")
                                );

                                info!("Replaying HTTP/HTTPS traffic to local service: {}", local_url);

                                // 构建并发送 HTTP 请求
                                let local_req = Request::builder()
                                    .method(req.method())
                                    .uri(local_url);

                                // 添加请求头
                                let local_req = if let Some(headers) = &record.request.headers {
                                    let mut builder = local_req;
                                    for (name, value) in headers {
                                        builder = builder.header(name, value);
                                    }
                                    builder
                                } else {
                                    local_req
                                };

                                // 发送请求
                                match local_req.body(Full::new(Bytes::from(record.request.body.clone()))) {
                                    Ok(request) => {
                                        let client = Builder::new((*self.executor).clone())
                                            .pool_idle_timeout(std::time::Duration::from_secs(30))
                                            .build_http();

                                        match client.request(request).await {
                                            Ok(resp) => {
                                                info!("Successfully replayed HTTP/HTTPS traffic to local service");
                                                let body = resp.into_body();
                                                match body.collect().await {
                                                    Ok(collected) => {
                                                        let bytes = collected.to_bytes();
                                                        return Response::new(Full::new(bytes));
                                                    }
                                                    Err(e) => {
                                                        warn!("Failed to collect response body: {}, falling back to recorded response", e);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                warn!("Failed to send HTTP/HTTPS request: {}, falling back to recorded response", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to build HTTP/HTTPS request: {}, falling back to recorded response", e);
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        warn!("No local service found for {}, using recorded response", service_name);
                    }
                    Err(e) => {
                        warn!("Error finding local service: {}, using recorded response", e);
                    }
                }
            }

            // 如果本地服务不可用或出错，返回记录的响应
            let mut response = Response::builder()
                .status(record.response.status.unwrap_or(200));

            // 添加响应头
            if let Some(headers) = record.response.headers {
                for (name, value) in headers {
                    response = response.header(name, value);
                }
            }

            // 构建并返回响应
            response.body(Full::new(Bytes::from(record.response.body)))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::new(Bytes::from("Failed to construct response")))
                        .unwrap()
                })
        } else {
            // 如果没有找到匹配的记录，返回 404
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("No matching record found")))
                .unwrap()
        }
    }

    /// 获取最近的流量记录
    /// 
    /// # 参数
    /// * `since` - 起始时间点，如果为 None 则默认获取最近一小时的记录
    /// 
    /// # 返回值
    /// 返回指定时间段内的流量记录列表
    pub async fn get_recent_records(&self, since: Option<SystemTime>) -> Vec<TrafficRecord> {
        let records = self.records.read().await;
        let since = since.unwrap_or_else(|| {
            SystemTime::now() - Duration::from_secs(3600) // 默认获取最近一小时的记录
        });

        records.values()
            .filter(|record| record.timestamp >= since)
            .cloned()
            .collect()
    }

    /// 获取所有流量记录
    /// 
    /// # 返回值
    /// 返回所有存储的流量记录列表
    pub async fn get_all_records(&self) -> Vec<TrafficRecord> {
        let records = self.records.read().await;
        records.values().cloned().collect()
    }

    /// 同步来自peer的记录
    /// 
    /// # 参数
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    /// * `records` - 要同步的记录列表
    pub async fn sync_from_peer(&self, peer_id: &str, shard_id: &str, records: Vec<TrafficRecord>) -> Result<(), redis::RedisError> {
        if records.is_empty() {
            return Ok(());
        }

        // 开始同步
        if !self.start_sync(peer_id, shard_id).await? {
            warn!("Failed to acquire sync lock for peer: {}, shard: {}", peer_id, shard_id);
            return Ok(());
        }

        let result = async {
            // 按时间戳排序
            let mut sorted_records = records;
            sorted_records.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

            // 获取最后一条记录的信息
            let last_record = sorted_records.last().unwrap();
            let last_sync_time = last_record.timestamp
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // 添加所有记录
            for record in sorted_records.clone() {
                self.add_record(record.clone()).await;
            }

            // 更新checkpoint
            let checkpoint = CheckpointInfo {
                peer_id: peer_id.to_string(),
                shard_id: shard_id.to_string(),
                last_sync_time,
                last_record_id: last_record.id.clone(),
                status: SyncStatus::Completed,
                retry_count: 0,
                error_message: None,
            };
            self.update_checkpoint(checkpoint).await?;

            Ok::<_, redis::RedisError>(())
        }.await;

        match result {
            Ok(_) => {
                self.complete_sync(peer_id, shard_id, true, None).await?;
                info!("Successfully synced records from peer: {}, shard: {}", peer_id, shard_id);
            }
            Err(e) => {
                self.complete_sync(peer_id, shard_id, false, Some(e.to_string())).await?;
                error!("Failed to sync records from peer: {}, shard: {}, error: {}", peer_id, shard_id, e);
            }
        }

        Ok(())
    }

    /// 获取所有同步位点信息
    pub async fn get_all_checkpoints(&self) -> Result<Vec<CheckpointInfo>, redis::RedisError> {
        let mut conn = self.redis.clone();
        let pattern = format!("{}*", CHECKPOINT_KEY_PREFIX);
        let keys: Vec<String> = conn.keys(&pattern).await?;
        
        let mut checkpoints = Vec::new();
        for key in keys {
            let json: String = conn.get(&key).await?;
            if let Ok(checkpoint) = serde_json::from_str(&json) {
                checkpoints.push(checkpoint);
            }
        }
        
        Ok(checkpoints)
    }

    /// 触发流量回放
    /// 
    /// 当收到新的流量记录时调用此方法来触发回放
    /// 支持以下场景:
    /// 1. HTTP/TCP 录制流量
    /// 2. 从 peer 同步的流量
    /// 3. 轮询获取的新流量
    pub async fn trigger_replay(&self, record: &TrafficRecord) {
        if let Some(service_name) = record.request.service_name.as_ref() {
            match SERVICE_REGISTRY.read().await.find_local_service(service_name).await {
                Ok(Some(service)) => {
                    // 构建本地服务连接地址
                    let addr = format!("{}:{}", service.host, service.port);
                    info!("Triggering traffic replay to local service: {}", addr);

                    match record.protocol {
                        Protocol::TCP => {
                            if let Ok(mut stream) = TcpStream::connect(&addr).await {
                                if let Err(e) = stream.write_all(&record.request.body).await {
                                    warn!("Failed to replay TCP traffic: {}", e);
                                } else {
                                    info!("Successfully replayed TCP traffic to {}", addr);
                                }
                            }
                        }
                        Protocol::HTTP | Protocol::HTTPS => {
                            let scheme = if record.protocol == Protocol::HTTPS { "https" } else { "http" };
                            let local_url = format!("{}://{}:{}{}", 
                                scheme,
                                service.host, 
                                service.port,
                                record.request.params.as_ref().map(|u| u.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<String>>().join("&")).unwrap_or("".to_string())
                            );

                            let local_req = Request::builder()
                                .method(record.request.method.as_ref().unwrap_or(&"GET".to_string()).as_str())
                                .uri(&local_url);

                            let local_req = if let Some(headers) = &record.request.headers {
                                let mut builder = local_req;
                                for (name, value) in headers {
                                    builder = builder.header(name, value);
                                }
                                builder
                            } else {
                                local_req
                            };

                            if let Ok(request) = local_req.body(Full::new(Bytes::from(record.request.body.clone()))) {
                                let client = Builder::new((*self.executor).clone())
                                    .pool_idle_timeout(std::time::Duration::from_secs(30))
                                    .build_http();

                                match client.request(request).await {
                                    Ok(_) => info!("Successfully replayed HTTP traffic to {}", local_url),
                                    Err(e) => warn!("Failed to replay HTTP traffic: {}", e),
                                }
                            }
                        }
                    }
                }
                Ok(None) => warn!("No local service found for {}", service_name),
                Err(e) => warn!("Error finding local service: {}", e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::{RequestData, ResponseData}, service_discovery::{ServiceInstance, ServiceRegistry}};
    use http::Method;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_record_storage_and_retrieval() {
        let service = PlaybackService::new("redis://localhost:6379").await.unwrap();
        let record = TrafficRecord::new_http(
            "GET".to_string(),
            "http://test-service/api/test".to_string(),
            Some(vec![]),
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
        assert_eq!(found_record.request.service_name, Some("http://test-service/api/test".to_string()));
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
        SERVICE_REGISTRY.write().await.register(instance).await;

        let service = PlaybackService::new("redis://localhost:6379").await.unwrap();
        let record = TrafficRecord::new_http(
            "GET".to_string(),
            "http://test-service/api/test".to_string(),
            Some(vec![]),
            vec![],
            vec![],
            200,
            vec![],
            b"recorded response".to_vec(),
        );

        service.add_record(record.clone()).await;

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
        let service = PlaybackService::new("redis://localhost:6379").await.unwrap();
        let now = SystemTime::now();
        let old_time = now - Duration::from_secs(7200); // 2小时前

        // 添加一个旧记录
        let old_record = TrafficRecord {
            id: "old".to_string(),
            protocol: Protocol::HTTP,
            timestamp: old_time,
            request: RequestData {
                method: Some("GET".to_string()),
                service_name: Some("http://test/old".to_string()),
                params: Some(vec![]),
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
                service_name: Some("http://test/new".to_string()),
                params: Some(vec![("param1".to_string(), "value1".to_string())]),
                headers: None,
                body: vec![],
            },
            response: ResponseData {
                status: Some(200),
                headers: None,
                body: vec![],
            },
        };

        service.add_record(old_record.clone()).await;
        service.add_record(new_record.clone()).await;

        // 获取最近1小时的记录
        let recent_records = service.get_records_for_sync("test", "default").await.unwrap();
        assert_eq!(recent_records.len(), 1);
        assert_eq!(recent_records[0].id, "new");
    }
} 