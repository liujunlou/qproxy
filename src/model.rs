use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{errors::Error, mqtt_client::codec::MessageType};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrafficRecord {
    pub id: String,
    pub peer_id: String,
    pub protocol: Protocol,
    pub codec: Option<MessageType>,
    pub timestamp: u128,
    pub request: RequestData,
    pub response: ResponseData,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone)]
pub enum Protocol {
    HTTP,
    HTTPS,
    TCP,
    GRPC,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestData {
    pub method: Option<String>,       // HTTP/HTTPS only
    pub service_name: Option<String>, // 请求的服务名
    pub params: Option<Vec<(String, String)>>,
    pub headers: Option<Vec<(String, String)>>, // HTTP/HTTPS only
    pub body: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseData {
    pub status: Option<u16>,                    // HTTP/HTTPS only
    pub headers: Option<Vec<(String, String)>>, // HTTP/HTTPS only
    pub body: Vec<u8>,
}

impl TrafficRecord {
    pub fn new_http(
        method: String,
        service_name: String,
        params: Option<Vec<(String, String)>>,
        request_headers: Vec<(String, String)>,
        request_body: Vec<u8>,
        status: u16,
        response_headers: Vec<(String, String)>,
        response_body: Vec<u8>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            peer_id: service_name.clone(),
            protocol: Protocol::HTTP,
            codec: None,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            request: RequestData {
                method: Some(method),
                service_name: Some(service_name.clone()),
                params: params,
                headers: Some(request_headers),
                body: request_body,
            },
            response: ResponseData {
                status: Some(status),
                headers: Some(response_headers),
                body: response_body,
            },
        }
    }

    pub fn new_tcp(service_name: &str, request_data: Vec<u8>, response_data: Vec<u8>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            peer_id: service_name.to_string(),
            protocol: Protocol::TCP,
            // TODO 需要根据具体的消息类型来确定
            codec: Some(MessageType::Publish),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            request: RequestData {
                method: None,
                service_name: Some(service_name.to_string()),
                params: None,
                headers: None,
                body: request_data,
            },
            response: ResponseData {
                status: None,
                headers: None,
                body: response_data,
            },
        }
    }

    pub fn new_grpc(service_name: &str, request_data: Vec<u8>, response_data: Vec<u8>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            peer_id: service_name.to_string(),
            protocol: Protocol::GRPC,
            codec: None,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            request: RequestData {
                method: None,
                service_name: Some(service_name.to_string()),
                params: None,
                headers: None,
                body: request_data,
            },
            response: ResponseData {
                status: None,
                headers: None,
                body: response_data,
            },
        }
    }

    // 过滤字段
    pub fn filter_fields(&self, fields: Vec<String>) -> Result<Self, Error> {
        let mut filtered = self.clone();
        // 在TCP的情况下，解析拉取的body为具体的消息对象，并过滤其中的字段
        // if self.protocol == Protocol::TCP {
        //     // 解析拉取的body为具体的消息对象
        //     let request_body = MqttCodec::decode(&filtered.request.body)?;
        //     // TODO 反序列化为具体的消息对象

        //     // 过滤其中的字段
        //     filtered.request.body = fields.iter().map(|field| {
        //         filtered.request.body.iter().filter(|item| {
        //             item.contains(field)
        //         }).collect()
        //     }).collect();
        // }

        Ok(filtered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Offset {
    pub peer_id: String,
    pub shard_id: String,
    pub offset: u128,
    pub record_id: String,
}

impl Offset {
    pub fn new(peer_id: String, shard_id: String, offset: u128, record_id: String) -> Self {
        Self { peer_id, shard_id, offset, record_id }
    }
}
