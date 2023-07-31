use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ProtocolErrorType, SerializableProtocolError};
use crate::ProtocolError;

/// The id field name used by the request and response.
pub const ID_KEY: &str = "id";
/// The method field name used by the request and notification.
pub const METHOD_KEY: &str = "method";
/// The version of JSON-RPC used by this crate.
pub const JSON_RPC_VERSION: &str = "2.0";

/// Data structure for a JSON-RPC request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    pub jsonrpc_version: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Value,
}

/// Data structure for a JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(rename = "jsonrpc")]
    pub jsonrpc_version: String,
    pub result: Option<Value>,
    pub error: Option<JsonRpcResponseError>,
    pub id: Value,
}

/// Data structure for a JSON-RPC notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    #[serde(rename = "jsonrpc")]
    pub jsonrpc_version: String,
    pub method: String,
    pub params: Option<Value>,
}

/// Parameters used to return a result and error
/// for a notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotificationResultParams {
    pub result: Option<Value>,
    pub error: Option<JsonRpcResponseError>,
}

/// Data structure for the error in a JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponseError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// A subset of JSON-RPC error codes.
#[derive(Clone, PartialEq, Debug)]
#[repr(i32)]
pub enum JsonRpcErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
}

impl From<i32> for JsonRpcErrorCode {
    fn from(value: i32) -> Self {
        match value {
            -32700 => Self::ParseError,
            -32600 => Self::InvalidRequest,
            -32601 => Self::MethodNotFound,
            -32602 => Self::InvalidParams,
            -32603 => Self::InternalError,
            _ => Self::InternalError,
        }
    }
}

impl From<ProtocolErrorType> for JsonRpcErrorCode {
    fn from(value: ProtocolErrorType) -> Self {
        match value {
            ProtocolErrorType::BadRequest => JsonRpcErrorCode::InvalidRequest,
            ProtocolErrorType::Unauthorized => JsonRpcErrorCode::InvalidRequest,
            ProtocolErrorType::Internal => JsonRpcErrorCode::InternalError,
            _ => JsonRpcErrorCode::InternalError,
        }
    }
}

impl Into<ProtocolErrorType> for JsonRpcErrorCode {
    fn into(self) -> ProtocolErrorType {
        match self {
            Self::ParseError => ProtocolErrorType::BadRequest,
            Self::InvalidRequest => ProtocolErrorType::BadRequest,
            Self::MethodNotFound => ProtocolErrorType::BadRequest,
            Self::InvalidParams => ProtocolErrorType::BadRequest,
            Self::InternalError => ProtocolErrorType::Internal,
        }
    }
}

/// All supported types of JSON-RPC messages.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

impl JsonRpcRequest {
    pub fn new(method: String, params: Option<Value>) -> Self {
        Self {
            jsonrpc_version: JSON_RPC_VERSION.to_string(),
            method,
            params,
            id: Value::Null,
        }
    }

    /// Parses request parameters into `R`. Returns a "bad request" protocol error,
    /// if deserialization fails.
    pub fn parse_params<R: DeserializeOwned>(self) -> Result<R, SerializableProtocolError> {
        let params = self.params.ok_or_else(|| SerializableProtocolError {
            error_type: ProtocolErrorType::BadRequest,
            description: "missing parameters".to_string(),
        })?;

        serde_json::from_value::<R>(params).map_err(|error| SerializableProtocolError {
            error_type: ProtocolErrorType::BadRequest,
            description: error.to_string(),
        })
    }
}

fn get_result_and_error(
    result: Result<Value, ProtocolError>,
) -> (Option<Value>, Option<JsonRpcResponseError>) {
    match result {
        Ok(result) => (Some(result), None),
        Err(e) => (
            None,
            Some(JsonRpcResponseError {
                code: JsonRpcErrorCode::from(e.error_type.clone()) as i32,
                message: e.to_string(),
                data: None,
            }),
        ),
    }
}

impl JsonRpcResponse {
    pub fn new(result: Result<Value, ProtocolError>, id: Value) -> Self {
        let (result, error) = get_result_and_error(result);
        JsonRpcResponse {
            jsonrpc_version: JSON_RPC_VERSION.to_string(),
            result,
            error,
            id: id.into(),
        }
    }

    /// Evaluates `result` and `error` from the response and returns
    /// a `Result`.
    pub fn get_result(self) -> Result<Value, SerializableProtocolError> {
        if let Some(error) = self.error {
            let jsonrpc_error_type = JsonRpcErrorCode::from(error.code);
            return Err(SerializableProtocolError {
                error_type: jsonrpc_error_type.into(),
                description: error.message,
            });
        }
        Ok(self.result.unwrap_or(Value::Null))
    }
}

impl JsonRpcNotification {
    pub fn new(method: String, params: Option<Value>) -> Self {
        JsonRpcNotification {
            jsonrpc_version: JSON_RPC_VERSION.to_string(),
            method,
            params,
        }
    }

    /// Creates a notification for a given result.
    /// The `result` argument will be serialized into [`JsonRpcNotificationResultParams`],
    /// which will be used as the value for the `params` field in the notification.
    pub fn new_with_result_params(result: Result<Value, ProtocolError>, method: String) -> Self {
        JsonRpcNotification {
            jsonrpc_version: JSON_RPC_VERSION.to_string(),
            method,
            params: serde_json::to_value(JsonRpcNotificationResultParams::new(result)).ok(),
        }
    }

    /// Retrieves a `Result` from a given notification.
    /// The `params` notification value must be a [`JsonRpcNotificationResultParams`].
    /// Returns [`Value::Null`] if the result cannot be deserialized, or if the value is not present.
    pub fn get_result(self) -> Result<Value, SerializableProtocolError> {
        let params: JsonRpcNotificationResultParams = serde_json::from_value(
            self.params.unwrap_or(Value::Null),
        )
        .unwrap_or(JsonRpcNotificationResultParams {
            result: Some(Value::Null),
            error: None,
        });
        if let Some(error) = params.error {
            let jsonrpc_error_type = JsonRpcErrorCode::from(error.code);
            return Err(SerializableProtocolError {
                error_type: jsonrpc_error_type.into(),
                description: error.message,
            });
        }
        Ok(params.result.unwrap_or(Value::Null))
    }
}

impl JsonRpcNotificationResultParams {
    pub fn new(result: Result<Value, ProtocolError>) -> Self {
        let (result, error) = get_result_and_error(result);
        Self { result, error }
    }
}

impl From<JsonRpcRequest> for JsonRpcMessage {
    fn from(value: JsonRpcRequest) -> Self {
        Self::Request(value)
    }
}

impl From<JsonRpcResponse> for JsonRpcMessage {
    fn from(value: JsonRpcResponse) -> Self {
        Self::Response(value)
    }
}

impl From<JsonRpcNotification> for JsonRpcMessage {
    fn from(value: JsonRpcNotification) -> Self {
        Self::Notification(value)
    }
}

impl TryFrom<serde_json::Value> for JsonRpcMessage {
    type Error = serde_json::Error;

    fn try_from(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        Ok(match value.get(METHOD_KEY).is_some() {
            true => match value.get(ID_KEY).is_some() {
                true => JsonRpcMessage::Request(serde_json::from_value(value)?),
                false => JsonRpcMessage::Notification(serde_json::from_value(value)?),
            },
            false => JsonRpcMessage::Response(serde_json::from_value(value)?),
        })
    }
}
