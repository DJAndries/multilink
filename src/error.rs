use std::error::Error;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProtocolErrorType {
    NotFound,
    HttpMethodNotAllowed,
    BadRequest,
    Unauthorized,
    Internal,
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct ProtocolError {
    pub error_type: ProtocolErrorType,
    #[source]
    pub error: Box<dyn Error + Send + Sync + 'static>,
}

impl ProtocolError {
    pub fn new(
        error_type: ProtocolErrorType,
        error: Box<dyn Error + Send + Sync + 'static>,
    ) -> Self {
        Self { error_type, error }
    }
}

impl From<Box<dyn Error + Send + Sync + 'static>> for ProtocolError {
    fn from(error: Box<dyn Error + Send + Sync + 'static>) -> Self {
        match error.downcast::<Self>() {
            Ok(e) => *e,
            Err(e) => ProtocolError::new(ProtocolErrorType::Internal, e),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error, Serialize, Deserialize)]
#[error("{description}")]
pub struct SerializableProtocolError {
    pub error_type: ProtocolErrorType,
    pub description: String,
}

impl From<ProtocolError> for SerializableProtocolError {
    fn from(value: ProtocolError) -> Self {
        Self {
            error_type: value.error_type,
            description: value.error.to_string(),
        }
    }
}

impl From<SerializableProtocolError> for ProtocolError {
    fn from(value: SerializableProtocolError) -> Self {
        Self {
            error_type: value.error_type.clone(),
            error: Box::new(value),
        }
    }
}
