mod comm;

use std::{
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    stream::{pending, select_all, SelectAll},
    Stream, StreamExt,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{stdin, stdout, AsyncBufReadExt, BufReader, Stdin, Stdout},
    sync::{
        mpsc::{self, UnboundedSender},
        Mutex,
    },
};
use tower::{timeout::Timeout, Service};

use crate::{
    ConfigExampleSnippet, NotificationStream, ProtocolError, ServiceError, ServiceFuture,
    ServiceResponse, DEFAULT_TIMEOUT_SECS,
};

use super::{serialize_payload, RequestJsonRpcConvert, ResponseJsonRpcConvert};

/// Configuration for the stdio server.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StdioServerConfig {
    /// Timeout for service requests in seconds.
    pub service_timeout_secs: u64,
}

impl ConfigExampleSnippet for StdioServerConfig {
    fn config_example_snippet() -> String {
        r#"# The timeout duration in seconds for the underlying backend service.
# service_timeout_secs = 60"#
            .into()
    }
}

impl Default for StdioServerConfig {
    fn default() -> Self {
        Self {
            service_timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

struct IdentifiedNotification<Response> {
    id: u64,
    result: Option<Result<Response, ProtocolError>>,
}

/// Server for stdio communication via a parent process.
pub struct StdioServer<Request, Response, S>
where
    Request: RequestJsonRpcConvert<Request> + Send,
    Response: ResponseJsonRpcConvert<Request, Response> + Send,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + 'static,
{
    service: Timeout<S>,
    stdin: BufReader<Stdin>,
    stdout: Arc<Mutex<Stdout>>,
    notification_streams_tx: Option<UnboundedSender<ServerNotificationLink<Response>>>,
    request_phantom: PhantomData<Request>,
}

struct ServerNotificationLink<Response> {
    id: u64,
    stream: NotificationStream<Response>,
    is_complete: bool,
}

impl<Response> Stream for ServerNotificationLink<Response> {
    type Item = IdentifiedNotification<Response>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.stream.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => match result {
                None => match self.is_complete {
                    true => Poll::Ready(None),
                    false => {
                        self.is_complete = true;
                        Poll::Ready(Some(IdentifiedNotification {
                            id: self.id,
                            result: None,
                        }))
                    }
                },
                Some(result) => Poll::Ready(Some(IdentifiedNotification {
                    id: self.id,
                    result: Some(result),
                })),
            },
        }
    }
}

impl<Request, Response, S> StdioServer<Request, Response, S>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
    S: Service<
            Request,
            Response = ServiceResponse<Response>,
            Error = ServiceError,
            Future = ServiceFuture<ServiceResponse<Response>>,
        > + Send
        + 'static,
{
    /// Creates a new server for stdio communication. Client requests will be
    /// converted and forwarded to the `service`.
    pub fn new(service: S, config: StdioServerConfig) -> Self {
        Self {
            service: Timeout::new(service, Duration::from_secs(config.service_timeout_secs)),
            stdin: BufReader::new(stdin()),
            stdout: Arc::new(Mutex::new(stdout())),
            notification_streams_tx: None,
            request_phantom: Default::default(),
        }
    }

    /// Listens & processes requests from the parent process via stdin, until a [`std::io::Error`]
    /// is encountered.
    pub async fn run(mut self) -> std::io::Result<()> {
        // insert dummy notification stream so that tokio::select (in main loop)
        // does not immediately return if no streams exist
        let (notification_stream_tx, mut notification_stream_rx) = mpsc::unbounded_channel();
        self.notification_streams_tx = Some(notification_stream_tx);
        let mut notification_streams: SelectAll<ServerNotificationLink<Response>> =
            select_all([ServerNotificationLink {
                id: u64::MAX,
                stream: pending().boxed(),
                is_complete: false,
            }]);

        loop {
            let mut serialized_request = String::new();
            tokio::select! {
                read_result = self.stdin.read_line(&mut serialized_request) => {
                    if read_result? == 0 {
                        break;
                    }
                    self.handle_request(serialized_request);
                },
                id_notification = notification_streams.next() => {
                    self.handle_notification(id_notification.unwrap()).await;
                }
                stream = notification_stream_rx.recv() => {
                    notification_streams.push(stream.unwrap());
                }
            }
        }
        Ok(())
    }
}
