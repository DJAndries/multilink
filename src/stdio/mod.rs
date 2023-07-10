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

pub trait RequestJsonRpcConvert<Request> {
    fn from_jsonrpc_request(value: JsonRpcRequest) -> Result<Option<Request>, ProtocolError>;

    fn into_jsonrpc_request(&self) -> JsonRpcRequest;
}

pub trait ResponseJsonRpcConvert<Request, Response> {
    fn from_jsonrpc_message(
        value: JsonRpcMessage,
        original_request: &Request,
    ) -> Result<Option<Response>, ProtocolError>;

    fn into_jsonrpc_message(response: Response, id: Value) -> JsonRpcMessage;
}

fn serialize_payload<R: Serialize>(payload: &R) -> String {
    let mut serialized = serde_json::to_string(payload).unwrap();
    serialized.push_str("\n");
    serialized
}
