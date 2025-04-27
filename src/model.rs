use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrafficRecord {
    pub id: String,
    pub protocol: Protocol,
    pub timestamp: SystemTime,
    pub request: RequestData,
    pub response: ResponseData,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone)]
pub enum Protocol {
    HTTP,
    HTTPS,
    TCP,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestData {
    pub method: Option<String>,  // HTTP/HTTPS only
    pub uri: Option<String>,     // HTTP/HTTPS only
    pub headers: Option<Vec<(String, String)>>,  // HTTP/HTTPS only
    pub body: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseData {
    pub status: Option<u16>,     // HTTP/HTTPS only
    pub headers: Option<Vec<(String, String)>>,  // HTTP/HTTPS only
    pub body: Vec<u8>,
}

impl TrafficRecord {
    pub fn new_http(
        method: String,
        uri: String,
        request_headers: Vec<(String, String)>,
        request_body: Vec<u8>,
        status: u16,
        response_headers: Vec<(String, String)>,
        response_body: Vec<u8>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            protocol: Protocol::HTTP,
            timestamp: SystemTime::now(),
            request: RequestData {
                method: Some(method),
                uri: Some(uri),
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

    pub fn new_tcp(request_data: Vec<u8>, response_data: Vec<u8>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            protocol: Protocol::TCP,
            timestamp: SystemTime::now(),
            request: RequestData {
                method: None,
                uri: None,
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
} 