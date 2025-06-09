use super::{ServiceDiscoveryBackend, ServiceInstance};
use crate::errors::Error;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct StaticServiceDiscovery {
    services: Arc<RwLock<HashMap<String, ServiceInstance>>>,
}

impl StaticServiceDiscovery {
    /// 创建新的静态服务发现实例
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 添加服务实例
    pub async fn add_service(&self, instance: ServiceInstance) -> Result<(), Error> {
        let mut services = self.services.write().await;
        services.insert(instance.name.clone(), instance);
        Ok(())
    }
}

#[async_trait]
impl ServiceDiscoveryBackend for StaticServiceDiscovery {
    async fn init(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn register(&self, instance: ServiceInstance) -> Result<(), Error> {
        self.add_service(instance).await
    }

    async fn deregister(&self, name: &str) -> Result<(), Error> {
        let mut services = self.services.write().await;
        services.remove(name);
        Ok(())
    }

    async fn get_instances(&self, name: &str) -> Result<Vec<ServiceInstance>, Error> {
        let services = self.services.read().await;
        Ok(services.get(name).cloned().map(|s| vec![s]).unwrap_or_default())
    }

    async fn get_all_services(&self) -> Result<Vec<ServiceInstance>, Error> {
        let services = self.services.read().await;
        Ok(services.values().cloned().collect())
    }
}
