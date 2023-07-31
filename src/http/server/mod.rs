mod conn;

use std::{
    collections::HashSet, convert::Infallible, marker::PhantomData, net::SocketAddr, sync::Arc,
    time::Duration,
};

use hyper::{
    server::conn::AddrStream, service::make_service_fn, Body, Response as HttpResponse, Server,
};
use serde::{Deserialize, Serialize};
use tower::{timeout::Timeout, Service};
use tracing::info;

use crate::{
    http::{server::conn::HttpServerConnService, API_KEY_HEADER},
    ConfigExampleSnippet, ProtocolError, ServiceError, ServiceFuture, ServiceResponse,
    DEFAULT_TIMEOUT_SECS,
};

use super::util::serialize_to_http_response;

use super::{
    generic_error, ModalHttpResponse, ProtocolHttpError, RequestHttpConvert, ResponseHttpConvert,
};

/// Configuration for the HTTP server.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpServerConfig {
    /// Port to listen on.
    pub port: u16,
    /// An optional set of API keys for restricting access to the server.
    /// If omitted, an API key is not needed to make a request.
    pub api_keys: HashSet<String>,
    /// Timeout for service requests in seconds.
    pub service_timeout_secs: u64,
}

impl ConfigExampleSnippet for HttpServerConfig {
    fn config_example_snippet() -> String {
        r#"# The port number on which the server listens.
# port = 8080

# The API keys allowed to access the server. If omitted, an API key is not
# needed to make a request.
# api_keys = ["key1", "key2", "key3"]

# The timeout duration in seconds for the underlying backend service.
# service_timeout_secs = 60"#
            .into()
    }
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

impl Into<HttpResponse<Body>> for ProtocolError {
    fn into(self) -> HttpResponse<Body> {
        let payload = ProtocolHttpError {
            error: self.error.to_string(),
        };
        serialize_to_http_response(&payload, self.error_type.into())
            .expect("should serialize error into http response")
    }
}

/// Server for HTTP communication with remote clients.
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
    /// Creates a new client for HTTP communication. Client requests will be
    /// converted and forwarded to the `service`.
    pub fn new(service: S, config: HttpServerConfig) -> Self {
        let service = Timeout::new(service, Duration::from_secs(config.service_timeout_secs));
        Self {
            config: Arc::new(config),
            service,
            request_phantom: Default::default(),
            response_phantom: Default::default(),
        }
    }

    /// Listens & processes requests from remote clients, until a [`hyper::Error`]
    /// is encountered.
    pub async fn run(self) -> Result<(), hyper::Error> {
        let config_cl = self.config.clone();
        let service_cl = self.service.clone();
        let make_service = make_service_fn(move |conn: &AddrStream| {
            let config = config_cl.clone();
            let service = service_cl.clone();
            let remote_addr = conn.remote_addr();
            async move { Ok::<_, Infallible>(HttpServerConnService::new(config, service, remote_addr)) }
        });
        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.port));

        let server = Server::try_bind(&addr)?;

        info!("listening to http requests on port {}", self.config.port);

        server.serve(make_service).await
    }
}
