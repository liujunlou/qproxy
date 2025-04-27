use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{error, info};

use crate::errors::Error;
use crate::model::TrafficRecord;
use crate::options::{Options, PeerOptions};
use crate::playback::PlaybackService;

/// 同步服务，用于同步流量记录
/// 定时从peer拉取流量记录，并回放
pub struct SyncService {
    options: Arc<Options>,
    playback_service: Arc<PlaybackService>,
}

impl SyncService {
    pub fn new(options: Options, playback_service: Arc<PlaybackService>) -> Self {
        Self {
            options: Arc::new(options),
            playback_service,
        }
    }

    pub async fn start(&self) {
        if let Some(peer) = &self.options.peer {
            let peer = peer.clone();
            let playback_service = self.playback_service.clone();
            
            tokio::spawn(async move {
                loop {
                    if let Err(e) = Self::sync_from_peer(&peer, playback_service.clone()).await {
                        error!("Failed to sync from peer: {}", e);
                    }
                    time::sleep(Duration::from_secs(60)).await;
                }
            });
        }
    }

    async fn sync_from_peer(peer: &PeerOptions, playback_service: Arc<PlaybackService>) -> Result<(), Error> {
        let client = reqwest::Client::new();
        let scheme = if peer.tls { "https" } else { "http" };
        let url = format!("{}://{}:{}/sync", scheme, peer.host, peer.port);

        info!("Syncing traffic records from peer: {}", url);

        let response = client.get(&url)
            .send()
            .await
            .map_err(|e| Error::Proxy(format!("Failed to connect to peer: {}", e)))?;

        let records: Vec<TrafficRecord> = response.json()
            .await
            .map_err(|e| Error::Proxy(format!("Failed to parse peer response: {}", e)))?;

        for record in records {
            playback_service.add_record(record).await;
        }

        info!("Successfully synced traffic records from peer");
        Ok(())
    }
} 