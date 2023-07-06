use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{ProtocolErrorType, SerializableProtocolError};

pub fn parse_from_value<R: DeserializeOwned>(value: Value) -> Result<R, SerializableProtocolError> {
    serde_json::from_value::<R>(value).map_err(|error| SerializableProtocolError {
        error_type: ProtocolErrorType::BadRequest,
        description: error.to_string(),
    })
}

#[cfg(all(feature = "http-client", feature = "stdio-client"))]
pub mod service {
    use crate::{
        http::{
            client::{HttpClient, HttpClientConfig},
            RequestHttpConvert, ResponseHttpConvert,
        },
        stdio::{
            client::{StdioClient, StdioClientConfig},
            RequestJsonRpcConvert, ResponseJsonRpcConvert,
        },
        BoxedService, ServiceError,
    };

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
