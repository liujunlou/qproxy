// 引入生成的 proto 代码
// mod route {
//     include!(concat!(env!("OUT_DIR"), "/route.rs"));
// }

use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::Lazy;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;
// use route::route_service_client::RouteServiceClient;
// use route::RouteMessage;
// use route::RouteResponse;
use crate::errors::Error;
use crate::playback::route::route_service_client::RouteServiceClient;
use crate::playback::route::RouteMessage;
use crate::playback::route::RouteResponse;
use tonic::transport::Channel;
use tonic::Code;
use tonic::Request;
use tonic::Response;

pub static GRPC_CLIENT_POOL: Lazy<Arc<RwLock<HashMap<String, GrpcClient>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

pub async fn get_grpc_client(addr: &str) -> Result<GrpcClient, Error> {
    {
        let pool = GRPC_CLIENT_POOL.read().await;
        if let Some(client) = pool.get(addr) {
            return Ok((*client).clone());
        }
    }

    GRPC_CLIENT_POOL
        .write()
        .await
        .insert(addr.to_string(), GrpcClient::new(addr).await?);

    let pool = GRPC_CLIENT_POOL.read().await;
    match pool.get(addr) {
        Some(client) => Ok((*client).clone()),
        None => Err(Error::GrpcStatus("Failed to get client".to_string())),
    }
}

#[derive(Clone, Debug)]
pub struct GrpcClient {
    client: RouteServiceClient<Channel>,
}

impl GrpcClient {
    pub async fn new(addr: &str) -> Result<Self, Error> {
        let route_service_client = match RouteServiceClient::connect(addr.to_string()).await {
            Ok(client) => client,
            Err(e) => return Err(Error::Grpc(e)),
        };
        info!("Create grpc client, addr: {}", addr);
        Ok(Self {
            client: route_service_client,
        })
    }

    pub async fn call(
        &mut self,
        request: Request<RouteMessage>,
    ) -> Result<Response<RouteResponse>, Error> {
        info!("Call grpc client, request: {:?}", request);
        match self.client.send_message(request).await {
            Ok(response) => Ok(response),
            Err(status) => {
                if status.code() == Code::Ok {
                    Ok(Response::new(RouteResponse {
                        message_id: None,
                        status_code: Some(200),
                        status_message: Some("OK".to_string()),
                        payload: None,
                    }))
                } else {
                    error!("Call grpc client failed, status: {:?}", status);
                    Err(Error::GrpcStatus(status.message().to_string()))
                }
            }
        }
    }
}
