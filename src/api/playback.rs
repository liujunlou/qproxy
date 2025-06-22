use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Body;
use tracing::{error, info};

use crate::{errors::Error, PLAYBACK_SERVICE};

/// 处理回放请求
pub async fn handle_playback_request<B>(req: Request<B>) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    info!("Handling playback request");

    let playback_service = PLAYBACK_SERVICE.read().await;
    if let Some(playback_service) = playback_service.as_ref() {
        // 直接调用回放服务的 playback 方法
        let response = playback_service.playback(req).await;
        Ok(response)
    } else {
        error!("Playback service not initialized");
        Err(Error::Proxy("Playback service not initialized".to_string()))
    }
}
