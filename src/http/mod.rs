pub use hyper;

use hyper::{Body, StatusCode, Uri};
pub use hyper::{Request as HttpRequest, Response as HttpResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    error::{ProtocolErrorType, SerializableProtocolError},
    ProtocolError, ServiceResponse,
};

/// HTTP client components.
#[cfg(any(feature = "http-client"))]
pub mod client;
/// HTTP server components
#[cfg(any(feature = "http-server"))]
pub mod server;
/// HTTP utilities for request/response conversion.
pub mod util;

const API_KEY_HEADER: &str = "X-API-Key";
const SSE_DATA_PREFIX: &str = "data: ";

/// Body for an HTTP error response.
#[derive(Debug, Error, Serialize, Deserialize)]
#[error("{error}")]
pub struct ProtocolHttpError {
    pub error: String,
}

impl Into<StatusCode> for ProtocolErrorType {
    fn into(self) -> StatusCode {
        match self {
            ProtocolErrorType::BadRequest => StatusCode::BAD_REQUEST,
            ProtocolErrorType::Unauthorized => StatusCode::UNAUTHORIZED,
            ProtocolErrorType::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ProtocolErrorType::NotFound => StatusCode::NOT_FOUND,
            ProtocolErrorType::HttpMethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
        }
    }
}

impl From<StatusCode> for ProtocolErrorType {
    fn from(code: StatusCode) -> Self {
        match code {
            StatusCode::BAD_REQUEST => ProtocolErrorType::BadRequest,
            StatusCode::UNAUTHORIZED => ProtocolErrorType::Unauthorized,
            StatusCode::INTERNAL_SERVER_ERROR => ProtocolErrorType::Internal,
            StatusCode::NOT_FOUND => ProtocolErrorType::NotFound,
            StatusCode::METHOD_NOT_ALLOWED => ProtocolErrorType::HttpMethodNotAllowed,
            _ => ProtocolErrorType::Internal,
        }
    }
}

/// A multilink HTTP response.
pub enum ModalHttpResponse {
    /// Contains a single HTTP response returned by the server.
    Single(HttpResponse<Body>),
    /// Contains a single serializable event returned by the server,
    /// as part of a stream.
    Event(Value),
}

/// A request that can convert to and from a [`HttpRequest<Body>`].
#[async_trait::async_trait]
pub trait RequestHttpConvert<Request> {
    /// Deserializes a [`HttpRequest<Body>`] into `Request`. Returns a protocol error
    /// if the request conversion fails (i.e. request validation fails,
    /// unexpected error, etc.). Returns `None` if the request type is unknown or unsupported for remote host scenarios,
    /// which is synonymous with a "not found" error.
    async fn from_http_request(
        request: HttpRequest<Body>,
    ) -> Result<Option<Request>, ProtocolError>;

    /// Serializes a `Request` into a [`HttpRequest<Body>`]. Returns `None` if
    /// the request is unsupported for this protocol, which is synonymous with a
    /// "not found" error.
    fn to_http_request(&self, base_url: &Uri) -> Result<Option<HttpRequest<Body>>, ProtocolError>;
}

/// A response that can convert to and from a [`ModalHttpResponse`].
#[async_trait::async_trait]
pub trait ResponseHttpConvert<Request, Response>
where
    Request: Clone,
    Response: ResponseHttpConvert<Request, Response>,
{
    /// Deserializes a [`ModalHttpResponse`] into `ServiceResponse<Response>`.
    /// Returns a protocol error if the response conversion fails (i.e.
    /// response validation fails, unexpected error, etc.). A reference to the associated
    /// request is provided, in case it's helpful. Returns `None` if the response type is unknown or unsupported
    /// for remote host scenarios, which is synonymous with a "not found" error.
    async fn from_http_response(
        response: ModalHttpResponse,
        original_request: &Request,
    ) -> Result<Option<ServiceResponse<Response>>, ProtocolError>;

    /// Serializes a `Response` into a [`ModalHttpResponse`].
    /// Returns `None` if the response type is unsupported, which is synonymous
    /// with a "not found" error.
    fn to_http_response(
        response: ServiceResponse<Response>,
    ) -> Result<Option<ModalHttpResponse>, ProtocolError>;
}

/// The JSON payload for a server-side event/notification.
#[derive(Debug, Serialize, Deserialize)]
pub struct HttpNotificationPayload {
    pub result: Option<Value>,
    pub error: Option<SerializableProtocolError>,
}

impl From<Result<Option<Value>, ProtocolError>> for HttpNotificationPayload {
    fn from(result: Result<Option<Value>, ProtocolError>) -> Self {
        let result =
            result.and_then(|r| r.ok_or_else(|| generic_error(ProtocolErrorType::NotFound)));
        let (result, error) = match result {
            Ok(result) => (Some(result), None),
            Err(e) => (None, Some(e.into())),
        };
        Self { result, error }
    }
}

impl Into<Result<Value, ProtocolError>> for HttpNotificationPayload {
    fn into(self) -> Result<Value, ProtocolError> {
        if let Some(e) = self.error {
            return Err(e.into());
        }
        self.result
            .ok_or_else(|| generic_error(ProtocolErrorType::NotFound))
    }
}

/// Creates a generic [`ProtocolError`] using the HTTP status code
/// description (i.e. "Bad Request" or "Not Found") as the error text.
pub fn generic_error(error_type: ProtocolErrorType) -> ProtocolError {
    let status: StatusCode = error_type.clone().into();
    let error = Box::new(ProtocolHttpError {
        error: status.to_string(),
    });
    ProtocolError { error_type, error }
}
