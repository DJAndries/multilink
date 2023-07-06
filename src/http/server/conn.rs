use std::{
    marker::PhantomData,
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
};

use hyper::{Body, Request as HttpRequest, Response as HttpResponse};
use tower::{timeout::Timeout, Service};
use tracing::{debug, info, warn};

use crate::{
    error::ProtocolErrorType, ProtocolError, ServiceError, ServiceFuture, ServiceResponse,
};

use super::{
    generic_error, HttpServerConfig, ModalHttpResponse, RequestHttpConvert, ResponseHttpConvert,
    API_KEY_HEADER,
};

fn check_api_key(
    config: &HttpServerConfig,
    request: &HttpRequest<Body>,
) -> Result<(), ProtocolError> {
    if !config.api_keys.is_empty() {
        let key_header = request
            .headers()
            .get(API_KEY_HEADER)
            .map(|v| v.to_str().unwrap_or_default())
            .unwrap_or_default();
        if !config.api_keys.contains(key_header) {
            return Err(generic_error(ProtocolErrorType::Unauthorized));
        }
    }
    Ok(())
}

pub(super) struct HttpServerConnService<Request, Response, S>
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

impl<Request, Response, S> HttpServerConnService<Request, Response, S>
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
    pub(super) fn new(
        config: Arc<HttpServerConfig>,
        service: Timeout<S>,
        remote_addr: SocketAddr,
    ) -> Self {
        Self {
            config,
            service,
            remote_addr,
            request_phantom: Default::default(),
            response_phantom: Default::default(),
        }
    }
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
            if let Err(e) = check_api_key(&config, &request) {
                return Ok(e.into());
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
