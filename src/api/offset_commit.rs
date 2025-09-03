use crate::errors::Error;
use crate::model::Offset;
use crate::options::Options;
use crate::playback::CheckpointInfo;
use crate::PLAYBACK_SERVICE;
use bytes::Bytes;
use http::Response;
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use hyper::Request;
use serde_json::json;
use tracing::{error, info};

/// 处理offset commit请求
pub async fn handle_offset_commit_request<B>(
    req: Request<B>,
) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    let path = req.uri().path();
    let method = req.method().as_str();

    match (method, path) {
        ("POST", "/commit") => {
            // 获取 POST 请求的body参数
            let body = req
                .into_body()
                .collect()
                .await
                .map_err(|_| Error::Http1("Failed to collect request body".to_string()))?
                .to_bytes();
            let offset: Offset = serde_json::from_slice(&body)
                .map_err(|e| Error::Proxy(format!("Failed to parse records: {}", e)))?;

            info!("Receive offset commit {:?}", offset.clone());
            // 更新offset
            if let Err(e) = update_offset(&offset).await {
                error!(
                    "Failed to update offset for peer: {} for {:?}",
                    offset.peer_id.clone(),
                    e
                );
                let response = json!({
                    "error": "Internal Server Error",
                    "message": "Failed to update offset"
                });
                let response = serde_json::to_string(&response)?;
                return Ok(Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from(response)))
                    .unwrap());
            }
            let response = json!({
                "error": "OK",
                "message": "Offset updated successfully"
            });
            let response = serde_json::to_string(&response)?;
            return Ok(Response::builder()
                .status(http::StatusCode::OK)
                .body(Full::new(Bytes::from(response)))
                .unwrap());
        }
        _ => {
            let response = json!({
                "error": "Not Found",
                "message": "The requested resource was not found"
            });
            let response = serde_json::to_string(&response)?;
            return Ok(Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from(response)))
                .unwrap());
        }
    }
}

/// 更新offset, 即更新redis中的CheckpointInfo
async fn update_offset(offset: &Offset) -> Result<(), Error> {
    let playback_service = PLAYBACK_SERVICE.read().await;
    if let Some(playback_service) = playback_service.as_ref() {
        let checkpoint = CheckpointInfo::new_with_offset(
            &offset.peer_id,
            &offset.shard_id,
            offset.offset,
            &offset.record_id,
        )?;
        playback_service.update_checkpoint(&checkpoint).await?;
    }
    Ok(())
}
