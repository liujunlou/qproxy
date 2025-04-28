use std::sync::Arc;
use rand::seq::IndexedRandom;
use tokio::sync::RwLock;
use std::collections::HashMap;
use url::Url;
use crate::errors::Error;
use tracing::{info, error, warn};
use crate::options::{Options, ServiceConfig, ServiceDiscoveryProvider};
use async_trait::async_trait;

mod static_provider;
mod zookeeper_provider;
mod kubernetes_provider;

use static_provider::StaticServiceDiscovery;
use zookeeper_provider::ZookeeperServiceDiscovery;
use kubernetes_provider::KubernetesServiceDiscovery;

#[derive(Clone, Debug)]
pub struct ServiceInstance {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub metadata: HashMap<String, String>,
}

impl ServiceInstance {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[async_trait]
pub trait ServiceDiscoveryBackend: Send + Sync {
    async fn init(&self) -> Result<(), Error>;
    async fn register(&self, instance: ServiceInstance) -> Result<(), Error>;
    async fn deregister(&self, name: &str) -> Result<(), Error>;
    async fn get_instances(&self, name: &str) -> Result<Vec<ServiceInstance>, Error>;
    async fn get_all_services(&self) -> Result<Vec<ServiceInstance>, Error>;
}

pub enum ServiceDiscoveryBackendEnum {
    Static(StaticServiceDiscovery),
    Zookeeper(ZookeeperServiceDiscovery),
    Kubernetes(KubernetesServiceDiscovery),
}

#[async_trait]
impl ServiceDiscoveryBackend for ServiceDiscoveryBackendEnum {
    async fn init(&self) -> Result<(), Error> {
        match self {
            Self::Static(s) => s.init().await,
            Self::Zookeeper(z) => z.init().await,
            Self::Kubernetes(k) => k.init().await,
        }
    }

    async fn register(&self, instance: ServiceInstance) -> Result<(), Error> {
        match self {
            Self::Static(s) => s.register(instance).await,
            Self::Zookeeper(z) => z.register(instance).await,
            Self::Kubernetes(k) => k.register(instance).await,
        }
    }

    async fn deregister(&self, name: &str) -> Result<(), Error> {
        match self {
            Self::Static(s) => s.deregister(name).await,
            Self::Zookeeper(z) => z.deregister(name).await,
            Self::Kubernetes(k) => k.deregister(name).await,
        }
    }

    async fn get_instances(&self, name: &str) -> Result<Vec<ServiceInstance>, Error> {
        match self {
            Self::Static(s) => s.get_instances(name).await,
            Self::Zookeeper(z) => z.get_instances(name).await,
            Self::Kubernetes(k) => k.get_instances(name).await,
        }
    }

