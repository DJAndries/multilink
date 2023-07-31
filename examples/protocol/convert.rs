use async_trait::async_trait;
use hyper::{Body, Method, StatusCode};
use multilink::{
    http::{
        util::{
            notification_sse_response, notification_sse_stream, parse_request, parse_response,
            serialize_to_http_request, serialize_to_http_response, validate_method,
        },
        ModalHttpResponse, RequestHttpConvert, ResponseHttpConvert,
    },
    jsonrpc::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse},
    stdio::{RequestJsonRpcConvert, ResponseJsonRpcConvert},
    util::parse_from_value,
    ProtocolError, ServiceResponse,
};
use serde_json::Value;

use super::{Request, Response};

const SAY_HELLO_HTTP_PATH: &str = "/say_hello";
const SAY_GREETING_HTTP_PATH: &str = "/say_greeting";
const SAY_HELLO_STREAM_HTTP_PATH: &str = "/say_hello_stream";

const SAY_HELLO_JSONRPC_METHOD: &str = "sayHello";
const SAY_GREETING_JSONRPC_METHOD: &str = "sayGreeting";
const SAY_HELLO_STREAM_JSONRPC_METHOD: &str = "sayHelloStream";

#[async_trait]
impl RequestHttpConvert<Request> for Request {
    async fn from_http_request(
        request: hyper::Request<Body>,
    ) -> Result<Option<Self>, ProtocolError> {
        let path = request.uri().path();
        let request = match path {
            SAY_HELLO_HTTP_PATH => {
                validate_method(&request, Method::GET)?;
                Self::SayHello(parse_request(request).await?)
            }
            SAY_GREETING_HTTP_PATH => {
                validate_method(&request, Method::POST)?;
                Self::SayCustomGreeting(parse_request(request).await?)
            }
            SAY_HELLO_STREAM_HTTP_PATH => {
                validate_method(&request, Method::POST)?;
                Self::SayHelloStream(parse_request(request).await?)
            }
            _ => return Ok(None),
        };
        Ok(Some(request))
    }

    fn to_http_request(
        &self,
        base_url: &hyper::Uri,
    ) -> Result<Option<hyper::Request<Body>>, ProtocolError> {
        let request = match self {
            Self::SayHello(request) => {
                serialize_to_http_request(base_url, SAY_HELLO_HTTP_PATH, Method::GET, &request)?
            }
            Self::SayCustomGreeting(request) => {
                serialize_to_http_request(base_url, SAY_GREETING_HTTP_PATH, Method::POST, &request)?
            }
            Self::SayHelloStream(request) => serialize_to_http_request(
                base_url,
                SAY_HELLO_STREAM_HTTP_PATH,
                Method::POST,
                &request,
            )?,
        };
        Ok(Some(request))
    }
}

#[async_trait]
impl ResponseHttpConvert<Request, Response> for Response {
    async fn from_http_response(
        response: ModalHttpResponse,
        original_request: &Request,
    ) -> Result<Option<ServiceResponse<Self>>, ProtocolError> {
        Ok(Some(match response {
            ModalHttpResponse::Single(response) => match original_request {
                Request::SayHello(_) => {
                    ServiceResponse::Single(Self::SayHello(parse_response(response).await?))
                }
                Request::SayCustomGreeting(_) => ServiceResponse::Single(Self::SayCustomGreeting(
                    parse_response(response).await?,
                )),
                Request::SayHelloStream(_) => ServiceResponse::Multiple(notification_sse_stream(
                    original_request.clone(),
                    response,
                )),
            },
            ModalHttpResponse::Event(event) => ServiceResponse::Single(match original_request {
                Request::SayHelloStream(_) => Self::SayHelloStream(parse_from_value(event)?),
                _ => return Ok(None),
            }),
        }))
    }

    fn to_http_response(
        response: ServiceResponse<Self>,
    ) -> Result<Option<ModalHttpResponse>, ProtocolError> {
        let response = match response {
            ServiceResponse::Single(response) => match response {
                Self::SayHello(response) => ModalHttpResponse::Single(serialize_to_http_response(
                    &response,
                    StatusCode::OK,
                )?),
                Self::SayCustomGreeting(response) => ModalHttpResponse::Single(
                    serialize_to_http_response(&response, StatusCode::OK)?,
                ),
                Self::SayHelloStream(response) => {
                    ModalHttpResponse::Event(serde_json::to_value(response).unwrap())
                }
            },
            ServiceResponse::Multiple(stream) => {
                // Output a single server-side event HTTP response
                ModalHttpResponse::Single(notification_sse_response(stream))
            }
        };
        Ok(Some(response))
    }
}

impl RequestJsonRpcConvert<Request> for Request {
    fn from_jsonrpc_request(value: JsonRpcRequest) -> Result<Option<Self>, ProtocolError> {
        Ok(Some(match value.method.as_str() {
            SAY_HELLO_JSONRPC_METHOD => Self::SayHello(value.parse_params()?),
            SAY_GREETING_JSONRPC_METHOD => Self::SayCustomGreeting(value.parse_params()?),
            SAY_HELLO_STREAM_JSONRPC_METHOD => Self::SayHelloStream(value.parse_params()?),
            _ => return Ok(None),
        }))
    }

    fn into_jsonrpc_request(&self) -> JsonRpcRequest {
        let (method, params) = match self {
            Self::SayHello(request) => (
                SAY_HELLO_JSONRPC_METHOD,
                Some(serde_json::to_value(request).unwrap()),
            ),
            Self::SayCustomGreeting(request) => (
                SAY_GREETING_JSONRPC_METHOD,
                Some(serde_json::to_value(request).unwrap()),
            ),
            Self::SayHelloStream(request) => (
                SAY_HELLO_STREAM_JSONRPC_METHOD,
                Some(serde_json::to_value(request).unwrap()),
            ),
        };
        JsonRpcRequest::new(method.to_string(), params)
    }
}

impl ResponseJsonRpcConvert<Request, Response> for Response {
    fn from_jsonrpc_message(
        value: JsonRpcMessage,
        original_request: &Request,
    ) -> Result<Option<Self>, ProtocolError> {
        match value {
            JsonRpcMessage::Response(resp) => {
                let result = resp.get_result()?;
                Ok(Some(match original_request {
                    Request::SayHello(_) => Self::SayHello(parse_from_value(result)?),
                    Request::SayCustomGreeting(_) => {
                        Self::SayCustomGreeting(parse_from_value(result)?)
                    }
                    _ => return Ok(None),
                }))
            }
            JsonRpcMessage::Notification(resp) => {
                let result = resp.get_result()?;
                Ok(Some(match original_request {
                    Request::SayHelloStream(_) => Self::SayHelloStream(parse_from_value(result)?),
                    _ => return Ok(None),
                }))
            }
            _ => Ok(None),
        }
    }

    fn into_jsonrpc_message(response: Response, id: Value) -> JsonRpcMessage {
        let mut is_notification = false;
        let result = Ok(match response {
            Response::SayHello(response) => serde_json::to_value(response).unwrap(),
            Response::SayCustomGreeting(response) => serde_json::to_value(response).unwrap(),
            Response::SayHelloStream(response) => {
                is_notification = true;
                serde_json::to_value(response).unwrap()
            }
        });
        match is_notification {
            true => JsonRpcNotification::new_with_result_params(result, id.to_string()).into(),
            false => JsonRpcResponse::new(result, id).into(),
        }
    }
}
