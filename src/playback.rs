use crate::{
    client::grpc_client::{get_grpc_client, GrpcClient},
    errors::Error,
    get_shutdown_rx,
    model::{Offset, Protocol, TrafficRecord},
    options::SyncOptions,
    SERVICE_REGISTRY,
};
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use hyper_util::{
    client::legacy::{Builder, Client},
    rt::TokioExecutor,
};
use prost::Message;
use redis::{aio::ConnectionManager, AsyncCommands};
use route::RouteMessage;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, sync::Arc};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::RwLock,
};
// 引入生成的 proto 代码
pub mod route {
    include!(concat!(env!("OUT_DIR"), "/route.rs"));
}

use tracing::{error, info, warn};

const CHECKPOINT_KEY_PREFIX: &str = "qproxy:checkpoint:";
const SYNC_LOCK_PREFIX: &str = "qproxy:sync_lock:";
const TRAFFIC_RECORDS_PREFIX: &str = "qproxy:traffic:";
const LOCK_EXPIRE_SECS: u64 = 300; // 5分钟锁过期时间

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub peer_id: String,
    pub shard_id: String,
    pub last_sync_time: u128,
    pub last_record_id: String,
    pub status: SyncStatus,
    pub retry_count: u32,
    pub error_message: Option<String>,
}

impl CheckpointInfo {
    pub fn new(peer_id: &str, shard_id: &str) -> Result<Self, Error> {

        Ok(Self {
            peer_id: peer_id.to_string(),
            shard_id: shard_id.to_string(),
            last_sync_time: Self::get_start_timestamp()?,
            last_record_id: String::new(),
            status: SyncStatus::Pending,
            retry_count: 0,
            error_message: None,
        })
    }
    pub fn new_with_offset(
        peer_id: &str,
        shard_id: &str,
        offset: u128,
        record_id: &str,
    ) -> Result<Self, Error> {
        Ok(Self {
            peer_id: peer_id.to_string(),
            shard_id: shard_id.to_string(),
            last_sync_time: offset,
            last_record_id: record_id.to_string(),
            status: SyncStatus::Completed,
            retry_count: 0,
            error_message: None,
        })
    }
    fn get_start_timestamp() -> Result<u128, Error> {
        let start = SystemTime::now() - Duration::from_secs(60*5);
        Ok(start.duration_since(UNIX_EPOCH)?.as_millis())
    }
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
    /// Tokio 执行器，用于异步任务调度
    executor: Arc<TokioExecutor>,
    /// 存储流量记录的哈希表，使用 RwLock 保证并发安全, 用于回放节点本地缓存
    records: Arc<RwLock<HashMap<String, Vec<TrafficRecord>>>>,
    /// Redis 连接管理器, 用于录制节点的公共缓存
    redis_pool: Arc<ConnectionManager>,
    /// 发送新流量通知的通道, 用于触发回放
    notify_tx: Arc<Mutex<mpsc::Sender<String>>>,
    /// 接收新流量通知的通道, 用于触发回放
    notify_rx: Arc<Mutex<mpsc::Receiver<String>>>,
}

impl PlaybackService {
    /// 使用已有的Redis连接创建服务实例
    pub async fn new(redis_pool: ConnectionManager) -> Result<Self, Error> {
        let (tx, rx) = mpsc::channel(100);
        Ok(Self {
            executor: Arc::new(TokioExecutor::new()),
            records: Arc::new(RwLock::new(HashMap::new())),
            redis_pool: Arc::new(redis_pool),
            notify_tx: Arc::new(Mutex::new(tx)),
            notify_rx: Arc::new(Mutex::new(rx)),
        })
    }

