use std::collections::HashMap;
use std::sync::Arc;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use serde_json;

use crate::errors::Error;
use crate::playback::PlaybackService;

pub async fn handle_sync_request(
    req: Request<hyper::body::Incoming>,
    playback_service: Arc<PlaybackService>,
) -> Result<Response<Full<Bytes>>, Error> {
    let uri = req.uri().clone();
    let query_params: HashMap<_, _> = uri.query()
        .map(|q| q.split('&')
            .filter_map(|param| {
                let parts: Vec<&str> = param.split('=').collect();
                if parts.len() == 2 {
                    Some((parts[0], parts[1]))
                } else {
                    None
                }
            })
            .collect())
        .unwrap_or_default();

    let peer_id = query_params.get("peer_id").unwrap_or(&"default");
    let shard_id = query_params.get("shard_id").unwrap_or(&"default");

    let records = playback_service.get_recent_records(peer_id, shard_id, None).await?;
    
    let json = serde_json::to_string(&records)
        .map_err(|e| Error::Proxy(format!("Failed to serialize records: {}", e)))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .unwrap())
} 