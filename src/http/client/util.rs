use std::collections::VecDeque;

use async_stream::stream;
use futures::StreamExt;
use hyper::{
    body::to_bytes, header::CONTENT_TYPE, Body, Method, Request as HttpRequest,
    Response as HttpResponse, Uri,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::{
    error::ProtocolErrorType,
    http::{
        generic_error, HttpNotificationPayload, ModalHttpResponse, ResponseHttpConvert,
        SSE_DATA_PREFIX,
    },
    service::{NotificationStream, ServiceError, ServiceResponse},
    ProtocolError,
};

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
