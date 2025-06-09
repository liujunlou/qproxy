use crate::errors::Error;
use crate::options::{Options, ServiceDiscoveryProvider};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use url::Url;

mod kubernetes_provider;
mod static_provider;
mod zookeeper_provider;

use kubernetes_provider::KubernetesServiceDiscovery;
use static_provider::StaticServiceDiscovery;
use zookeeper_provider::ZookeeperServiceDiscovery;

#[derive(Clone, Serialize, Deserialize, Debug)]
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
    // 被代理服务 服务发现组件，这里如果要支持动态监听服务节点，需要改为Arc<RwLock<ServiceDiscoveryBackendEnum>>
    backend: Arc<ServiceDiscoveryBackendEnum>,
    services_cache: Arc<RwLock<HashMap<String, ServiceInstance>>>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self {
            backend: Arc::new(ServiceDiscoveryBackendEnum::Static(
                StaticServiceDiscovery::new(),
            )),
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
                        provider
                            .add_service(ServiceInstance {
                                name: service.name.clone(),
                                host: service.host.clone(),
                                port: service.port,
                                metadata: service.metadata.clone(),
                            })
                            .await?;
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
                    )
                    .await?;
                    Arc::new(ServiceDiscoveryBackendEnum::Zookeeper(zk))
                } else {
                    warn!("Zookeeper configuration is missing, falling back to static provider");
                    Arc::new(ServiceDiscoveryBackendEnum::Static(
                        StaticServiceDiscovery::new(),
                    ))
                }
            }
            ServiceDiscoveryProvider::Kubernetes => {
                if let Some(k8s_config) = &options.service_discovery.config.kubernetes {
                    match KubernetesServiceDiscovery::new(
                        k8s_config.namespace.clone(),
                        k8s_config.service_account_token_path.clone(),
                        k8s_config.api_server.clone(),
                        k8s_config.label_selector.clone(),
                    )
                    .await
                    {
                        Ok(k8s) => Arc::new(ServiceDiscoveryBackendEnum::Kubernetes(k8s)),
                        Err(e) => {
                            error!("Failed to initialize Kubernetes provider: {}, falling back to static provider", e);
                            Arc::new(ServiceDiscoveryBackendEnum::Static(
                                StaticServiceDiscovery::new(),
                            ))
                        }
                    }
                } else {
                    warn!("Kubernetes configuration is missing, falling back to static provider");
                    Arc::new(ServiceDiscoveryBackendEnum::Static(
                        StaticServiceDiscovery::new(),
                    ))
                }
            }
        };

        self.backend.init().await?;
        self.refresh_cache().await?;
        info!(
            "Service registry initialized with provider: {:?}",
            options.service_discovery.provider
        );
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

    /// 注册服务节点
    pub async fn register(&self, instance: ServiceInstance) -> Result<(), Error> {
        self.backend.register(instance.clone()).await?;
        let mut cache = self.services_cache.write().await;
        cache.insert(instance.name.clone(), instance);
        Ok(())
    }

    /// 注销服务节点
    pub async fn deregister(&self, name: &str) -> Result<(), Error> {
        self.backend.deregister(name).await?;
        let mut cache = self.services_cache.write().await;
        cache.remove(name);
        Ok(())
    }

    /// 查找本地服务节点
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

    /// 从URL中提取服务名称
    pub fn extract_service_name(&self, url: &str) -> Result<String, Error> {
        let parsed_url =
            Url::parse(url).map_err(|e| Error::Config(format!("Invalid URL: {}", e)))?;

        let host = parsed_url
            .host_str()
            .ok_or_else(|| Error::Config("No host in URL".to_string()))?;

        let service_name = host
            .split('.')
            .next()
            .ok_or_else(|| Error::Config("Invalid service name".to_string()))?
            .to_string();

        Ok(service_name)
    }

    /// 从URL中查找本地服务节点
    pub async fn find_local_service_from_url(
        &self,
        original_url: &str,
    ) -> Result<Option<ServiceInstance>, Error> {
        let service_name = self.extract_service_name(original_url)?;
        self.find_local_service(&service_name).await
    }
}
