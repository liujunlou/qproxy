use crate::{model::{Protocol, TrafficRecord}, service_discovery::SERVICE_REGISTRY};
use bytes::Bytes;
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use hyper_util::{client::legacy::{Builder, Client}, rt::TokioExecutor};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{SystemTime, Duration};
use tracing::{info, warn};
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
pub struct PlaybackService {
    records: Arc<RwLock<HashMap<String, TrafficRecord>>>,
    executor: Arc<TokioExecutor>,
}

impl PlaybackService {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            executor: Arc::new(TokioExecutor::new()),
        }
    }

    pub async fn add_record(&self, record: TrafficRecord) {
        let mut records = self.records.write().await;
        records.insert(record.id.clone(), record);
    }

    pub async fn clear_records(&self) {
        let mut records = self.records.write().await;
        records.clear();
    }

    pub async fn find_matching_record<B>(&self, req: &Request<B>) -> Option<TrafficRecord>
    where
        B: Body,
    {
        let records = self.records.read().await;
        
        records.values()
            .find(|record| {
                matches!(record.protocol, Protocol::HTTP | Protocol::HTTPS)
                    && record.request.method.as_ref() == Some(&req.method().to_string())
                    && record.request.uri.as_ref() == Some(&req.uri().to_string())
            })
            .cloned()
    }

    pub async fn playback<B>(&self, req: Request<B>) -> Response<Full<Bytes>>
    where
        B: Body,
    {
        if let Some(record) = self.find_matching_record(&req).await {
            // 尝试查找本地服务
            if let Some(original_url) = record.request.uri.as_ref() {
                match SERVICE_REGISTRY.find_local_service(original_url).await {
                    Ok(Some(service)) => {
                        // 构建本地服务连接
                        let addr = format!("{}:{}", service.host, service.port);
                        info!("Replaying traffic to local service: {}", addr);

                        match TcpStream::connect(&addr).await {
                            Ok(mut stream) => {
                                // 发送请求数据
                                if let Err(e) = stream.write_all(&record.request.body).await {
                                    warn!("Failed to write request to local service: {}", e);
                                } else {
                                    // 读取响应数据
                                    let mut response_data = Vec::new();
                                    let mut buffer = [0u8; 8192];
                                    
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

                                    if !response_data.is_empty() {
                                        info!("Successfully replayed traffic to local service");
                                        return Response::new(Full::new(Bytes::from(response_data)));
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to replay to local service: {}, falling back to recorded response", e);
                            }
                        }
                    }
                    Ok(None) => {
                        warn!("No local service found for {}, using recorded response", original_url);
                    }
                    Err(e) => {
                        warn!("Error finding local service: {}, using recorded response", e);
                    }
                }
            }

            // 如果本地服务不可用或出错，返回记录的响应
            let mut response = Response::builder()
                .status(record.response.status.unwrap_or(200));

            if let Some(headers) = record.response.headers {
                for (name, value) in headers {
                    response = response.header(name, value);
                }
            }

            response.body(Full::new(Bytes::from(record.response.body)))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::new(Bytes::from("Failed to construct response")))
                        .unwrap()
                })
        } else {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("No matching record found")))
                .unwrap()
        }
    }

    // 获取最近的记录
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

    // 获取所有记录
    pub async fn get_all_records(&self) -> Vec<TrafficRecord> {
        let records = self.records.read().await;
        records.values().cloned().collect()
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
} 