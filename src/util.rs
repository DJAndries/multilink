#[cfg(any(feature = "stdio-server", feature = "stdio-client"))]
use serde::de::DeserializeOwned;
#[cfg(any(feature = "stdio-server", feature = "stdio-client"))]
use serde_json::Value;

#[cfg(any(feature = "stdio-server", feature = "stdio-client"))]
use crate::error::{ProtocolErrorType, SerializableProtocolError};

/// Parses/deserializes a [`serde_json::Value`] into `R`. Returns
/// a "bad request" protocol error if deserialization fails. Can be useful for
/// parsing events when implementing [`ResponseJsonRpcConvert::from_jsonrpc_message`](crate::stdio::ResponseJsonRpcConvert::from_jsonrpc_message).
#[cfg(any(feature = "stdio-server", feature = "stdio-client"))]
pub fn parse_from_value<R: DeserializeOwned>(value: Value) -> Result<R, SerializableProtocolError> {
    serde_json::from_value::<R>(value).map_err(|error| SerializableProtocolError {
        error_type: ProtocolErrorType::BadRequest,
        description: error.to_string(),
    })
}

/// Utility functions related to services.
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

    /// Creates a [`StdioClient`](crate::stdio::client::StdioClient) or
    /// [`HttpClient`](crate::http::client::HttpClient) service depending
    /// on the arguments provided. If `http_client_config` is `Some`, an
    /// HTTP-based service will be created. If it is `None`, a stdio-based service
    /// will be created.
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
