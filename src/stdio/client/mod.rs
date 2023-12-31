mod comm;

use std::{
    path::Path,
    process::Stdio,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::BufReader,
    process::{Child, Command},
    sync::{mpsc::UnboundedSender, oneshot},
    time::timeout,
};
use tower::Service;

use crate::{
    ConfigExampleSnippet, ProtocolError, ServiceError, ServiceFuture, ServiceResponse,
    DEFAULT_TIMEOUT_SECS,
};

use self::comm::StdioClientCommTask;

use super::{serialize_payload, RequestJsonRpcConvert, ResponseJsonRpcConvert, StdioError};

/// Configuration for the stdio client.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StdioClientConfig {
    /// Optional binary path for spawning child processes.
    /// Defaults to PATH.
    pub bin_path: Option<String>,
    /// Timeout for client requests in seconds.
    pub timeout_secs: u64,
}

impl ConfigExampleSnippet for StdioClientConfig {
    fn config_example_snippet() -> String {
        r#"# Path containing all llmvm binaries, defaults to $PATH
# bin_path = ""

# The timeout duration in seconds for requests, defaults to 900
# timeout_secs = 60"#
            .into()
    }
}

impl Default for StdioClientConfig {
    fn default() -> Self {
        Self {
            bin_path: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

struct ClientRequestTrx<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send,
    Response: ResponseJsonRpcConvert<Request, Response> + Send,
{
    request: Request,
    response_tx: oneshot::Sender<Result<ServiceResponse<Response>, ProtocolError>>,
}

struct ClientNotificationLink<Request, Response> {
    request: Request,
    notification_tx: UnboundedSender<Result<Response, ProtocolError>>,
}

/// Client for stdio communication via a child process.
/// If cloned, this client will continue to communicate with the same child process.
#[derive(Clone)]
pub struct StdioClient<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
{
    _child: Arc<Child>,
    to_child_tx: UnboundedSender<ClientRequestTrx<Request, Response>>,
    config: StdioClientConfig,
}

impl<Request, Response> Service<Request> for StdioClient<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
{
    type Response = ServiceResponse<Response>;
    type Error = ServiceError;
    type Future = ServiceFuture<ServiceResponse<Response>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let to_child_tx = self.to_child_tx.clone();
        let timeout_duration = Duration::from_secs(self.config.timeout_secs);
        Box::pin(async move {
            let (response_tx, response_rx) = oneshot::channel();
            to_child_tx
                .send(ClientRequestTrx {
                    request,
                    response_tx,
                })
                .map_err(|_| StdioError::SendRequestCommTask)?;
            let response_result = timeout(timeout_duration, response_rx)
                .await
                .map_err(|_| StdioError::Timeout)?;
            Ok(response_result.map_err(|_| StdioError::RecvResponseCommTask)??)
        })
    }
}

impl<Request, Response> StdioClient<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
{
    /// Creates a new client for stdio communication. A new child process will be
    /// spawned, and a [`std::io::Error`] will be returned if spawning fails.
    pub async fn new(
        program: &str,
        args: &[&str],
        config: StdioClientConfig,
    ) -> std::io::Result<Self> {
        let program_with_bin_path = config.bin_path.as_ref().map(|bin_path| {
            Path::new(bin_path)
                .join(program)
                .to_str()
                .expect("command name with bin path should convert to string")
                .to_string()
        });
        let mut child = Command::new(
            program_with_bin_path
                .as_ref()
                .map(|v| v.as_str())
                .unwrap_or(program),
        )
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let comm_task = StdioClientCommTask::new(stdin, stdout);
        let to_child_tx = comm_task.start();
        Ok(Self {
            _child: Arc::new(child),
            to_child_tx,
            config,
        })
    }
}
