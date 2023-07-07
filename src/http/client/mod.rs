pub mod util;

use std::{
    marker::PhantomData,
    str::FromStr,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use hyper::{
    client::HttpConnector,
    http::{uri::InvalidUri, HeaderValue},
    Client, StatusCode, Uri,
};
use hyper_rustls::HttpsConnector;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower::{timeout::Timeout, Service};

use crate::{
    error::{ProtocolError, ProtocolErrorType},
    ConfigExampleSnippet, ServiceError, ServiceFuture, ServiceResponse, DEFAULT_TIMEOUT_SECS,
};

use self::util::parse_response;

use super::{
    generic_error, HttpNotificationPayload, ModalHttpResponse, ProtocolHttpError,
    RequestHttpConvert, ResponseHttpConvert, API_KEY_HEADER,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpClientConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_secs: u64,
}

impl ConfigExampleSnippet for HttpClientConfig {
    fn config_example_snippet() -> String {
        r#"# The base URL for the HttpClient.
# base_url = "https://example.com"

# The API key for authenticating requests made by the HttpClient (optional).
# This field can be omitted if an API key is not required.
# api_key = "YOUR_API_KEY"

# The timeout duration in seconds for the HttpClient.
# timeout_secs = 60"#
            .into()
    }
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
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

impl Into<Result<Value, ProtocolError>> for HttpNotificationPayload {
    fn into(self) -> Result<Value, ProtocolError> {
        if let Some(e) = self.error {
            return Err(e.into());
        }
        self.result
            .ok_or_else(|| generic_error(ProtocolErrorType::NotFound))
    }
}

#[derive(Clone)]
pub struct HttpClient<Request, Response>
where
    Request: RequestHttpConvert<Request> + Clone + Send + 'static,
    Response: ResponseHttpConvert<Request, Response> + Send + 'static,
{
    base_url: Arc<Uri>,
    config: Arc<HttpClientConfig>,
    client: Timeout<Client<HttpsConnector<HttpConnector>>>,
    request_phantom: PhantomData<Request>,
    response_phantom: PhantomData<Response>,
}

impl<Request, Response> HttpClient<Request, Response>
where
    Request: RequestHttpConvert<Request> + Clone + Send + 'static,
    Response: ResponseHttpConvert<Request, Response> + Send + 'static,
{
    pub fn new(config: HttpClientConfig) -> Result<Self, InvalidUri> {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_or_http()
            .enable_http1()
            .build();
        let client = Timeout::new(
            Client::builder().build(https),
            Duration::from_secs(config.timeout_secs),
        );
        let base_url = Arc::new(Uri::from_str(&config.base_url)?);
        Ok(Self {
            base_url,
            config: Arc::new(config),
            client,
            request_phantom: Default::default(),
            response_phantom: Default::default(),
        })
    }
}

impl<Request, Response> Service<Request> for HttpClient<Request, Response>
where
    Request: RequestHttpConvert<Request> + Clone + Send + Sync + 'static,
    Response: ResponseHttpConvert<Request, Response> + Send + 'static,
{
    type Response = ServiceResponse<Response>;
    type Error = ServiceError;
    type Future = ServiceFuture<ServiceResponse<Response>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let http_request = request.to_http_request(&self.base_url);
        let mut client = self.client.clone();
        let api_key = self.config.api_key.clone();
        Box::pin(async move {
            let mut http_request =
                http_request?.ok_or_else(|| generic_error(ProtocolErrorType::NotFound))?;
            if let Some(api_key) = api_key {
                http_request
                    .headers_mut()
                    .insert(API_KEY_HEADER, HeaderValue::from_str(&api_key)?);
            }
            let response = client.call(http_request).await?;
            let status = response.status();
            if !status.is_success() {
                return Err(Box::new(ProtocolError {
                    error_type: response.status().into(),
                    error: Box::new(parse_response::<ProtocolHttpError>(response).await?),
                }))?;
            }
            let response =
                Response::from_http_response(ModalHttpResponse::Single(response), &request).await?;
            Ok(response.ok_or_else(|| generic_error(ProtocolErrorType::NotFound))?)
        })
    }
}
