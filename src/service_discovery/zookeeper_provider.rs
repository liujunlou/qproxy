use super::{ServiceDiscoveryBackend, ServiceInstance};
use crate::errors::Error;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info};
use zookeeper_async::{WatchedEvent, Watcher, ZooKeeper};

struct ZkWatcher;
impl Watcher for ZkWatcher {
    fn handle(&self, e: WatchedEvent) {
        info!("Zookeeper event: {:?}", e);
    }
}

#[derive(Serialize, Deserialize)]
struct ServiceData {
    name: String,
    host: String,
    port: u16,
    metadata: std::collections::HashMap<String, String>,
}

pub struct ZookeeperServiceDiscovery {
    zk: ZooKeeper,
    base_path: String,
}

impl ZookeeperServiceDiscovery {
    pub async fn new(hosts: Vec<String>, base_path: String, timeout: u64) -> Self {
        let connect_string = hosts.join(",");
        let zk = ZooKeeper::connect(&connect_string, Duration::from_millis(timeout), ZkWatcher)
            .await
            .expect("Failed to connect to ZooKeeper");
        
        Self {
            zk,
            base_path,
        }
    }

    async fn ensure_path(&self, path: &str) -> Result<(), Error> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current = String::new();
        
        for part in parts {
            current.push('/');
            current.push_str(part);
            
            if let Err(e) = self.zk.exists(&current, false).await {
                self.zk.create(&current, vec![], zookeeper_async::Acl::open_unsafe().clone(), zookeeper_async::CreateMode::Persistent)
                    .await
                    .map_err(|e| Error::ServiceDiscovery(format!("Failed to create ZK path: {}", e)))?;
            }
        }
        
        Ok(())
    }

    fn get_service_path(&self, name: &str) -> String {
        format!("{}/{}", self.base_path, name)
    }
}

#[async_trait]
impl ServiceDiscoveryBackend for ZookeeperServiceDiscovery {
    async fn init(&self) -> Result<(), Error> {
        self.ensure_path(&self.base_path).await
    }

    async fn register(&self, instance: ServiceInstance) -> Result<(), Error> {
        let path = self.get_service_path(&instance.name);
        self.ensure_path(&path).await?;

        let data = ServiceData {
            name: instance.name.clone(),
            host: instance.host.clone(),
            port: instance.port,
            metadata: instance.metadata.clone(),
        };

        let json = serde_json::to_vec(&data)
            .map_err(|e| Error::ServiceDiscovery(format!("Failed to serialize service data: {}", e)))?;

        self.zk.create(
            &format!("{}/node", path),
            json,
            zookeeper_async::Acl::open_unsafe().clone(),
            zookeeper_async::CreateMode::EphemeralSequential,
        )
        .await
        .map_err(|e| Error::ServiceDiscovery(format!("Failed to register service: {}", e)))?;

        Ok(())
    }

    async fn deregister(&self, name: &str) -> Result<(), Error> {
        let path = self.get_service_path(name);
        
        // 获取所有子节点
        let children = self.zk.get_children(&path, false)
            .await
            .map_err(|e| Error::ServiceDiscovery(format!("Failed to get children: {}", e)))?;

        // 删除所有子节点
        for child in children {
            let child_path = format!("{}/{}", path, child);
            if let Err(e) = self.zk.delete(&child_path, None).await {
                error!("Failed to delete node {}: {}", child_path, e);
            }
        }

        // 删除服务节点
        self.zk.delete(&path, None)
            .await
            .map_err(|e| Error::ServiceDiscovery(format!("Failed to deregister service: {}", e)))?;

        Ok(())
    }

    async fn get_instances(&self, name: &str) -> Result<Vec<ServiceInstance>, Error> {
        let path = self.get_service_path(name);
        
        let children = match self.zk.get_children(&path, false).await {
            Ok(c) => c,
            Err(_) => return Ok(vec![]),
        };

        let mut instances = Vec::new();
        for child in children {
            let child_path = format!("{}/{}", path, child);
            if let Ok((data, _)) = self.zk.get_data(&child_path, false).await {
                if let Ok(service_data) = serde_json::from_slice::<ServiceData>(&data) {
                    instances.push(ServiceInstance {
                        name: service_data.name,
                        host: service_data.host,
                        port: service_data.port,
                        metadata: service_data.metadata,
                    });
                }
            }
        }

        Ok(instances)
    }

    async fn get_all_services(&self) -> Result<Vec<ServiceInstance>, Error> {
        let services = self.zk.get_children(&self.base_path, false)
            .await
            .map_err(|e| Error::ServiceDiscovery(format!("Failed to get services: {}", e)))?;

        let mut instances = Vec::new();
        for service in services {
            instances.extend(self.get_instances(&service).await?);
        }

        Ok(instances)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_instance() -> ServiceInstance {
        let mut metadata = HashMap::new();
        metadata.insert("version".to_string(), "1.0".to_string());
        
        ServiceInstance {
            name: "test-service".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            metadata,
        }
    }

    #[test]
    fn test_instance_to_path() {
        let instance = create_test_instance();
        let path = format!("{}/{}", "/services", instance.name);
        assert_eq!(path, "/services/test-service");
    }

    #[test]
    fn test_instance_serialization() {
        let instance = create_test_instance();
        let data = ServiceData {
            name: instance.name.clone(),
            host: instance.host.clone(),
            port: instance.port,
            metadata: instance.metadata.clone(),
        };
        
        let json = serde_json::to_vec(&data).unwrap();
        let parsed: ServiceData = serde_json::from_slice(&json).unwrap();
        
        assert_eq!(parsed.name, "test-service");
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.metadata.get("version").unwrap(), "1.0");
    }

    #[test]
    fn test_invalid_data_deserialization() {
        let invalid_data = b"invalid json data";
        let result = serde_json::from_slice::<ServiceData>(invalid_data);
        assert!(result.is_err());
    }
} 