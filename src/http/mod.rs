pub use hyper;

use hyper::{Body, StatusCode, Uri};
pub use hyper::{Request as HttpRequest, Response as HttpResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    error::{ProtocolErrorType, SerializableProtocolError},
    service::ServiceResponse,
    ProtocolError,
};

#[cfg(any(feature = "http-client"))]
pub mod client;
#[cfg(any(feature = "http-server"))]
pub mod server;

const API_KEY_HEADER: &str = "X-API-Key";
const SSE_DATA_PREFIX: &str = "data: ";

#[derive(Debug, Error, Serialize, Deserialize)]
#[error("{error}")]
pub struct ProtocolHttpError {
    pub error: String,
}

pub enum ModalHttpResponse {
    Single(HttpResponse<Body>),
    Event(Value),
}

#[async_trait::async_trait]
pub trait RequestHttpConvert<Request> {
    async fn from_http_request(
        request: HttpRequest<Body>,
    ) -> Result<Option<Request>, ProtocolError>;

    fn to_http_request(&self, base_url: &Uri) -> Result<Option<HttpRequest<Body>>, ProtocolError>;
}

#[async_trait::async_trait]
pub trait ResponseHttpConvert<Request, Response>
where
    Request: Clone,
    Response: ResponseHttpConvert<Request, Response>,
{
    async fn from_http_response(
        response: ModalHttpResponse,
        original_request: &Request,
    ) -> Result<Option<ServiceResponse<Response>>, ProtocolError>;

    fn to_http_response(
        response: ServiceResponse<Response>,
    ) -> Result<Option<ModalHttpResponse>, ProtocolError>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpNotificationPayload {
    pub result: Option<Value>,
    pub error: Option<SerializableProtocolError>,
}

pub fn generic_error(error_type: ProtocolErrorType) -> ProtocolError {
    let status: StatusCode = error_type.clone().into();
    let error = Box::new(ProtocolHttpError {
        error: status.to_string(),
    });
    ProtocolError { error_type, error }
}
