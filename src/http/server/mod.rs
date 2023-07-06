mod conn;
pub mod util;

use std::{
    collections::HashSet, convert::Infallible, marker::PhantomData, net::SocketAddr, sync::Arc,
    time::Duration,
};

use hyper::{
    server::conn::AddrStream, service::make_service_fn, Body, Response as HttpResponse, Server,
    StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower::{timeout::Timeout, Service};
use tracing::info;

use crate::{
    error::ProtocolErrorType,
    http::{server::conn::HttpServerConnService, API_KEY_HEADER},
    ProtocolError, ServiceError, ServiceFuture, ServiceResponse, DEFAULT_TIMEOUT_SECS,
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
            async move { Ok::<_, Infallible>(HttpServerConnService::new(config, service, remote_addr)) }
        });
        let addr = SocketAddr::from(([127, 0, 0, 1], self.config.port));

        let server = Server::try_bind(&addr)?;

        info!("listening to http requests on port {}", self.config.port);

        server.serve(make_service).await
    }
}
