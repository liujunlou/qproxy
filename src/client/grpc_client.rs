
// 引入生成的 proto 代码
// mod route {
//     include!(concat!(env!("OUT_DIR"), "/route.rs"));
// }

// use route::route_service_client::RouteServiceClient;
// use route::RouteMessage;
// use route::RouteResponse;
use tonic::transport::Channel;
use tonic::Request;
use tonic::Response;
use tonic::Code;
use crate::errors::Error;
use crate::playback::route::route_service_client::RouteServiceClient;
use crate::playback::route::RouteMessage;
use crate::playback::route::RouteResponse;

pub struct GrpcClient {
    client: RouteServiceClient<Channel>,
}

impl GrpcClient {
    pub async fn new(addr: &str) -> Result<Self, Error> {
        let route_service_client = match RouteServiceClient::connect(addr.to_string()).await {
            Ok(client) => client,
            Err(e) => return Err(Error::Grpc(e)),
        };
        Ok(Self {
            client: route_service_client,
        })
    }

    pub async fn call(&mut self, request: Request<RouteMessage>) -> Result<Response<RouteResponse>, Error> {
        match self.client.send_message(request).await {
            Ok(response) => Ok(response),
            Err(status) => {
                if status.code() == Code::Ok {
                    Ok(Response::new(RouteResponse {
                        message_id: "".to_string(),
                        status_code: 200,
                        status_message: "OK".to_string(),
                        payload: vec![],
                    }))
                } else {
                    Err(Error::GrpcStatus(status.message().to_string()))
                }
            }
        }
    }
}
