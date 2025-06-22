use crate::{errors::Error, METRICS_COLLECTOR};
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Body;
use tracing::error;

/// 处理健康检查请求
pub async fn handle_health_check<B>(_req: Request<B>) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    let metrics_collector = METRICS_COLLECTOR.read().await;
    if let Some(metrics_collector) = metrics_collector.as_ref() {
        let health = match metrics_collector.get_health().await {
            Ok(health) => health,
            Err(e) => e.to_string(),
        };
        return Ok(Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(health)))
            .unwrap());
    } else {
        error!("Metrics collector not initialized");
        return Ok(Response::builder()
            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Metrics collector not initialized")))
            .unwrap());
    }
}
