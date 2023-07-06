pub mod error;
#[cfg(any(feature = "http-client", feature = "http-server"))]
pub mod http;
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
#[cfg(any(feature = "stdio-client", feature = "stdio-server"))]
pub mod stdio;
pub mod util;

pub use error::ProtocolError;
pub use tower;

use std::{error::Error, pin::Pin};

use futures::{Future, Stream};
use tower::Service;

const DEFAULT_TIMEOUT_SECS: u64 = 900;

pub trait ConfigExampleSnippet {
    fn config_example_snippet() -> &'static str;
}
pub type NotificationStream<Response> =
    Pin<Box<dyn Stream<Item = Result<Response, ProtocolError>> + Send>>;

pub enum ServiceResponse<Response> {
    Single(Response),
    Multiple(NotificationStream<Response>),
}

pub type ServiceError = Box<dyn Error + Send + Sync + 'static>;
pub type ServiceFuture<Response> =
    Pin<Box<dyn Future<Output = Result<Response, ServiceError>> + Send>>;
pub type BoxedService<Request, Response> = Box<
    dyn Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Sync,
>;