    /// 使用Redis URL创建服务实例
    pub async fn new_with_url(redis_url: &str) -> Result<Self, Error> {
        let client = redis::Client::open(redis_url)?;
        let redis_pool = ConnectionManager::new(client).await?;
        let (tx, rx) = mpsc::channel(100);
        Ok(Self {
            executor: Arc::new(TokioExecutor::new()),
            records: Arc::new(RwLock::new(HashMap::new())),
            redis_pool: Arc::new(redis_pool),
            notify_tx: Arc::new(Mutex::new(tx)),
            notify_rx: Arc::new(Mutex::new(rx)),
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
        redis::cmd("DEL").arg(&key).query_async(&mut conn).await?;
        Ok(())
    }

    /// 更新同步位点
    pub async fn update_checkpoint(&self, checkpoint: &CheckpointInfo) -> Result<(), Error> {
        // 因为现在所有的录制数据都在default节点，所以这里直接用default节点的key
        let key = self.get_checkpoint_key("default", "default");
        // let key = self.get_checkpoint_key(&checkpoint.peer_id, &checkpoint.shard_id);
        let old: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut self.redis_pool.as_ref().clone())
            .await?;

        let mut new_checkpoint = checkpoint.clone();
        if let Some(old) = old {
            info!("Get old offset {} by {}", old, key);
            info!("Get new checkpoint {:?}", checkpoint);

            let old_checkpoint: CheckpointInfo = serde_json::from_str(&old)?;
            // 如果旧值大于新值，则直接跳过
            if old_checkpoint.last_sync_time > checkpoint.last_sync_time {
                info!("skip update offset");
                return Ok(());
            }
        } else {
            // 如果旧的checkpoint不存在，则直接更新
            info!(
                "derict update checkpoint: {}_{}",
                checkpoint.peer_id, checkpoint.shard_id
            );
        };

        let json = serde_json::to_string(&new_checkpoint).unwrap();
        info!("Update offset {} by {}", json, key);

        let mut conn = self.redis_pool.as_ref().clone();
        conn.set(&key, json).await?;
        Ok(())
    }

    /// 获取同步位点
    pub async fn get_checkpoint(
        &self,
        peer_id: &str,
        shard_id: &str,
    ) -> Result<Option<CheckpointInfo>, Error> {
        let key = self.get_checkpoint_key(peer_id, shard_id);
        // 拉取同步记录
        let mut conn = self.redis_pool.as_ref().clone();
        let json: Option<String> = match conn.get(&key).await {
            Ok(json) => json,
            Err(e) => {
                error!("get checkpoint failed: {:?}", e);
                return Err(Error::Redis(e));
            }
        };

        match json {
            Some(j) => {
                match serde_json::from_str::<CheckpointInfo>(&j) {
                    Ok(checkpoint) => {
                        if checkpoint.status == SyncStatus::Pending || checkpoint.status == SyncStatus::Completed {
                            Ok(Some(checkpoint))
                        } else {
                            // 如果同步状态为InProgress，说明有节点正在同步数据，则跳过
                            Ok(None)
                        }
                    }
                    Err(e) => {
                        error!("get checkpoint failed: {:?}", e);
                        Err(Error::Json(e))
                    }
                }
            }
            None => Ok(None),
        }
    }

    /// 开始同步流程
    pub async fn start_sync(&self, peer_id: &str, shard_id: &str) -> Result<bool, Error> {
        // 尝试获取锁
        if !self.acquire_sync_lock(peer_id, shard_id).await? {
            return Ok(false);
        }

        // 获取或创建checkpoint
        let mut checkpoint = self
            .get_checkpoint(peer_id, shard_id)
            .await?
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
    pub async fn complete_sync(
        &self,
        peer_id: &str,
        shard_id: &str,
        success: bool,
        error_msg: Option<String>,
    ) -> Result<(), Error> {
        let mut checkpoint = match self.get_checkpoint(peer_id, shard_id).await? {
            Some(cp) => cp,
            None => {
                error!(
                    "No checkpoint found for peer: {}, shard: {}",
                    peer_id, shard_id
                );
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

    /// 获取需要同步的记录，从checkpoint开始（如果checkpoint不存在，则从0开始），到之后的5分钟的记录
    ///
    /// # 参数
    /// * `peer_id` - 对等节点标识
    /// * `shard_id` - 分片标识
    ///
    /// # 返回值
    /// 返回需要同步的记录列表
    pub async fn get_records_for_sync(
        &self,
        peer_id: &str,
        shard_id: &str,
    ) -> Result<Vec<TrafficRecord>, Error> {
        let cp = if let Some(cp) = self.get_checkpoint(peer_id, shard_id).await? {
            cp
        } else {
            // 初始化checkpoint, 并更新到redis, 防止初始化期间没有checkpoint
            let mut cp = CheckpointInfo::new(peer_id, shard_id)?;
            // 设置状态为进行中，防止分布式环境下并行同步任务
            cp.status = SyncStatus::InProgress;
            cp
        };

        info!("Get records for sync, peer_id: {}, shard_id: {}, last_sync_time: {}", peer_id, shard_id, cp.last_sync_time);
        let key = self.get_traffic_key(peer_id, shard_id);
        // 从checkpoint的 last_sync_time 开始同步，记录的 last_sync_time 转时间
        let since = cp.last_sync_time as u64;
        let mut min_score = cp.last_sync_time as u64;
        let mut step = 300 * 1000;

        let now = Instant::now();

        let mut records: Vec<String> = Vec::new();
        let end_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let max_records = 1000;
        let max_duration = Duration::from_secs(5);

        let mut max_score = since + step;
        let mut conn = self.redis_pool.as_ref().clone();
        loop {
            info!("Get records from redis, key: {}, min_score: {}, max_score: {}, {}", key, min_score, max_score, max_records);
            // 获取指定范围内的记录，并限制数量，避免数据量太大
            match conn
                .zrangebyscore_limit::<_, _, _, Vec<String>>(
                    &key,
                    min_score,
                    max_score,
                    0,
                    max_records as isize,
                )
                .await
            {
                Ok(tmp) => {
                    records.extend(tmp.into_iter().map(|x| x.to_string()));
                    // 如果已经达到最大记录数，立即退出
                    if records.len() >= max_records {
                        info!("Reached maximum record limit of {}", max_records);
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to get records: {:?}", e);
                    break;
                }
            }

            // 检查是否超时
            if now.elapsed() >= max_duration {
                info!("Sync operation timed out after {:?}", max_duration);
                break;
            }

            // 检查是否已经到达当前时间
            let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
            if max_score >= current_time {
                info!("Reached current time boundary");
                break;
            }

            // 更新min_score和max_score，获取下一步长的回放记录
            min_score = max_score;
            max_score += step;
        }
    
        // 待优化，这里提交的最后的一个位点在redis的zrangebyscore指令是大于等于，会导致重复拉取该条记录
        // 所以这里移除已提交的最后一条记录
        info!("Get record size {}", records.clone().len());
        let mut result = Vec::new();
        for json in records {
            if let Ok(record) = serde_json::from_str::<TrafficRecord>(&json) {
                if record.timestamp >= since as u128
                    && record.id != cp.last_record_id
                {
                    result.push(record);
                }
            }
        }

        info!("Convert to result size {}", result.clone().len());
        // 按时间戳排序
        result.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(result)
    }

    /// 添加新的流量记录到Redis缓存队列
    ///
    /// # 参数
    /// * `record` - 要添加的流量记录
    pub async fn add_record(&self, record: TrafficRecord) -> Result<(), Error> {
        // 暂时不做录制记录区分，来保证数据在一个队列上
        let key = self.get_traffic_key("default", "default");
        let json = serde_json::to_string(&record)?;
        let score = record.timestamp as u64;

        let mut conn = self.redis_pool.as_ref().clone();
        info!("Add record to redis, key: {}, score: {}, json: {}", key, score, json);
        conn.zadd(&key, &json, score).await?;
        Ok(())
    }

    /// 清除所有流量记录
    pub async fn clear_records(&self, peer_id: &str, shard_id: &str) -> Result<(), Error> {
        let key = self.get_traffic_key(peer_id, shard_id);
        let mut conn = self.redis_pool.as_ref().clone();
        conn.del(&key).await?;
        Ok(())
    }

    pub async fn add_local_record(&self, record: &TrafficRecord) -> Result<(), Error> {
        // 将records 根据 peer_id 进行分流，来分别推入不同的 self.records key中
        let tx_key = record.peer_id.clone();
        let key = self.get_traffic_key("default", "default");
        self.records
            .write()
            .await
            .entry(key.clone())
            .or_insert(Vec::new())
            .push(record.clone());
        info!("Add local record, peer_id: {}, record: {:?}", record.peer_id, record);
        let tx: tokio::sync::MutexGuard<'_, mpsc::Sender<String>> = self.notify_tx.lock().await;
        tx.send(tx_key).await.map_err(|e| Error::Playback(e.to_string()))?;
        Ok(())
    }

    /// 清除本地流量记录
    pub async fn clear_local_records(&self, record: TrafficRecord) -> Result<(), Error> {
        let key = self.get_traffic_key(&record.peer_id, "default");
        if let Some(records) = self.records.write().await.get_mut(&key) {
            records.retain(|r| r.id != record.id);
        }
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
    pub async fn find_matching_record<B>(
        &self,
        req: &Request<B>,
        peer_id: &str,
        shard_id: &str,
    ) -> Result<Option<TrafficRecord>, Error>
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
            }
        };
        let req = Request::from_parts(parts, Full::new(body_bytes));
        if let Ok(Some(record)) = self
            .find_matching_record(&req, &record.peer_id, "default")
            .await
        {
            // 尝试查找本地服务
            if let Some(service_name) = record.request.service_name.as_ref() {
                match SERVICE_REGISTRY
                    .read()
                    .await
                    .find_local_service(service_name)
                    .await
                {
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
                                        if let Err(e) = stream.write_all(&record.request.body).await
                                        {
                                            warn!(
                                                "Failed to write request to local service: {}",
                                                e
                                            );
                                        } else {
                                            // 读取本地服务的响应数据
                                            let mut response_data = Vec::new();
                                            let mut buffer = [0u8; 8192];

                                            // 循环读取响应数据直到 EOF 或错误发生
                                            loop {
                                                match stream.read(&mut buffer).await {
                                                    Ok(0) => break, // EOF
                                                    Ok(n) => response_data
                                                        .extend_from_slice(&buffer[..n]),
                                                    Err(e) => {
                                                        warn!("Failed to read response from local service: {}", e);
                                                        break;
                                                    }
                                                }
                                            }

                                            // 如果成功读取到响应数据，则返回
                                            if !response_data.is_empty() {
                                                info!("Successfully replayed TCP traffic to local service");
                                                return Response::new(Full::new(Bytes::from(
                                                    response_data,
                                                )));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to establish TCP connection: {}, falling back to recorded response", e);
                                    }
                                }
                            }
                            Protocol::GRPC => {
                                // GRPC 协议回放
                                let mut client = match GrpcClient::new(&addr).await {
                                    Ok(client) => client,
                                    Err(e) => {
                                        warn!("Failed to create GRPC client: {}", e);
                                        return Response::builder()
                                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                                            .body(Full::new(Bytes::from(
                                                "Failed to create GRPC client",
                                            )))
                                            .unwrap();
                                    }
                                };
                                // 反序列化RouteMessage
                                let route_message =
                                    match prost::Message::decode(&mut &record.request.body[..]) {
                                        Ok(route_message) => route_message,
                                        Err(e) => {
                                            warn!("Failed to decode RouteMessage: {}", e);
                                            return Response::builder()
                                                .status(StatusCode::BAD_REQUEST)
                                                .body(Full::new(Bytes::from(
                                                    "Failed to decode RouteMessage",
                                                )))
                                                .unwrap();
                                        }
                                    };
                                let request = tonic::Request::new(route_message);
                                // 通过grpc回放流量，并返回响应
                                match client.call(request).await {
                                    Ok(response) => {
                                        if let Some(code) = response.get_ref().status_code {
                                            if code == 200 {
                                                let payload = response
                                                    .get_ref()
                                                    .payload
                                                    .clone()
                                                    .unwrap_or(Vec::new());
                                                return Response::new(Full::new(Bytes::from(
                                                    payload,
                                                )));
                                            }
                                        }
                                        return Response::builder()
                                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                                            .body(Full::new(Bytes::from(
                                                "Failed to send GRPC request",
                                            )))
                                            .unwrap();
                                    }
                                    Err(e) => {
                                        warn!("Failed to send GRPC request: {}", e);
                                        return Response::builder()
                                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                                            .body(Full::new(Bytes::from(
                                                "Failed to send GRPC request",
                                            )))
                                            .unwrap();
                                    }
                                };
                            }
                            Protocol::HTTP | Protocol::HTTPS => {
                                // HTTP/HTTPS 协议回放
                                let scheme = if record.protocol == Protocol::HTTPS {
                                    "https"
                                } else {
                                    "http"
                                };
                                let local_url = format!(
                                    "{}://{}:{}{}",
                                    scheme,
                                    service.host,
                                    service.port,
                                    req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("")
                                );

                                info!(
                                    "Replaying HTTP/HTTPS traffic to local service: {}",
                                    local_url
                                );

                                // 构建并发送 HTTP 请求
                                let local_req =
                                    Request::builder().method(req.method()).uri(local_url);

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
                                match local_req
                                    .body(Full::new(Bytes::from(record.request.body.clone())))
                                {
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
                        warn!(
                            "No local service found for {}, using recorded response",
                            service_name
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Error finding local service: {}, using recorded response",
                            e
                        );
                    }
                }
            }

            // 如果本地服务不可用或出错，返回记录的响应
            let mut response = Response::builder().status(record.response.status.unwrap_or(200));

            // 添加响应头
            if let Some(headers) = record.response.headers {
                for (name, value) in headers {
                    response = response.header(name, value);
                }
            }

            // 构建并返回响应
            response
                .body(Full::new(Bytes::from(record.response.body)))
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
    pub async fn get_recent_records(
        &self,
        peer_id: &str,
        shard_id: &str,
        since: Option<u128>,
    ) -> Result<Vec<TrafficRecord>, Error> {
        let key = self.get_traffic_key(peer_id, shard_id);
        let since = since.unwrap_or_else(|| {
            SystemTime::now()
                .checked_sub(Duration::from_secs(3600))
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() // 默认获取最近一小时的记录
        });
        let min_score = since.clone() as u64;

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
    pub async fn get_all_records(
        &self,
        peer_id: &str,
        shard_id: &str,
    ) -> Result<Vec<TrafficRecord>, Error> {
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
    /// * `record` - 要同步的记录
    pub async fn sync_from_peer(&self, record: &mut TrafficRecord) -> Result<Vec<u8>, Error> {
        let responses = self.trigger_replay(record).await?;
        Ok(responses)
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
    pub async fn trigger_replay(&self, record: &mut TrafficRecord) -> Result<Vec<u8>, Error> {
        if let Some(service_name) = record.request.service_name.as_ref() {
            match SERVICE_REGISTRY
                .read()
                .await
                .find_local_service(service_name)
                .await
            {
                Ok(Some(service)) => {
                    // 构建本地服务连接地址
                    let addr = format!("{}:{}", service.host, service.port);
                    info!("Triggering traffic replay to local service: {} ,record: {:?}", addr, record.clone());

                    match record.protocol {
                        Protocol::TCP => {
                            // TODO 这里完成 MQTT connect和流量回放，复用连接
                            if let Ok(mut stream) = TcpStream::connect(&addr).await {
                                if let Err(e) = stream.write_all(&record.request.body).await {
                                    warn!("Failed to replay TCP traffic: {}", e);
                                    return Err(Error::ServiceError(
                                        "Failed to replay TCP traffic".to_string(),
                                    ));
                                } else {
                                    info!("Successfully replayed TCP traffic to {}", addr);
                                    // TODO 这里需要返回响应数据
                                    return Ok(vec![]);
                                }
                            } else {
                                warn!("Failed to connect to {}", addr);
                                return Err(Error::ServiceError(
                                    "Failed to connect to".to_string(),
                                ));
                            }
                        }
                        Protocol::GRPC => {
                            info!("Triggering traffic replay to local grpc service: {} ,record: {:?}", addr, record.clone());
                            // 这里完成 MQTT connect和流量回放，复用连接
                            let mut client = match get_grpc_client(&addr).await {
                                Ok(c) => c,
                                Err(e) => {
                                    error!("Get grpc client failed: {}", e);
                                    return Err(e);
                                }
                            };
                            let route_message = RouteMessage::decode(&mut &record.request.body[..])
                                .map_err(|_| {
                                    Error::GrpcStatus(format!(
                                        "Failed to decode RouteMessage {:?}",
                                        record.request.body
                                    ))
                                })?;
                            let request = tonic::Request::new(route_message);
                            info!("Request grpc service: {:?}", request);
                            let response = match client.call(request).await {
                                Ok(resp) => resp,
                                Err(e) => {
                                    error!("Failed to call grpc service: {:?}", e);
                                    return Err(Error::GrpcStatus(e.to_string()));
                                }
                            };
                            info!("Get grpc response: {:?}", response);
                            return Ok(response.get_ref().payload.clone().unwrap_or(Vec::new()));
                        }
                        Protocol::HTTP | Protocol::HTTPS => {
                            info!("Triggering traffic replay to local http service: {} ,record: {:?}", addr, record.clone());
                            let scheme = if record.protocol == Protocol::HTTPS {
                                "https"
                            } else {
                                "http"
                            };

                            // 根据回放的 http 请求的 Content-Type 来构建回放请求
                            let mut params: String = record
                                .request
                                .params
                                .as_ref()
                                .map(|u| {
                                    u.iter()
                                        .map(|(k, v)| format!("{}={}", k, v))
                                        .collect::<Vec<String>>()
                                        .join("&")
                                })
                                .unwrap_or_else(|| "".to_string());
                            
                            if let Some(headers) = record.request.headers.clone() {
                                if let Some((_, content_type)) = headers.iter().find(|(k, _)| k.to_lowercase() == "content-type") {
                                    if content_type.contains("application/x-www-form-urlencoded") {
                                        if let Some(params) = record.request.params.clone() {
                                            let body = serde_urlencoded::to_string(params).map_err(|e| {
                                                Error::ServiceError(format!("Failed to encode params: {}", e))
                                            })?;
                                            record.request.body = body.into_bytes();
                                        }
                                        // 在 application/x-www-form-urlencoded 中，直接将 params 置空
                                        params.clear();
                                    }
                                }
                            }

                            let query_suffix = if params.is_empty() { String::new() } else { format!("?{}", params) };
                            let local_url = format!(
                                "{}://{}:{}{}{}",
                                scheme,
                                service.host,
                                service.port,
                                record.request.path.as_deref().unwrap_or(""),
                                query_suffix
                            );
                            info!("Request local url: {}", local_url.clone());

                            let local_req = Request::builder()
                                .method(
                                    record
                                        .request
                                        .method
                                        .as_ref()
                                        .unwrap_or(&"GET".to_string())
                                        .as_str(),
                                )
                                .uri(&local_url);

                            // TODO 确认录制流量是否带有鉴权信息
                            let local_req = if let Some(headers) = &record.request.headers {
                                let mut builder = local_req;
                                for (name, value) in headers {
                                    if name.eq_ignore_ascii_case("content-length") {
                                        continue;
                                    }
                                    builder = builder.header(name, value);
                                }
                                builder
                            } else {
                                local_req
                            };

                            let request = local_req
                                .body(Full::new(Bytes::from(record.request.body.clone())))
                                .map_err(|_| {
                                    warn!("Has no body for HTTP request {}", local_url);
                                    Error::Http1("Has no body".to_string())
                                })?;

                            let client = Builder::new((*self.executor).clone())
                                .pool_idle_timeout(std::time::Duration::from_secs(30))
                                .build_http();
                            
                            match client.request(request).await {
                                Ok(resp) => {
                                    let status = resp.status();
                                    if status.is_success() {
                                        info!("Successfully replayed HTTP traffic to {} with status {}", local_url, status);
                                        return Ok(resp
                                            .into_body()
                                            .collect()
                                            .await?
                                            .to_bytes()
                                            .to_vec());
                                    } else {
                                        warn!("HTTP request failed with status {}: {}", status, local_url);
                                        // 对于HTTP错误状态码，返回错误
                                        return Err(Error::Http1(format!("HTTP request failed with status {}", status)));
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to replay HTTP traffic: {}", e);
                                    return Err(Error::Http1(e.to_string()));
                                }
                            }
                        }
                    }
                }
                Ok(None) => {
                    warn!("No local service found for {}", service_name);
                    return Err(Error::ServiceDiscovery(
                        "No local service found".to_string(),
                    ));
                }
                Err(e) => {
                    warn!("Error finding local service: {}", e);
                    return Err(Error::ServiceDiscovery(e.to_string()));
                }
            }
        } else {
            warn!("No service name found for traffic record");
            return Err(Error::Config("No service name found".to_string()));
        }
    }

    /// 启动流量回放服务
    pub async fn auto_playback(&self, sync: &SyncOptions) {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);
        let mut notify_rx = self.notify_rx.lock().await;

        loop {
            if get_shutdown_rx().await.try_recv().is_ok() {
                info!("Sync service received shutdown signal");
                return;
            }
            tokio::select! {
                // 处理新流量通知
                Some(peer_id) = notify_rx.recv() => {
                    info!("Received new traffic notification for peer: {}", peer_id);
                    let offset = match self.replay_pending_traffic("default", "default").await {
                        Ok(offset) => offset,
                        Err(e) => {
                            error!("Failed to replay traffic for peer {}: {}", peer_id, e);
                            continue;
                        }
                    };
                    // 更新offset
                    if let Some(offset) = offset {
                        if let Err(e) = update_offset(&sync, offset).await {
                            error!("Failed to update offset for peer {}: {}", peer_id, e);
                        }
                    }
                    // 重置回退时间
                    backoff = Duration::from_secs(1);
                }
                // 轮询检查待回放流量
                _ = tokio::time::sleep(backoff) => {
                    info!("Polling for pending traffic with backoff: {:?}", backoff);
                    let offset = match self.replay_pending_traffic("default", "default").await {
                        Ok(offset) => offset,
                        Err(e) => {
                            error!("Failed to replay traffic during polling: {}", e);
                            continue;
                        }
                    };
                    // 更新offset
                    if let Some(offset) = offset {
                        if let Err(e) = update_offset(&sync, offset).await {
                            error!("Failed to update offset for peer {}: {}", "default", e);
                        }
                    }
                    // 指数回退，但不超过最大回退时间
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                }
            }
        }
    }

    /// 回放待处理的流量
    async fn replay_pending_traffic(&self, peer_id: &str, shard_id: &str) -> Result<Option<Offset>, Error> {
        let key = self.get_traffic_key(peer_id, shard_id);

        // 1. 先获取所有需要回放的记录
        let records_to_replay = {
            let records_guard = self.records.read().await;
            records_guard
                .get(&key)
                .map(|records| records.clone())
                .unwrap_or_default()
        };
        let size = records_to_replay.len();
        info!("Starting replay records {}", size);
        if size == 0 {
            return Ok(None)
        }

        // 2. 回放流量
        let mut last_offset = 0;
        let mut last_record_id = String::new();
        let mut failed_records = Vec::new();
        for record in &records_to_replay {
            info!("Replaying traffic record: {}", record.id);
            if let Err(e) = self.trigger_replay(&mut record.clone()).await {
                // 将回放失败的记录写入列表进行重试
                error!("Failed to replay traffic record {}: {}", record.id, e);
                failed_records.push(record.id.clone());
            }
            // 更新offset
            if record.timestamp > last_offset {
                last_offset = record.timestamp.clone();
                last_record_id = record.id.clone();
                info!("Update offset {} record_id {}", last_offset.clone(), last_record_id.clone());
            }
        }

        info!("Lastly offset {} record_id {}", last_offset.clone(), last_record_id.clone());
        // 3. 清理已成功回放的记录
        if !records_to_replay.is_empty() {
            let mut records_guard = self.records.write().await;
            if let Some(records) = records_guard.get_mut(&key) {
                // 清理已成功的回放记录
                records.retain(|r| failed_records.contains(&r.id));
            }
        }

        let remaining = self.records.read().await;
        match remaining.get(&key) {
            Some(records) => {
                info!("End remaining records {}", records.len());
            }
            None => {
                info!("End remaining no records");
            }
        }
        
        // 4. 返回已回放的offset
        let offset = Offset::new(
            peer_id.to_string(),
            shard_id.to_string(),
            last_offset,
            last_record_id,
        );
        Ok(Some(offset))
    }
}

/// 上报offset
async fn update_offset(opts: &SyncOptions, offset: Offset) -> Result<(), Error> {
    if let Some(peer) = opts.peer.as_ref() {
        let client = reqwest::Client::new();
        match client
            .post(format!(
                "http://{}:{}/commit",
                peer.host.clone(),
                peer.port.clone()
            ))
            .json(&offset)
            .send()
            .await
        {
            Ok(res) => {
                if res.status().is_success() {
                    info!(
                        "Successfully updated offset for peer: {}",
                        offset.peer_id.clone()
                    );
                    Ok(())
                } else {
                    error!(
                        "Failed to update offset for peer: {}",
                        offset.peer_id.clone()
                    );
                    Err(Error::Http1("Failed to update offset".to_string()))
                }
            }
            Err(e) => {
                error!(
                    "Failed to update offset for peer: {}, {:?}",
                    offset.peer_id.clone(),
                    e
                );
                Err(Error::Http1("Failed to update offset".to_string()))
            }
        }
    } else {
        error!("No peer found for sync");
        return Err(Error::Config("No peer found for sync".to_string()));
    }
}
