use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;

use crate::playback::PlaybackService;
use crate::sync::SyncService;
use crate::{errors::Error, options::Options};
use hyper::body::Body;
use prometheus::{Encoder, TextEncoder};
use serde_json::json;
use tracing::{error, info};

pub mod health;
pub mod metrics;
pub mod offset_commit;
pub mod playback;
pub mod sync;

/// 处理 API 请求
pub async fn handle_api_request<B>(
    req: Request<B>,
    options: Arc<Options>,
) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    let path = req.uri().path();
    let method = req.method().as_str();

    match (method, path) {
        ("GET", "/health") => health::handle_health_check(req).await,
        ("GET", "/metrics") => metrics::handle_metrics(req).await,
        ("GET", "/sync") => sync::handle_sync_request(req, options).await,
        ("POST", "/sync") => sync::handle_sync_request(req, options).await,
        ("POST", "/playback") => playback::handle_playback_request(req).await,
        ("POST", "/commit") => offset_commit::handle_offset_commit_request(req).await,
        _ => {
            let response = json!({
                "error": "Not Found",
                "message": "The requested resource was not found"
            });
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(response.to_string())))
                .unwrap())
        }
    }
}

#[allow(dead_code)]
pub async fn handle_sync_request(
    req: Request<hyper::body::Incoming>,
    playback_service: Arc<PlaybackService>,
) -> Result<Response<Full<Bytes>>, Error> {
    let uri = req.uri().clone();
    let query_params: HashMap<_, _> = uri
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|param| {
                    let parts: Vec<&str> = param.split('=').collect();
                    if parts.len() == 2 {
                        Some((parts[0], parts[1]))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let peer_id = query_params.get("peer_id").unwrap_or(&"default");
    let shard_id = query_params.get("shard_id").unwrap_or(&"default");

    let records = playback_service
        .get_recent_records(peer_id, shard_id, None)
        .await?;

    let json = serde_json::to_string(&records)
        .map_err(|e| Error::Proxy(format!("Failed to serialize records: {}", e)))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .unwrap())
}
