use futures::StreamExt;
use hyper::{
    body::to_bytes, header::CONTENT_TYPE, Body, Method, Request as HttpRequest,
    Response as HttpResponse, StatusCode,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::ProtocolErrorType,
    http::{generic_error, HttpNotificationPayload, ModalHttpResponse, ResponseHttpConvert},
    NotificationStream, ProtocolError, ServiceResponse,
};

pub async fn parse_request<T: DeserializeOwned>(
    request: HttpRequest<Body>,
) -> Result<T, ProtocolError> {
    let bytes = to_bytes(request)
        .await
        .map_err(|e| ProtocolError::new(ProtocolErrorType::Internal, Box::new(e)))?;
    serde_json::from_slice(bytes.as_ref())
        .map_err(|e| ProtocolError::new(ProtocolErrorType::BadRequest, Box::new(e)))
}

pub fn validate_method(
    request: &HttpRequest<Body>,
    expected_method: Method,
) -> Result<(), ProtocolError> {
    match request.method() == &expected_method {
        true => Ok(()),
        false => Err(generic_error(ProtocolErrorType::HttpMethodNotAllowed).into()),
    }
}

fn serialize_response<T: Serialize>(response: &T) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(response)
        .map_err(|e| ProtocolError::new(ProtocolErrorType::Internal, Box::new(e)))
}

pub fn serialize_to_http_response<T: Serialize>(
    response: &T,
    status: StatusCode,
) -> Result<HttpResponse<Body>, ProtocolError> {
    let bytes = serialize_response(response)?;
    Ok(HttpResponse::builder()
        .header(CONTENT_TYPE, "application/json")
        .status(status)
        .body(bytes.into())
        .expect("should be able to create http response"))
}

pub fn notification_sse_response<Request, Response>(
    notification_stream: NotificationStream<Response>,
) -> HttpResponse<Body>
where
    Request: Clone,
    Response: ResponseHttpConvert<Request, Response> + 'static,
{
    let payload_stream = notification_stream.map(|result| {
        let payload = HttpNotificationPayload::from(result.and_then(|response| {
            Response::to_http_response(ServiceResponse::Single(response)).map(|opt| {
                opt.and_then(|response| match response {
                    ModalHttpResponse::Event(value) => Some(value),
                    _ => None,
                })
            })
        }));
        let payload_str = serde_json::to_string(&payload)?;
        Ok::<String, serde_json::Error>(format!("data: {}\n\n", payload_str))
    });
    HttpResponse::new(Body::wrap_stream(payload_stream))
}
