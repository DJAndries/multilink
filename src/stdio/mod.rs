use serde::Serialize;
use serde_json::Value;

use crate::{
    error::SerializableProtocolError,
    jsonrpc::{JsonRpcMessage, JsonRpcRequest},
};

#[cfg(feature = "stdio-client")]
pub mod client;

#[cfg(feature = "stdio-server")]
pub mod server;

pub trait RequestJsonRpcConvert<Request> {
    fn from_jsonrpc_request(
        value: JsonRpcRequest,
    ) -> Result<Option<Request>, SerializableProtocolError>;

    fn into_jsonrpc_request(&self) -> JsonRpcRequest;
}

pub trait ResponseJsonRpcConvert<Request, Response> {
    fn from_jsonrpc_message(
        value: JsonRpcMessage,
        original_request: &Request,
    ) -> Result<Option<Response>, SerializableProtocolError>;

    fn into_jsonrpc_message(response: Response, id: Value) -> JsonRpcMessage;
}

fn serialize_payload<R: Serialize>(payload: &R) -> String {
    let mut serialized = serde_json::to_string(payload).unwrap();
    serialized.push_str("\n");
    serialized
}
