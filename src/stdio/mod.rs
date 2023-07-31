use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::{
    error::ProtocolErrorType,
    jsonrpc::{JsonRpcMessage, JsonRpcRequest},
    ProtocolError,
};

#[cfg(feature = "stdio-client")]
pub mod client;

#[cfg(feature = "stdio-server")]
pub mod server;

/// Errors that are specific to stdio communication.
#[derive(Debug, Error)]
pub enum StdioError {
    #[error("unable to send stdio request to comm task")]
    SendRequestCommTask,
    #[error("request timed out")]
    Timeout,
    #[error("unable to recv response for stdio request from comm task")]
    RecvResponseCommTask,
    #[error("client does not support serving request")]
    ClientRequestUnsupported,
}

impl Into<ProtocolError> for StdioError {
    fn into(self) -> ProtocolError {
        let error_type = match &self {
            StdioError::SendRequestCommTask => ProtocolErrorType::Internal,
            StdioError::Timeout => ProtocolErrorType::Internal,
            StdioError::RecvResponseCommTask => ProtocolErrorType::Internal,
            StdioError::ClientRequestUnsupported => ProtocolErrorType::BadRequest,
        };
        ProtocolError {
            error_type,
            error: Box::new(self),
        }
    }
}

/// A request that can convert to and from a [`JsonRpcRequest`].
pub trait RequestJsonRpcConvert<Request> {
    /// Deserializes a [`JsonRpcRequest`] into `Request`. Returns a protocol error
    /// if the request conversion fails (i.e. request validation fails,
    /// unexpected error, etc.). Returns `None` if the request type is unknown or unsupported,
    /// which is synonymous with a "not found" error.
    fn from_jsonrpc_request(value: JsonRpcRequest) -> Result<Option<Request>, ProtocolError>;

    /// Serializes a `Request` into a [`JsonRpcRequest`].
    fn into_jsonrpc_request(&self) -> JsonRpcRequest;
}

/// A response that can convert to and from a [`JsonRpcResponse`](crate::jsonrpc::JsonRpcResponse)
/// or [`JsonRpcNotification`](crate::jsonrpc::JsonRpcNotification).
pub trait ResponseJsonRpcConvert<Request, Response> {
    /// Deserializes a [`JsonRpcResponse`](crate::jsonrpc::JsonRpcResponse) or
    /// [`JsonRpcNotification`](crate::jsonrpc::JsonRpcNotification) into `Response`.
    /// Returns a protocol error if the response conversion fails (i.e.
    /// response validation fails, unexpected error, etc.). A reference to the associated
    /// request is provided, in case it's helpful. Returns `None` if the response type is unknown or unsupported,
    /// which is synonymous with a "not found" error.
    fn from_jsonrpc_message(
        value: JsonRpcMessage,
        original_request: &Request,
    ) -> Result<Option<Response>, ProtocolError>;

    /// Serializes a `Response` into a [`JsonRpcResponse`](crate::jsonrpc::JsonRpcResponse) or
    /// [`JsonRpcNotification`](crate::jsonrpc::JsonRpcNotification).
    /// Notifications must use the provided `id` argument as the `method` value.
    /// Returns [`Value::Null`]
    fn into_jsonrpc_message(response: Response, id: Value) -> JsonRpcMessage;
}

fn serialize_payload<R: Serialize>(payload: &R) -> String {
    let mut serialized = serde_json::to_string(payload).unwrap();
    serialized.push_str("\n");
    serialized
}
