pub mod error;
#[cfg(any(feature = "http-client", feature = "http-server"))]
pub mod http;
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
pub mod service;
#[cfg(any(feature = "stdio-client", feature = "stdio-server"))]
pub mod stdio;
pub mod util;

pub use error::ProtocolError;
pub use tower;

const DEFAULT_TIMEOUT_SECS: u64 = 900;