    async fn get_all_services(&self) -> Result<Vec<ServiceInstance>, Error> {
        match self {
            Self::Static(s) => s.get_all_services().await,
            Self::Zookeeper(z) => z.get_all_services().await,
            Self::Kubernetes(k) => k.get_all_services().await,
        }
    }
}

pub struct ServiceRegistry {
    backend: Arc<ServiceDiscoveryBackendEnum>,
    services_cache: Arc<RwLock<HashMap<String, ServiceInstance>>>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self {
            backend: Arc::new(ServiceDiscoveryBackendEnum::Static(StaticServiceDiscovery::new())),
            services_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 初始化服务注册表
    pub async fn init(&mut self, options: &Options) -> Result<(), Error> {
        self.backend = match options.service_discovery.provider {
            ServiceDiscoveryProvider::Static => {
                let provider = StaticServiceDiscovery::new();
                if let Some(services) = &options.service_discovery.config.static_services {
                    for service in services {
                        provider.add_service(ServiceInstance {
                            name: service.name.clone(),
                            host: service.host.clone(),
                            port: service.port,
                            metadata: service.metadata.clone(),
                        }).await?;
                    }
                }
                Arc::new(ServiceDiscoveryBackendEnum::Static(provider))
            }
            ServiceDiscoveryProvider::Zookeeper => {
                if let Some(zk_config) = &options.service_discovery.config.zookeeper {
                    let zk = ZookeeperServiceDiscovery::new(
                        zk_config.hosts.clone(),
                        zk_config.base_path.clone(),
                        zk_config.timeout.unwrap_or(30000),
                    ).await;
                    Arc::new(ServiceDiscoveryBackendEnum::Zookeeper(zk))
                } else {
                    return Err(Error::Config("Zookeeper configuration is missing".to_string()));
                }
            }
            ServiceDiscoveryProvider::Kubernetes => {
                if let Some(k8s_config) = &options.service_discovery.config.kubernetes {
                    Arc::new(ServiceDiscoveryBackendEnum::Kubernetes(KubernetesServiceDiscovery::new(
                        k8s_config.namespace.clone(),
                        k8s_config.service_account_token_path.clone(),
                        k8s_config.api_server.clone(),
                        k8s_config.label_selector.clone(),
                    ).await?))
                } else {
                    return Err(Error::Config("Kubernetes configuration is missing".to_string()));
                }
            }
        };

        self.backend.init().await?;
        self.refresh_cache().await?;
        info!("Service registry initialized with provider: {:?}", options.service_discovery.provider);
        Ok(())
    }

    /// 刷新服务缓存
    async fn refresh_cache(&self) -> Result<(), Error> {
        let services = self.backend.get_all_services().await?;
        let mut cache = self.services_cache.write().await;
        cache.clear();
        for service in services {
            cache.insert(service.name.clone(), service);
        }
        Ok(())
    }

    pub async fn register(&self, instance: ServiceInstance) -> Result<(), Error> {
        self.backend.register(instance.clone()).await?;
        let mut cache = self.services_cache.write().await;
        cache.insert(instance.name.clone(), instance);
        Ok(())
    }

    pub async fn deregister(&self, name: &str) -> Result<(), Error> {
        self.backend.deregister(name).await?;
        let mut cache = self.services_cache.write().await;
        cache.remove(name);
        Ok(())
    }

    pub async fn find_local_service(&self, name: &str) -> Result<Option<ServiceInstance>, Error> {
        // 先查缓存
        let cache = self.services_cache.read().await;
        if let Some(service) = cache.get(name) {
            return Ok(Some(service.clone()));
        }

        // 缓存未命中，查询后端
        let instances = self.backend.get_instances(name).await?;
        if let Some(instance) = instances.first() {
            let mut cache = self.services_cache.write().await;
            cache.insert(name.to_string(), instance.clone());
            Ok(Some(instance.clone()))
        } else {
            Ok(None)
        }
    }

    pub async fn get_all_services(&self) -> Result<Vec<ServiceInstance>, Error> {
        let cache = self.services_cache.read().await;
        Ok(cache.values().cloned().collect())
    }

    pub fn extract_service_name(&self, url: &str) -> Result<String, Error> {
        let parsed_url = Url::parse(url)
            .map_err(|e| Error::Config(format!("Invalid URL: {}", e)))?;
        
        let host = parsed_url.host_str()
            .ok_or_else(|| Error::Config("No host in URL".to_string()))?;

        let service_name = host.split('.')
            .next()
            .ok_or_else(|| Error::Config("Invalid service name".to_string()))?
            .to_string();

        Ok(service_name)
    }

    pub async fn find_local_service_from_url(&self, original_url: &str) -> Result<Option<ServiceInstance>, Error> {
        let service_name = self.extract_service_name(original_url)?;
        self.find_local_service(&service_name).await
    }
}

// 全局服务注册表
pub static SERVICE_REGISTRY: once_cell::sync::Lazy<Arc<RwLock<ServiceRegistry>>> = once_cell::sync::Lazy::new(|| {
    Arc::new(RwLock::new(ServiceRegistry::new()))
});

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_service_instance_new() {
        let mut metadata = HashMap::new();
        metadata.insert("version".to_string(), "1.0".to_string());
        
        let instance = ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata,
        };

        assert_eq!(instance.name, "test-service");
        assert_eq!(instance.host, "localhost");
        assert_eq!(instance.port, 8080);
        assert_eq!(instance.metadata.get("version").unwrap(), "1.0");
    }

    #[test]
    fn test_service_instance_address() {
        let instance = ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata: HashMap::new(),
        };

        assert_eq!(instance.address(), "localhost:8080");
    }

    #[tokio::test]
    async fn test_service_registration() {
        let registry = ServiceRegistry::new();
        let instance = ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata: HashMap::new(),
        };

        registry.register(instance.clone()).await.unwrap();
        let services = registry.get_all_services().await.unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "test-service");
        assert_eq!(services[0].port, 8080);
    }

    #[tokio::test]
    async fn test_service_deregistration() {
        let registry = ServiceRegistry::new();
        let instance = ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata: HashMap::new(),
        };

        registry.register(instance).await.unwrap();
        registry.deregister("test-service").await.unwrap();
        let services = registry.get_all_services().await.unwrap();
        assert_eq!(services.len(), 0);
    }

    #[tokio::test]
    async fn test_service_name_extraction() {
        let registry = ServiceRegistry::new();
        
        // 测试简单URL
        let name = registry.extract_service_name("http://user-service:8080/api/users").unwrap();
        assert_eq!(name, "user-service");

        // 测试带有域名的URL
        let name = registry.extract_service_name("http://user-service.default.svc.cluster.local:8080/api/users").unwrap();
        assert_eq!(name, "user-service");

        // 测试无效URL
        assert!(registry.extract_service_name("invalid-url").is_err());
    }

    #[tokio::test]
    async fn test_find_local_service() {
        let registry = ServiceRegistry::new();
        let instance = ServiceInstance {
            name: "user-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata: HashMap::new(),
        };

        registry.register(instance).await.unwrap();

        // 测试查找存在的服务
        let result = registry.find_local_service("user-service").await.unwrap();
        assert!(result.is_some());
        let service = result.unwrap();
        assert_eq!(service.name, "user-service");
        assert_eq!(service.port, 8080);

        // 测试查找不存在的服务
        let result = registry.find_local_service("unknown-service").await.unwrap();
        assert!(result.is_none());
    }
} 