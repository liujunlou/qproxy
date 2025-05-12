use crate::{errors::Error, model::{Protocol, TrafficRecord}, service_discovery::SERVICE_REGISTRY};
use bytes::Bytes;
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use hyper_util::{client::legacy::{Builder, Client}, rt::TokioExecutor};
use redis::{aio::ConnectionManager, AsyncCommands};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{error, info, warn};

const CHECKPOINT_KEY_PREFIX: &str = "qproxy:checkpoint:";
const SYNC_LOCK_PREFIX: &str = "qproxy:sync_lock:";
const TRAFFIC_RECORDS_PREFIX: &str = "qproxy:traffic:";
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
    // records: Arc<RwLock<HashMap<String, TrafficRecord>>>,
    /// Tokio 执行器，用于异步任务调度
    executor: Arc<TokioExecutor>,
    /// Redis 连接管理器
    redis_pool: Arc<ConnectionManager>,
}

impl PlaybackService {
    /// 使用已有的Redis连接创建服务实例
    pub async fn new(redis_pool: ConnectionManager) -> Result<Self, Error> {
        Ok(Self {
            executor: Arc::new(TokioExecutor::new()),
            redis_pool: Arc::new(redis_pool),
        })
    }

    /// 使用Redis URL创建服务实例
    pub async fn new_with_url(redis_url: &str) -> Result<Self, Error> {
        let client = redis::Client::open(redis_url)?;
        let redis_pool = ConnectionManager::new(client).await?;

        Ok(Self {
            // records: Arc::new(RwLock::new(HashMap::new())),
            executor: Arc::new(TokioExecutor::new()),
            redis_pool: Arc::new(redis_pool),
        })
    }

    /// 获取流量记录的Redis key
    fn get_traffic_key(&self, peer_id: &str, shard_id: &str) -> String {
        format!("{}{}:{}", TRAFFIC_RECORDS_PREFIX, peer_id, shard_id)
    }

    /// 获取checkpoint的Redis key
    fn get_checkpoint_key(&self, peer_id: &str, shard_id: &str) -> String {
        format!("{}{}:{}", CHECKPOINT_KEY_PREFIX, peer_id, shard_id)
    }

    /// 获取同步锁的Redis key
    fn get_lock_key(&self, peer_id: &str, shard_id: &str) -> String {
        format!("{}{}:{}", SYNC_LOCK_PREFIX, peer_id, shard_id)
    }

    /// 获取同步锁
    pub async fn acquire_sync_lock(&self, peer_id: &str, shard_id: &str) -> Result<bool, Error> {
        let key = self.get_lock_key(peer_id, shard_id);
        let mut conn = self.redis_pool.as_ref().clone();
        let acquired: bool = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(LOCK_EXPIRE_SECS)
            .query_async(&mut conn)
            .await?;
        Ok(acquired)
    }

