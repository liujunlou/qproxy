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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_instance() -> ServiceInstance {
        ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_static_provider_init() {
        let provider = StaticServiceDiscovery::new();
        provider.add_service(create_test_instance()).await.unwrap();
        assert!(provider.init().await.is_ok());
    }

    #[tokio::test]
    async fn test_static_provider_get_instances() {
        let instance = create_test_instance();
        let provider = StaticServiceDiscovery::new();
        provider.add_service(instance.clone()).await.unwrap();
        
        let instances = provider.get_instances("test-service").await.unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].name, "test-service");
        assert_eq!(instances[0].host, "localhost");
        assert_eq!(instances[0].port, 8080);
    }

    #[tokio::test]
    async fn test_static_provider_get_all_services() {
        let instance = create_test_instance();
        let provider = StaticServiceDiscovery::new();
        provider.add_service(instance.clone()).await.unwrap();
        
        let services = provider.get_all_services().await.unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "test-service");
    }

    #[tokio::test]
    async fn test_static_provider_register_unsupported() {
        let provider = StaticServiceDiscovery::new();
        let result = provider.register(create_test_instance()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_static_provider_deregister_unsupported() {
        let provider = StaticServiceDiscovery::new();
        let result = provider.deregister("test-service").await;
        assert!(result.is_err());
    }
} 