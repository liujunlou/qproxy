use std::sync::Arc;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use serde_json;

use crate::errors::Error;
use crate::playback::PlaybackService;

pub async fn handle_sync_request(
    _req: Request<hyper::body::Incoming>,
    playback_service: Arc<PlaybackService>,
) -> Result<Response<Full<Bytes>>, Error> {
    let records = playback_service.get_recent_records(None).await;
    
    let json = serde_json::to_string(&records)
        .map_err(|e| Error::Proxy(format!("Failed to serialize records: {}", e)))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .unwrap())
} 