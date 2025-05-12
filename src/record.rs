use crate::model::TrafficRecord;
use http::{Request, Response};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct RecordService {
    records: Arc<RwLock<HashMap<String, TrafficRecord>>>,
}

impl RecordService {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn record_request<B>(&self, req: &Request<B>) {
        // TODO: Implement request recording
    }

    pub async fn record_response<B>(&self, resp: &Response<B>) {
        // TODO: Implement response recording
    }

    pub async fn get_all_records(&self) -> Vec<TrafficRecord> {
        let records = self.records.read().await;
        records.values().cloned().collect()
    }

    pub async fn clear_records(&self) {
        let mut records = self.records.write().await;
        records.clear();
    }
} 