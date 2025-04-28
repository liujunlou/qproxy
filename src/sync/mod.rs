use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::errors::Error;
use crate::model::TrafficRecord;
use crate::options::{Options, PeerOptions};
use crate::playback::PlaybackService;
use crate::PLAYBACK_SERVICE;

/// 同步服务，用于同步流量记录
/// 定时从peer拉取流量记录，并回放
pub struct SyncService {
    options: Arc<Options>,
}

impl SyncService {
    pub fn new(options: Options) -> Self {
        Self {
            options: Arc::new(options),
        }
    }

    pub async fn start(&self) -> JoinHandle<()> {
        let options = self.options.clone();
        
        tokio::spawn(async move {
            loop {
                if let Err(e) = SyncService::sync_with_peers(&options).await {
                    error!("Sync error: {}", e);
                }
                // 每60秒同步一次
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        })
    }

    pub async fn abort(handle: JoinHandle<()>) {
        handle.abort_handle().abort();
        if let Err(e) = handle.await {
            error!("Failed to abort sync service: {}", e);
        }
    }

    /// 从peer拉取流量记录并推到回放服务
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
            playback_service.add_record(record.clone()).await;
        }

        info!("Successfully synced and replayed traffic records from peer");
        Ok(())
    }

    async fn sync_with_peers(options: &Arc<Options>) -> Result<(), Error> {
        if let Some(peer) = &options.peer {
            let peer = peer.clone();
            
            match PLAYBACK_SERVICE.lock().await.clone() {
                Some(playback_service) => {
                    if let Err(e) = SyncService::sync_from_peer(&peer, playback_service).await {
                        error!("Failed to sync from peer: {}", e);
                    }
                }
                None => {
                    error!("Playback service is not initialized");
                }
            }
        }
        Ok(())
    }
} 