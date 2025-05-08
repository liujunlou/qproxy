use std::sync::Arc;
use reqwest::{Client, ClientBuilder};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::errors::Error;
use crate::model::TrafficRecord;
use crate::options::{Options, PeerOptions};
use crate::playback::PlaybackService;
use crate::get_playback_service;

/// 同步服务，用于同步流量记录
/// 定时从peer拉取流量记录，并回放
#[derive(Clone)]
pub struct SyncService {
    options: Arc<Options>,
    client: Arc<Client>,
}

impl SyncService {
    pub fn new(options: Options) -> Result<Self, Error> {
        let client = ClientBuilder::new()
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        
        Ok(Self {
            options: Arc::new(options),
            client: Arc::new(client),
        })
    }

    /// 开启定时任务，拉取待回放流量
    pub async fn start(&self) -> JoinHandle<()> {
        let options = self.options.clone();
        let service = self.clone();
        
        tokio::spawn(async move {
            loop {
                if let Err(e) = service.sync_with_peers(&options).await {
                    error!("Sync error: {}", e);
                }
                // 每60秒同步一次
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                ()
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
    async fn sync_from_peer(&self, peer: &PeerOptions, playback_service: Arc<PlaybackService>) -> Result<(), Error> {
        let client = self.client.clone();
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
            playback_service.add_record(record.clone()).await?;
        }

        info!("Successfully synced and replayed traffic records from peer");
        Ok(())
    }

    /// 从peer拉取流量记录并推到回放服务
    async fn sync_with_peers(&self, options: &Arc<Options>) -> Result<(), Error> {
        if let Some(peer) = &options.peer {
            if !options.sync.enabled {
                return Ok(());
            }
            let peer = peer.clone();
            
            let playback_service = get_playback_service().await?;
            if let Err(e) = self.sync_from_peer(&peer, playback_service).await {
                error!("Failed to sync from peer: {}", e);
            }
        }
        Ok(())
    }
} 