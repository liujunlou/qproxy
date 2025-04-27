use std::sync::Arc;
use rand::seq::IndexedRandom;
use tokio::sync::RwLock;
use std::collections::HashMap;
use url::Url;
use crate::errors::Error;

#[derive(Clone)]
pub struct ServiceRegistry {
    // 服务名到服务实例列表的映射
    services: Arc<RwLock<HashMap<String, Vec<ServiceInstance>>>>,
}

#[derive(Clone, Debug)]
pub struct ServiceInstance {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub metadata: HashMap<String, String>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // 注册服务实例
    pub async fn register(&self, instance: ServiceInstance) {
        let mut services = self.services.write().await;
        let instances = services.entry(instance.name.clone()).or_insert_with(Vec::new);
        instances.push(instance);
    }

    // 注销服务实例
    pub async fn deregister(&self, name: &str, host: &str, port: u16) {
        let mut services = self.services.write().await;
        if let Some(instances) = services.get_mut(name) {
            instances.retain(|i| i.host != host || i.port != port);
        }
    }

    // 根据服务名查找服务实例
    pub async fn find_service(&self, name: &str) -> Option<Vec<ServiceInstance>> {
        let services = self.services.read().await;
        services.get(name).cloned()
    }

    // 从URL解析服务名
    pub fn extract_service_name(&self, url: &str) -> Result<String, Error> {
        let parsed_url = Url::parse(url)
            .map_err(|e| Error::Config(format!("Invalid URL: {}", e)))?;
        
        let host = parsed_url.host_str()
            .ok_or_else(|| Error::Config("No host in URL".to_string()))?;

        // 从主机名中提取服务名
        // 例如: service.namespace.svc.cluster.local -> service
        let service_name = host.split('.')
            .next()
            .ok_or_else(|| Error::Config("Invalid service name".to_string()))?
            .to_string();

        Ok(service_name)
    }

    // 根据原始URL查找对应的本地服务
    pub async fn find_local_service(&self, original_url: &str) -> Result<Option<ServiceInstance>, Error> {
        let service_name = self.extract_service_name(original_url)?;
        
        if let Some(instances) = self.find_service(&service_name).await {
            // 简单的负载均衡：随机选择一个实例
            let mut rng = rand::rng();
            Ok(instances.choose(&mut rng).cloned())
        } else {
            Ok(None)
        }
    }
}

// 全局服务注册表
pub static SERVICE_REGISTRY: once_cell::sync::Lazy<ServiceRegistry> = once_cell::sync::Lazy::new(|| {
    ServiceRegistry::new()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_service_registration() {
        let registry = ServiceRegistry::new();
        let instance = ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata: HashMap::new(),
        };

        registry.register(instance.clone()).await;
        let services = registry.find_service("test-service").await.unwrap();
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

        registry.register(instance).await;
        registry.deregister("test-service", "localhost", 8080).await;
        let services = registry.find_service("test-service").await.unwrap();
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

        registry.register(instance).await;

        // 测试查找存在的服务
        let result = registry.find_local_service("http://user-service:8080/api/users").await.unwrap();
        assert!(result.is_some());
        let service = result.unwrap();
        assert_eq!(service.name, "user-service");
        assert_eq!(service.port, 8080);

        // 测试查找不存在的服务
        let result = registry.find_local_service("http://unknown-service:8080/api").await.unwrap();
        assert!(result.is_none());
    }
} 