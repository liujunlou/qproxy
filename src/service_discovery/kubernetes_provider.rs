use super::{ServiceDiscoveryBackend, ServiceInstance};
use crate::errors::Error;
use async_trait::async_trait;
use http::Uri;
use k8s_openapi::api::core::v1::Service;
use kube::config::Kubeconfig;
use kube::{
    api::{Api, ListParams},
    config::{self, AuthInfo, KubeConfigOptions},
    Client, Config,
};
use secrecy::SecretString;
use std::{collections::HashMap, str::FromStr};
use tokio::fs;
use tracing::{error, info};

pub struct KubernetesServiceDiscovery {
    client: Client,
    namespace: String,
    label_selector: Option<String>,
}

impl KubernetesServiceDiscovery {
    pub async fn new(
        namespace: String,
        service_account_token_path: Option<String>,
        api_server: Option<String>,
        label_selector: Option<String>,
    ) -> Result<Self, Error> {
        let client = match (api_server, service_account_token_path) {
            (Some(server), Some(token_path)) => {
                let token = fs::read_to_string(&token_path).await.map_err(|e| {
                    Error::ServiceDiscovery(format!("Failed to read token file: {}", e))
                })?;

                let server_uri = Uri::from_str(&server)
                    .map_err(|e| Error::ServiceDiscovery(format!("Invalid server URL: {}", e)))?;

                let mut config = Config::new(server_uri);
                config.default_namespace = namespace.clone();
                config.auth_info = AuthInfo {
                    token: Some(SecretString::from(token)),
                    ..Default::default()
                };

                Client::try_from(config).map_err(|e| {
                    Error::ServiceDiscovery(format!("Failed to create Kubernetes client: {}", e))
                })?
            }
            _ => Client::try_default().await.map_err(|e| {
                Error::ServiceDiscovery(format!(
                    "Failed to create default Kubernetes client: {}",
                    e
                ))
            })?,
        };

        Ok(Self {
            client,
            namespace,
            label_selector,
        })
    }

    async fn list_services(&self) -> Result<Vec<Service>, Error> {
        let api: Api<Service> = Api::namespaced(self.client.clone(), &self.namespace);
        let mut params = ListParams::default();

        if let Some(selector) = &self.label_selector {
            params = params.labels(selector);
        }

        api.list(&params)
            .await
            .map(|list| list.items)
            .map_err(|e| Error::ServiceDiscovery(format!("Failed to list services: {}", e)))
    }

    fn service_to_instance(svc: Service) -> Option<ServiceInstance> {
        let metadata = svc.metadata;
        let spec = svc.spec?;
        let name = metadata.name?;
        let cluster_ip = spec.cluster_ip?;
        let port = spec.ports?.first()?.port;

        let mut instance_metadata = HashMap::new();
        if let Some(labels) = metadata.labels {
            instance_metadata.extend(labels);
        }
        if let Some(annotations) = metadata.annotations {
            instance_metadata.extend(annotations);
        }

        Some(ServiceInstance {
            name,
            host: cluster_ip,
            port: port as u16,
            metadata: instance_metadata,
        })
    }
}

#[async_trait]
impl ServiceDiscoveryBackend for KubernetesServiceDiscovery {
    async fn init(&self) -> Result<(), Error> {
        // 测试连接
        self.list_services().await?;
        info!("Successfully connected to Kubernetes API server");
        Ok(())
    }

    async fn register(&self, _instance: ServiceInstance) -> Result<(), Error> {
        // Kubernetes中服务注册通常通过YAML或Operator完成
        Err(Error::ServiceDiscovery(
            "Service registration not supported in Kubernetes provider".to_string(),
        ))
    }

    async fn deregister(&self, _name: &str) -> Result<(), Error> {
        // Kubernetes中服务注销通常通过YAML或Operator完成
        Err(Error::ServiceDiscovery(
            "Service deregistration not supported in Kubernetes provider".to_string(),
        ))
    }

    async fn get_instances(&self, name: &str) -> Result<Vec<ServiceInstance>, Error> {
        let services = self.list_services().await?;

        Ok(services
            .into_iter()
            .filter(|svc| {
                svc.metadata
                    .name
                    .as_ref()
                    .map(|n| n == name)
                    .unwrap_or(false)
            })
            .filter_map(Self::service_to_instance)
            .collect())
    }

    async fn get_all_services(&self) -> Result<Vec<ServiceInstance>, Error> {
        let services = self.list_services().await?;

        Ok(services
            .into_iter()
            .filter_map(Self::service_to_instance)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ServicePort, ServiceSpec, ServiceStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn create_test_k8s_service(name: &str, cluster_ip: &str, port: i32) -> Service {
        Service {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                labels: Some({
                    let mut map = BTreeMap::new();
                    map.insert("app".to_string(), "test".to_string());
                    map
                }),
                ..ObjectMeta::default()
            },
            spec: Some(ServiceSpec {
                cluster_ip: Some(cluster_ip.to_string()),
                ports: Some(vec![ServicePort {
                    port,
                    ..ServicePort::default()
                }]),
                ..ServiceSpec::default()
            }),
            status: Some(ServiceStatus::default()),
        }
    }

    #[test]
    fn test_service_to_instance_conversion() {
        let k8s_service = create_test_k8s_service("test-service", "10.0.0.1", 8080);
        let instance = KubernetesServiceDiscovery::service_to_instance(k8s_service).unwrap();

        assert_eq!(instance.name, "test-service");
        assert_eq!(instance.host, "10.0.0.1");
        assert_eq!(instance.port, 8080);
        assert_eq!(instance.metadata.get("app").unwrap(), "test");
    }

    #[test]
    fn test_service_to_instance_missing_data() {
        let mut service = create_test_k8s_service("test-service", "10.0.0.1", 8080);
        service.spec = None;

        assert!(KubernetesServiceDiscovery::service_to_instance(service).is_none());
    }
}