    /// 释放同步锁
    pub async fn release_sync_lock(&self, peer_id: &str, shard_id: &str) -> Result<(), Error> {
        let key = self.get_lock_key(peer_id, shard_id);
        let mut conn = self.redis_pool.as_ref().clone();
        redis::cmd("DEL")
            .arg(&key)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// 更新同步位点
    pub async fn update_checkpoint(&self, checkpoint: &CheckpointInfo) -> Result<(), Error> {
        let key = self.get_checkpoint_key(&checkpoint.peer_id, &checkpoint.shard_id);
        let json = serde_json::to_string(&checkpoint).unwrap();
        let mut conn = self.redis_pool.as_ref().clone();
        conn.set(&key, json).await?;
        Ok(())
    }

    /// 获取同步位点
    pub async fn get_checkpoint(&self, peer_id: &str, shard_id: &str) -> Result<Option<CheckpointInfo>, Error> {
        let key = self.get_checkpoint_key(peer_id, shard_id);
        // 拉取同步记录
        let mut conn = self.redis_pool.as_ref().clone();
        let json: Option<String> = conn.get(&key).await?;
        
        match json {
            Some(j) => {
                match serde_json::from_str::<CheckpointInfo>(&j) {
                    Ok(checkpoint) => {
                        if checkpoint.status == SyncStatus::Pending {
                            Ok(Some(checkpoint))
                        } else {
                            // 如果同步状态为InProgress，说明有节点正在同步数据，则跳过
                            Ok(None)
                        }
                    },
                    Err(e) => {
                        error!("get checkpoint failed: {:?}", e);
                        Err(Error::Json(e))
                    }
                }
            },
            None => Ok(None)
        }
    }

    /// 开始同步流程
    pub async fn start_sync(&self, peer_id: &str, shard_id: &str) -> Result<bool, Error> {
        // 尝试获取锁
        if !self.acquire_sync_lock(peer_id, shard_id).await? {
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
        self.update_checkpoint(&checkpoint).await?;
        info!("acquire sync lock: {}_{}", peer_id, shard_id);
        Ok(true)
    }

    /// 完成同步流程
    pub async fn complete_sync(&self, peer_id: &str, shard_id: &str, success: bool, error_msg: Option<String>) -> Result<(), Error> {
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
        self.update_checkpoint(&checkpoint).await?;

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
    pub async fn get_records_for_sync(&self, peer_id: &str, shard_id: &str) -> Result<Vec<TrafficRecord>, Error> {
        let checkpoint = self.get_checkpoint(peer_id, shard_id).await?;
        let key = self.get_traffic_key(peer_id, shard_id);
        
        let since = checkpoint.as_ref()
            .map(|cp| UNIX_EPOCH + Duration::from_secs(cp.last_sync_time))
            .unwrap_or_else(|| SystemTime::now() - Duration::from_secs(3600));
        
        let min_score = since.duration_since(UNIX_EPOCH)?.as_secs() as f64;
        
        let mut conn = self.redis_pool.as_ref().clone();
        // 获取需要同步的记录，todo 减少时间范围
        let records: Vec<String> = conn.zrangebyscore(&key, min_score, "+inf").await?;
        
        let mut result = Vec::new();
        for json in records {
            if let Ok(record) = serde_json::from_str::<TrafficRecord>(&json) {
                if let Some(cp) = &checkpoint {
                    if record.timestamp >= since && record.id != cp.last_record_id {
                        result.push(record);
                    }
                }
            }
        }
        
        // 按时间戳排序
        result.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(result)
    }

    /// 添加新的流量记录
    /// 
    /// # 参数
    /// * `record` - 要添加的流量记录
    pub async fn add_record(&self, record: TrafficRecord) -> Result<(), Error> {
        let key = self.get_traffic_key(&record.peer_id, "default");
        let json = serde_json::to_string(&record)?;
        let score = record.timestamp.duration_since(UNIX_EPOCH)?.as_secs() as f64;
        
        let mut conn = self.redis_pool.as_ref().clone();
        conn.zadd(&key, &json, score).await?;
        
        // 触发回放
        self.trigger_replay(&record).await;
        Ok(())
    }

    /// 清除所有流量记录
    pub async fn clear_records(&self, peer_id: &str, shard_id: &str) -> Result<(), Error> {
        let key = self.get_traffic_key(peer_id, shard_id);
        let mut conn = self.redis_pool.as_ref().clone();
        conn.del(&key).await?;
        Ok(())
    }

    /// 查找匹配的流量记录
    /// 
    /// # 参数
    /// * `req` - HTTP 请求
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    /// 
    /// # 返回值
    /// 返回匹配的流量记录（如果找到）
    pub async fn find_matching_record<B>(&self, req: &Request<B>, peer_id: &str, shard_id: &str) -> Result<Option<TrafficRecord>, Error>
    where
        B: Body,
    {
        let key = self.get_traffic_key(peer_id, shard_id);
        let mut conn = self.redis_pool.as_ref().clone();
        let records: Vec<String> = conn.zrange(&key, 0, -1).await?;
        
        for json in records {
            if let Ok(record) = serde_json::from_str::<TrafficRecord>(&json) {
                if matches!(record.protocol, Protocol::HTTP | Protocol::HTTPS)
                    && record.request.method.as_ref() == Some(&req.method().to_string())
                    && record.request.service_name.as_ref() == Some(&req.uri().to_string())
                {
                    return Ok(Some(record));
                }
            }
        }
        Ok(None)
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
        let (parts, body) = req.into_parts();
        let body_bytes = match body.collect().await {
            Ok(body) => body.to_bytes(),
            Err(_) => {
                error!("Failed to collect request body");
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from("Failed to collect request body")))
                    .unwrap();
            }
        };
        let record: TrafficRecord = match serde_json::from_slice(&body_bytes) {
            Ok(record) => record,
            Err(e) => {
                error!("Failed to parse request body: {}", e);
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from("Failed to parse request body")))
                    .unwrap();
            },
        };
        let req = Request::from_parts(parts, Full::new(body_bytes));
        if let Ok(Some(record)) = self.find_matching_record(&req, &record.peer_id, "default").await {
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
                                // TCP 协议回放 TODO: 增加鉴权流程
                                // 维护连接池，复用连接
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
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    /// * `since` - 起始时间点，如果为 None 则默认获取最近一小时的记录
    /// 
    /// # 返回值
    /// 返回指定时间段内的流量记录列表
    pub async fn get_recent_records(&self, peer_id: &str, shard_id: &str, since: Option<SystemTime>) -> Result<Vec<TrafficRecord>, Error> {
        let key = self.get_traffic_key(peer_id, shard_id);
        let since = since.unwrap_or_else(|| {
            SystemTime::now() - Duration::from_secs(3600) // 默认获取最近一小时的记录
        });
        let min_score = since.duration_since(UNIX_EPOCH)?.as_secs() as f64;
        
        let mut conn = self.redis_pool.as_ref().clone();
        let records: Vec<String> = conn.zrangebyscore(&key, min_score, "+inf").await?;
        
        let mut result = Vec::new();
        for json in records {
            if let Ok(record) = serde_json::from_str::<TrafficRecord>(&json) {
                result.push(record);
            }
        }
        Ok(result)
    }

    /// 获取所有流量记录
    /// 
    /// # 参数
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    /// 
    /// # 返回值
    /// 返回所有存储的流量记录列表
    pub async fn get_all_records(&self, peer_id: &str, shard_id: &str) -> Result<Vec<TrafficRecord>, Error> {
        let key = self.get_traffic_key(peer_id, shard_id);
        let mut conn = self.redis_pool.as_ref().clone();
        let records: Vec<String> = conn.zrange(&key, 0, -1).await?;
        
        let mut result = Vec::new();
        for json in records {
            if let Ok(record) = serde_json::from_str::<TrafficRecord>(&json) {
                result.push(record);
            }
        }
        Ok(result)
    }

    /// 同步来自peer的记录
    /// 
    /// # 参数
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    /// * `records` - 要同步的记录列表
    pub async fn sync_from_peer(&self, peer_id: &str, shard_id: &str, records: Vec<TrafficRecord>) -> Result<(), Error> {
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
                self.add_record(record.clone()).await?;
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
            self.update_checkpoint(&checkpoint).await?;

            Ok::<(), Error>(())
        }.await;

        match result {
            Ok(()) => {
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
    pub async fn get_all_checkpoints(&self) -> Result<Vec<CheckpointInfo>, Error> {
        let mut conn = self.redis_pool.as_ref().clone();
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
                            // TODO 这里完成 MQTT connect和流量回放
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

                            // TODO 确认录制流量是否带有鉴权信息
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
    use crate::{model::{RequestData, ResponseData}, service_discovery::ServiceInstance};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_record_storage_and_retrieval() {
        // 创建Redis客户端
        let client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis_pool = redis::aio::ConnectionManager::new(client).await.unwrap();

        let service = PlaybackService::new(redis_pool).await.unwrap();
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

        service.add_record(record.clone()).await.unwrap();
        
        let req = Request::builder()
            .method("GET")
            .uri("http://test-service/api/test")
            .body(Full::new(Bytes::from("")))
            .unwrap();

        let found_record = service.find_matching_record(&req, &record.peer_id, "default").await.unwrap().unwrap();
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

        // 创建Redis客户端
        let client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis_pool = redis::aio::ConnectionManager::new(client).await.unwrap();

        let service = PlaybackService::new(redis_pool).await.unwrap();
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

        service.add_record(record.clone()).await.unwrap();

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
        // 创建Redis客户端
        let client = redis::Client::open("redis://localhost:6379").unwrap();
        let redis_pool = redis::aio::ConnectionManager::new(client).await.unwrap();

        let service = PlaybackService::new(redis_pool).await.unwrap();
        let now = SystemTime::now();
        let old_time = now - Duration::from_secs(7200); // 2小时前

        // 添加一个旧记录
        let old_record = TrafficRecord {
            id: "old".to_string(),
            peer_id: "test".to_string(),
            protocol: Protocol::HTTP,
            codec: None,
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
            peer_id: "test".to_string(),
            protocol: Protocol::HTTP,
            codec: None,
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

        service.add_record(old_record.clone()).await.unwrap();
        service.add_record(new_record.clone()).await.unwrap();

        // 获取最近1小时的记录
        let recent_records = service.get_recent_records("test", "default", Some(now - Duration::from_secs(3600))).await.unwrap();
        assert_eq!(recent_records.len(), 1);
        assert_eq!(recent_records[0].id, "new");
    }
} 