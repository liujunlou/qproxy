use crate::model::TrafficRecord;
use crate::sync::SyncService;
use crate::{errors::Error, options::Options};
use crate::{ONCE_FILTER_CHAIN, PLAYBACK_SERVICE};
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use serde_json::json;
use std::sync::Arc;
use tracing::{error, info};

/// 处理同步请求
pub async fn handle_sync_request<B>(
    req: Request<B>,
    options: Arc<Options>,
) -> Result<Response<Full<Bytes>>, Error>
where
    B: Body,
{
    let path = req.uri().path();
    let method = req.method().as_str();

    match (method, path) {
        ("GET", "/sync") => {
            // 获取 GET 请求的params参数
            let query_params = req.uri().query().unwrap_or("");
            let params: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(query_params.as_bytes())
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();

            let default_peer = "default".to_string();
            let default_shard = "default".to_string();
            let default_from = "internet".to_string();

            let peer_id = params.get("peer_id").unwrap_or(&default_peer);
            let shard_id = params.get("shard_id").unwrap_or(&default_shard);
            // 区分互联网端和警务网
            let from = params.get("from").unwrap_or(&default_from);
            info!(
                "Getting sync records for peer: {}, shard: {}",
                peer_id, shard_id
            );

            let playback_service = PLAYBACK_SERVICE.read().await;
            let records = if let Some(playback_service) = playback_service.as_ref() {
                playback_service
                    .get_records_for_sync(&peer_id, &shard_id)
                    .await
                    .map_err(|e| Error::Proxy(format!("Failed to get sync records: {}", e)))?
            } else {
                vec![]
            };
            // 判断拉取端，如果是互联网端，则需要做数据过滤
            let records = if from.eq_ignore_ascii_case("internet") {
                let mut filtered_records = Vec::new();
                for record in records {
                    let filtered = if options.http.filter_fields.is_some() {
                        ONCE_FILTER_CHAIN.read().await.filter(&record)
                    } else {
                        Ok(record)
                    }?;
                    filtered_records.push(filtered);
                }
                filtered_records
            } else {
                records
            };

            let json = serde_json::to_string(&records)
                .map_err(|e| Error::Proxy(format!("Failed to serialize records: {}", e)))?;

            return Ok(Response::builder()
                .status(http::StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(json)))
                .unwrap());
        }
        ("POST", "/sync") => {
            // 接收并同步记录
            let body = req
                .into_body()
                .collect()
                .await
                .map_err(|_| Error::Http1("Failed to collect request body".to_string()))?
                .to_bytes();
            let record: TrafficRecord = serde_json::from_slice(&body)
                .map_err(|e| Error::Proxy(format!("Failed to parse records: {}", e)))?;

            info!("Receive forward record: {}", record.id);

            // 返回透传的真实响应数据
            let playback_service = PLAYBACK_SERVICE.read().await;
            let response = if let Some(playback_service) = playback_service.as_ref() {
                match playback_service
                    .sync_from_peer(&mut record.clone())
                    .await
                    .map_err(|e| {
                        error!("Failed to sync from peer: {}", e);
                        Error::Proxy(format!("Failed to sync from peer: {}", e))
                    }) {
                        Ok(resp) => resp,
                        Err(e) => {
                            error!("Failed to sync from peer: {}", e);
                            return Err(e);
                        }
                    }
            } else {
                return Err(Error::Proxy("Playback service not initialized".to_string()));
            };

            let response_str = String::from_utf8(response).map_err(|e| {
                Error::Proxy(format!("Failed to convert response to string: {}", e))
            })?;

            info!("Sync post response: {}", response_str);
            return Ok(Response::builder()
                .status(http::StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(response_str)))
                .unwrap());
        }
        _ => {
            let response = json!({
                "error": "Method Not Allowed",
                "message": "The requested method is not allowed for this endpoint"
            });

            Ok(Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(response.to_string())))
                .unwrap())
        }
    }
}
