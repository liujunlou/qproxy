use std::sync::Arc;
use tokio::sync::RwLock;
use prometheus::TextEncoder;
use redis::{AsyncCommands, Commands};
use tokio::time::{interval, Duration};
use sysinfo::{System, Networks};
use crate::{errors::Error, monitor::MetricsCollector, options::{Options, ProxyMode}};

use super::{ServiceStatus, CPU_USAGE_GAUGE, MEMORY_USAGE_GAUGE, NETWORK_CONNECTIONS_GAUGE, NETWORK_IO_GAUGE};

pub struct MetricsCollectorTask {
    opts: Arc<Options>,
    metrics: Arc<RwLock<MetricsCollector>>,
    sys: System,
    network: Networks,
}

impl MetricsCollectorTask {
    pub fn new(opts: &Options) -> Self {
        Self {
            opts: Arc::new(opts.clone()),
            metrics: Arc::new(RwLock::new(MetricsCollector::new(opts.mode.clone()))),
            sys: System::new_all(),
            network: Networks::new(),
        }
    }

    pub async fn start(&mut self) {
        let mut interval = interval(Duration::from_secs(15));
        
        loop {
            interval.tick().await;
            self.collect_metrics().await;
        }
    }

    pub async fn get_metrics(&self) -> Result<String, Error> {
        let encoder = TextEncoder::new();
        let metrics = prometheus::gather();
        encoder.encode_to_string(&metrics).map_err(Error::Metric)
    }

    pub async fn get_health(&self) -> Result<String, Error> {
        let health = self.metrics.read().await
            .get_health_status().await;
        serde_json::to_string(&health).map_err(Error::from)
    }

    async fn collect_metrics(&mut self) {
        // 更新系统信息
        self.sys.refresh_all();
        self.network.refresh(true);

        // 收集 CPU 使用率
        CPU_USAGE_GAUGE.set(self.sys.global_cpu_usage() as f64 / 100.0);
        // 收集内存使用率
        MEMORY_USAGE_GAUGE.set(self.sys.used_memory() as f64 / self.sys.total_memory() as f64);
        // 收集网络IO
        let network_io = self.network.list().iter()
            .map(|(_, data)| data.received() + data.transmitted())
            .sum::<u64>();
        NETWORK_IO_GAUGE.set(network_io as f64);
        // 收集网络连接数
        NETWORK_CONNECTIONS_GAUGE.set(self.network.list().len() as f64);

        // 更新健康状态
        self.update_health_status().await;
    }

    async fn update_health_status(&self) {
        // 检查系统组件状态
        let redis_status = if self.opts.redis.url.is_empty() {
            ServiceStatus::Up
        } else {
            match redis::Client::open(self.opts.redis.url.as_str()) {
                Ok(client) => {
                    match client.get_connection_manager().await {
                        Ok(mut conn) => {
                            match conn.ping::<String>().await {
                                Ok(r) => if r.eq("PONG") {ServiceStatus::Up} else {ServiceStatus::Down},
                                Err(_) => ServiceStatus::Down,
                            }
                        },
                        Err(_) => ServiceStatus::Down,
                    }
                },
                Err(_) => ServiceStatus::Down,
            }
        };

        let system_status = if self.sys.global_cpu_usage() < 0.80 && 
            (self.sys.used_memory() as f64 / self.sys.total_memory() as f64) < 0.80 {
            ServiceStatus::Up
        } else {
            ServiceStatus::Down
        };

        // 更新组件状态
        self.metrics.write().await.update_component_status("redis", redis_status.clone()).await;
        self.metrics.write().await.update_component_status("system", system_status.clone()).await;

        // 更新整体状态
        let overall_status = if redis_status == crate::monitor::ServiceStatus::Up 
            && system_status == crate::monitor::ServiceStatus::Up {
            crate::monitor::ServiceStatus::Up
        } else {
            crate::monitor::ServiceStatus::Down
        };

        self.metrics.write().await.update_health_status(overall_status, None).await;
    }
} 