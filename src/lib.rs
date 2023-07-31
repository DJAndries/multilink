//! multilink is an IPC library that allows communication via two methods:
//!
//! - Local processes/stdio: JSON-RPC messages are passed between parent/child process via stdin/stdout
//! - Remote processes/HTTP: HTTP requests/responses are passed between processes on remote hosts
//!
//! Utilizes `tower` to handle RPC calls.

/// Protocol error types.
pub mod error;
#[cfg(any(feature = "http-client", feature = "http-server"))]
/// HTTP server and client.
pub mod http;
#[cfg(feature = "jsonrpc")]
/// JSON-RPC types and methods.
pub mod jsonrpc;
#[cfg(any(feature = "stdio-client", feature = "stdio-server"))]
/// JSON-RPC over stdio server and client.
pub mod stdio;
/// Miscellaneous utility functions.
pub mod util;

pub use error::ProtocolError;
pub use tower;

use std::{error::Error, pin::Pin};

use futures::{Future, Stream};
use tower::Service;

/// Default request timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 900;

/// A configuration data structure that provides an example for
/// generating new TOML configuration files. The example should
/// include customizable fields with comments explaining their purpose.
pub trait ConfigExampleSnippet {
    /// Returns the configuration example snippet to be used
    /// in new configuration files.
    fn config_example_snippet() -> String;
}

/// A stream of multiple response results returned by the service.
pub type NotificationStream<Response> =
    Pin<Box<dyn Stream<Item = Result<Response, ProtocolError>> + Send>>;

/// A response container returned by a multilink service.
pub enum ServiceResponse<Response> {
    /// Contains a single response returned by the service.
    Single(Response),
    /// Contains a stream of multiple responses returned by the service.
    Multiple(NotificationStream<Response>),
}

/// A boxed error type that may be returned by service calls.
pub type ServiceError = Box<dyn Error + Send + Sync + 'static>;
/// A future that returns a result with a generic response and [`ServiceError`].
/// This is returned by service calls.
pub type ServiceFuture<Response> =
    Pin<Box<dyn Future<Output = Result<Response, ServiceError>> + Send>>;
/// A boxed dynamic type for multilink services. The service must return
/// a result with a [`ServiceResponse`] or [`ServiceError`].
pub type BoxedService<Request, Response> = Box<
    dyn Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + Sync,
>;
