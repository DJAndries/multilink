use std::collections::VecDeque;

use async_stream::stream;
use futures::StreamExt;
use hyper::{
    body::to_bytes, header::CONTENT_TYPE, Body, Method, Request as HttpRequest,
    Response as HttpResponse, StatusCode, Uri,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::{
    error::ProtocolErrorType,
    http::{
        generic_error, HttpNotificationPayload, ModalHttpResponse, ResponseHttpConvert,
        SSE_DATA_PREFIX,
    },
    NotificationStream, ProtocolError, ServiceError, ServiceResponse,
};

/// Deserializes the body of [`HttpResponse<Body>`] into `T`.
/// Returns a "bad request" error if JSON deserialization fails,
/// and returns an "internal" error if raw data retrieval from the request fails.
/// Can be useful for implementing [`ResponseHttpConvert::from_http_response`].
pub async fn parse_response<T: DeserializeOwned>(
    response: HttpResponse<Body>,
) -> Result<T, ProtocolError> {
    let bytes = to_bytes(response)
        .await
        .map_err(|e| ProtocolError::new(ProtocolErrorType::Internal, Box::new(e)))?;
    parse_response_payload(bytes.as_ref())
}

fn parse_response_payload<T: DeserializeOwned>(response: &[u8]) -> Result<T, ProtocolError> {
    serde_json::from_slice(response)
        .map_err(|e| ProtocolError::new(ProtocolErrorType::BadRequest, Box::new(e)))
}

/// Serializes `T` into [`HttpRequest<Body>`]. Returns an "internal" error if
/// JSON serialization fails. Can be useful for
/// implementing [`RequestHttpConvert::to_http_request`](crate::http::RequestHttpConvert::to_http_request).
pub fn serialize_to_http_request<T: Serialize>(
    base_url: &Uri,
    path: &str,
    method: Method,
    request: &T,
) -> Result<HttpRequest<Body>, ProtocolError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|e| ProtocolError::new(ProtocolErrorType::Internal, Box::new(e)))?;
    let url = Uri::builder()
        .scheme(
            base_url
                .scheme()
                .expect("base url should contain scheme")
                .clone(),
        )
        .authority(
            base_url
                .authority()
                .expect("base url should contain authority")
                .clone(),
        )
        .path_and_query(path)
        .build()
        .expect("should be able to build url");
    Ok(HttpRequest::builder()
        .method(method)
        .uri(url)
        .header(CONTENT_TYPE, "application/json")
        .body(bytes.into())
        .expect("should be able to create http request"))
}

/// Converts an [`HttpResponse<Body>`] to a [`NotificationStream<Response>`] so
/// server-side events can be consumed by the HTTP client. Can be useful for implementing
/// [`ResponseHttpConvert::from_http_response`].
pub fn notification_sse_stream<Request, Response>(
    original_request: Request,
    http_response: HttpResponse<Body>,
) -> NotificationStream<Response>
where
    Request: Clone + Send + Sync + 'static,
    Response: ResponseHttpConvert<Request, Response> + Send + Sync + 'static,
{
    let mut body = http_response.into_body();
    stream! {
        let mut buffer = VecDeque::new();
        while let Some(bytes_result) = body.next().await {
            match bytes_result {
                Err(e) => {
                    let boxed_e: ServiceError = Box::new(e);
                    yield Err(boxed_e.into());
                    return;
                },
                Ok(bytes) => {
                    buffer.extend(bytes);
                }
            }
            while let Some(linebreak_pos) = buffer.iter().position(|b| b == &b'\n') {
                let line_bytes = buffer.drain(0..(linebreak_pos + 1)).collect::<Vec<_>>();
                if let Ok(line) = std::str::from_utf8(&line_bytes) {
                    if !line.starts_with(SSE_DATA_PREFIX) {
                        continue;
                    }
                    if let Ok(payload) = serde_json::from_str::<HttpNotificationPayload>(&line[SSE_DATA_PREFIX.len()..]) {
                        let result: Result<Value, ProtocolError> = payload.into();
                        match result {
                            Err(e) => yield Err(e),
                            Ok(value) => {
                                yield Response::from_http_response(ModalHttpResponse::Event(value), &original_request).await
                                    .and_then(|response| response.ok_or_else(|| generic_error(ProtocolErrorType::NotFound)))
                                    .and_then(|response| match response {
                                        ServiceResponse::Single(response) => Ok(response),
                                        _ => Err(generic_error(ProtocolErrorType::NotFound))
                                    });
                            }
                        }
                    }
                }
            }
        }
    }.boxed()
}

/// Deserializes the body of [`HttpRequest<Body>`] into `T`.
/// Returns a "bad request" error if JSON deserialization fails,
/// and returns an "internal" error if raw data retrieval from the request fails.
/// Can be useful for implementing [`RequestHttpConvert::from_http_request`](crate::http::RequestHttpConvert::from_http_request).
pub async fn parse_request<T: DeserializeOwned>(
    request: HttpRequest<Body>,
) -> Result<T, ProtocolError> {
    let bytes = to_bytes(request)
        .await
        .map_err(|e| ProtocolError::new(ProtocolErrorType::Internal, Box::new(e)))?;
    serde_json::from_slice(bytes.as_ref())
        .map_err(|e| ProtocolError::new(ProtocolErrorType::BadRequest, Box::new(e)))
}

/// Compares the request method with an expected method and returns
/// [`ProtocolErrorType::HttpMethodNotAllowed`] if there is a mismatch.
/// Can be useful for implementing [`RequestHttpConvert::from_http_request`](crate::http::RequestHttpConvert::from_http_request).
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

/// Serializes `T` into [`HttpResponse<Body>`]. Returns an "internal" error if
/// JSON serialization fails. Can be useful for
/// implementing [`ResponseHttpConvert::to_http_response`].
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

/// Converts a [`NotificationStream<Response>`] to an [`HttpResponse<Body>`] so
/// server-side events can be produced by the HTTP server. Can be useful for implementing
/// [`ResponseHttpConvert::to_http_response`].
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
