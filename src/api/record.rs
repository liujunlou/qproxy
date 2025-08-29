
use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info};

use crate::{errors::Error, model::{self}, options::{Options, ProxyMode}, playback::PlaybackService, PLAYBACK_SERVICE};

#[derive(Debug, Serialize, Deserialize)]
pub struct Record {
    pub method: String,
    pub service_name: String,
    pub path: Option<String>,
    #[serde(default)] // 如果没有提供，使用默认值
    pub params: Option<Vec<(String, String)>>,
    #[serde(default)] // 如果没有提供，使用默认值
    pub request_headers: Vec<(String, String)>,
    #[serde(rename = "request_body")]
    pub request_body: Option<HashMap<String, Value>>,
}

pub async fn handle_record_request<B>(
    req: Request<B>, 
    options: Arc<Options>
) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    let body = req
        .into_body()
        .collect()
        .await
        .map_err(|_| Error::Http1("Failed to record request body".to_string()))?
        .to_bytes();
    let mut record: Record = serde_json::from_slice(&body)
        .map_err(|e| Error::Proxy(format!("Failed to parse records: {}", e)))?;

    info!("Adding sync record: {:?}", record);

    // 在请求头上加上mode: playback来区分回放流量
    record.request_headers.push(("mode".to_string(), "playback".to_string()));

    let data = serde_json::to_vec(&record.request_body)?;
    let record = model::TrafficRecord::new_http(
        record.method,
        record.service_name,
        record.path,
        record.params,
        record.request_headers,
        data,
        200,
        Vec::new(),
        Vec::new(),
    );

    // 录制http流量
    let playback_service = PLAYBACK_SERVICE.read().await;
    match playback_service.as_ref() {
        Some(playback_service) => {
            match options.mode {
                ProxyMode::Record => {
                    record_http_traffic(record, playback_service).await?;
                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .body(Full::new(Bytes::from("Record added successfully")))
                        .map_err(|e| Error::Http1(e.to_string()))?)
                }
                ProxyMode::Playback => {
                    let response = match forward_http_traffic(record, playback_service).await {
                        Ok(r) => r,
                        Err(e) => {
                            error!("Failed to forward HTTP traffic: {:?}", e);
                            return Ok(Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Full::new(Bytes::from("Failed to forward HTTP traffic")))
                                .map_err(|e| Error::Http1(e.to_string()))?)
                        }
                    };
                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .body(Full::new(Bytes::from(response)))
                        .map_err(|e| Error::Http1(e.to_string()))?)
                }
            }
        }
        None => {
            error!("Playback service not initialized");
            Err(Error::Proxy("Playback service not initialized".to_string()))
        }
    }
}

async fn record_http_traffic(
    record: model::TrafficRecord,
    playback_service: &Arc<PlaybackService>,
) -> Result<(), Error> {
    playback_service.add_record(record).await
}

async fn forward_http_traffic(
    record: model::TrafficRecord,
    playback_service: &Arc<PlaybackService>,
) -> Result<Vec<u8>, Error> {
    playback_service.trigger_replay(&record).await
}