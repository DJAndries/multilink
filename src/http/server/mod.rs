pub mod util;

use std::{
    collections::HashSet,
    convert::Infallible,
    marker::PhantomData,
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use hyper::{
    server::conn::AddrStream, service::make_service_fn, Body, Request as HttpRequest,
    Response as HttpResponse, Server, StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower::{timeout::Timeout, Service};
use tracing::{debug, info, warn};

use crate::{
    error::ProtocolErrorType,
    http::API_KEY_HEADER,
    service::{ServiceError, ServiceFuture, ServiceResponse},
    ProtocolError, DEFAULT_TIMEOUT_SECS,
};

use self::util::serialize_to_http_response;

use super::{
    generic_error, HttpNotificationPayload, ModalHttpResponse, ProtocolHttpError,
    RequestHttpConvert, ResponseHttpConvert,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpServerConfig {
    pub port: u16,
    pub api_keys: HashSet<String>,
    pub service_timeout_secs: u64,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            api_keys: HashSet::new(),
            service_timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
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

impl Into<HttpResponse<Body>> for ProtocolError {
    fn into(self) -> HttpResponse<Body> {
        let payload = ProtocolHttpError {
            error: self.error.to_string(),
        };
        serialize_to_http_response(&payload, self.error_type.into())
            .expect("should serialize error into http response")
    }
}

struct HttpServerConnService<Request, Response, S>
where
    Request: RequestHttpConvert<Request> + Clone,
    Response: ResponseHttpConvert<Request, Response>,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Clone
        + 'static,
{
    config: Arc<HttpServerConfig>,
    service: Timeout<S>,
    remote_addr: SocketAddr,
    request_phantom: PhantomData<Request>,
    response_phantom: PhantomData<Response>,
}

impl<Request, Response, S> Service<HttpRequest<Body>>
    for HttpServerConnService<Request, Response, S>
where
    Request: RequestHttpConvert<Request> + Clone + Send,
    Response: ResponseHttpConvert<Request, Response> + Send,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Clone
        + 'static,
{
    type Response = HttpResponse<Body>;
    type Error = ServiceError;
    type Future = ServiceFuture<HttpResponse<Body>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: HttpRequest<Body>) -> Self::Future {
        let config = self.config.clone();
        let mut service = self.service.clone();
        debug!("received http request from {}", self.remote_addr);
        let remote_addr = self.remote_addr.clone();
        Box::pin(async move {
            if !config.api_keys.is_empty() {
                let key_header = request
                    .headers()
                    .get(API_KEY_HEADER)
                    .map(|v| v.to_str().unwrap_or_default())
                    .unwrap_or_default();
                if !config.api_keys.contains(key_header) {
                    return Ok(generic_error(ProtocolErrorType::Unauthorized).into());
                }
            }

            let uri = request.uri().to_string();
            let request_result = Request::from_http_request(request).await;
            let response = match request_result {
                Ok(request_option) => match request_option {
                    Some(request) => {
                        let response = service.call(request).await;
                        response
                            .map(|response| {
                                // Map an Ok service response into an http response
                                Response::to_http_response(response)
                                    .map(|r| r.and_then(|r| match r {
                                        ModalHttpResponse::Single(r) => Some(r),
                                        ModalHttpResponse::Event(_) => {
                                            warn!("unexpected event response returned from http response conversion, returning 404");
                                            None
                                        }
                                    }))
                                    .unwrap_or_else(|e| Some(e.into()))
                                    .unwrap_or_else(|| {
                                        generic_error(ProtocolErrorType::NotFound).into()
                                    })
                            })
                            .unwrap_or_else(|e| {
                                // Map service error into an http response
                                ProtocolError::from(e).into()
                            })
                    }
                    // If option is None, we can assume that the request resulted
                    // in Not Found
                    None => generic_error(ProtocolErrorType::NotFound).into(),
                },
                Err(e) => e.into(),
            };
            info!(
                uri = uri,
                status = response.status().to_string(),
                "handled http request from {}",
                remote_addr,
            );
            Ok(response)
        })
    }
}

pub struct HttpServer<Request, Response, S>
where
    Request: RequestHttpConvert<Request> + Clone + Send,
    Response: ResponseHttpConvert<Request, Response>,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Clone
        + 'static,
{
    config: Arc<HttpServerConfig>,
    service: Timeout<S>,
    request_phantom: PhantomData<Request>,
    response_phantom: PhantomData<Response>,
}

impl<Request, Response, S> HttpServer<Request, Response, S>
where
    Request: RequestHttpConvert<Request> + Send + Clone,
    Response: ResponseHttpConvert<Request, Response>,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Clone
        + 'static,
{
    pub fn new(service: S, config: HttpServerConfig) -> Self {
        let service = Timeout::new(service, Duration::from_secs(config.service_timeout_secs));
        Self {
            config: Arc::new(config),
            service,
            request_phantom: Default::default(),
            response_phantom: Default::default(),
        }
    }
}

impl<Request, Response, S> HttpServer<Request, Response, S>
where
    Request: RequestHttpConvert<Request> + Clone + Send + 'static,
    Response: ResponseHttpConvert<Request, Response> + Send + 'static,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Clone
        + 'static,
{
    pub async fn run(self) -> Result<(), hyper::Error> {
        let config_cl = self.config.clone();
        let service_cl = self.service.clone();
        let make_service = make_service_fn(move |conn: &AddrStream| {
            let config = config_cl.clone();
            let service = service_cl.clone();
            let remote_addr = conn.remote_addr();
            async move {
                Ok::<_, Infallible>(HttpServerConnService {
                    config,
                    service,
                    remote_addr,
                    request_phantom: Default::default(),
                    response_phantom: Default::default(),
                })
            }
        });
        let addr = SocketAddr::from(([127, 0, 0, 1], self.config.port));

        let server = Server::try_bind(&addr)?;

        info!("listening to http requests on port {}", self.config.port);

        server.serve(make_service).await
    }
}
