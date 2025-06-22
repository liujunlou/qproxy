use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Body;
use tracing::error;

use crate::{errors::Error, METRICS_COLLECTOR};


/// 处理 Prometheus 指标请求
pub async fn handle_metrics<B>(_req: Request<B>) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    // 返回prometheus metrics
    let metrics_collector = METRICS_COLLECTOR.read().await;
    if let Some(metrics_collector) = metrics_collector.as_ref() {
        let metrics = match metrics_collector.get_metrics().await {
            Ok(metrics) => metrics,
            Err(e) => e.to_string(),
        };
        return Ok(Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
            .body(Full::new(Bytes::from(metrics)))
            .unwrap());
    } else {
        error!("Metrics collector not initialized");
        return Ok(Response::builder()
            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Metrics collector not initialized")))
            .unwrap());
    }
}