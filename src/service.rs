use std::{error::Error, pin::Pin};

use futures::{Future, Stream};
use tower::Service;

use crate::ProtocolError;

pub type NotificationStream<Response> =
    Pin<Box<dyn Stream<Item = Result<Response, ProtocolError>> + Send>>;

pub enum ServiceResponse<Response> {
    Single(Response),
    Multiple(NotificationStream<Response>),
}

pub type ServiceError = Box<dyn Error + Send + Sync + 'static>;
pub type ServiceFuture<Response> =
    Pin<Box<dyn Future<Output = Result<Response, ServiceError>> + Send>>;
pub type BoxedService<Request, Response> = Box<
    dyn Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Sync,
>;

#[cfg(all(feature = "http-client", feature = "stdio-client"))]
pub mod util {
    use crate::{
        http::{
            client::{HttpClient, HttpClientConfig},
            RequestHttpConvert, ResponseHttpConvert,
        },
        stdio::{
            client::{StdioClient, StdioClientConfig},
            RequestJsonRpcConvert, ResponseJsonRpcConvert,
        },
    };

    use super::{BoxedService, ServiceError};

    pub async fn build_service_from_config<Request, Response>(
        command_name: &str,
        command_arguments: &[&str],
        stdio_client_config: Option<StdioClientConfig>,
        http_client_config: Option<HttpClientConfig>,
    ) -> Result<BoxedService<Request, Response>, ServiceError>
    where
        Request: RequestHttpConvert<Request>
            + RequestJsonRpcConvert<Request>
            + Clone
            + Send
            + Sync
            + 'static,
        Response: ResponseHttpConvert<Request, Response>
            + ResponseJsonRpcConvert<Request, Response>
            + Send
            + Sync
            + 'static,
    {
        Ok(match http_client_config {
            Some(config) => Box::new(HttpClient::new(config)?),
            None => Box::new(
                StdioClient::new(
                    command_name,
                    command_arguments,
                    stdio_client_config.unwrap_or_default(),
                )
                .await?,
            ),
        })
    }
}
