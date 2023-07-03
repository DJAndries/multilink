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
use serde_json::Value;
use tokio::{
    io::{stdin, stdout, AsyncBufReadExt, AsyncWriteExt, BufReader, Stdin, Stdout},
    sync::{
        mpsc::{self, UnboundedSender},
        Mutex,
    },
};
use tower::{timeout::Timeout, Service};
use tracing::error;

use crate::{
    jsonrpc::{JsonRpcMessage, JsonRpcNotification},
    service::{NotificationStream, ServiceError, ServiceFuture, ServiceResponse},
    ProtocolError, DEFAULT_TIMEOUT_SECS,
};

use super::{serialize_payload, RequestJsonRpcConvert, ResponseJsonRpcConvert};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StdioServerConfig {
    pub service_timeout_secs: u64,
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
    pub fn new(service: S, config: StdioServerConfig) -> Self {
        Self {
            service: Timeout::new(service, Duration::from_secs(config.service_timeout_secs)),
            stdin: BufReader::new(stdin()),
            stdout: Arc::new(Mutex::new(stdout())),
            notification_streams_tx: None,
            request_phantom: Default::default(),
        }
    }

    async fn output_message(stdout: &Mutex<Stdout>, message: JsonRpcMessage) {
        let serialized_message = serialize_payload(&message);
        stdout
            .lock()
            .await
            .write_all(serialized_message.as_bytes())
            .await
            .ok();
    }

    fn handle_request(&mut self, serialized_request: String) {
        let stdout = self.stdout.clone();
        let notification_streams_tx = self
            .notification_streams_tx
            .clone()
            .expect("notfication_streams_tx should be initialized");

        let value: Value = serde_json::from_str(&serialized_request).unwrap_or_default();
        let (result_future, id) = match JsonRpcMessage::try_from(value) {
            Err(e) => {
                error!("could not parse json rpc message from client: {e}, request: {serialized_request}");
                return;
            }
            Ok(message) => match message {
                JsonRpcMessage::Request(jsonrpc_request) => {
                    let id = jsonrpc_request.id.as_u64().unwrap_or_default();
                    match Request::from_jsonrpc_request(jsonrpc_request) {
                        Err(e) => {
                            error!("could not derive request enum from json rpc request: {e}");
                            return;
                        }
                        Ok(request) => match request {
                            None => {
                                error!("unknown json rpc request received");
                                return;
                            }
                            Some(request) => (self.service.call(request), id),
                        },
                    }
                }
                _ => {
                    error!("ignoring non-request json rpc message from client");
                    return;
                }
            },
        };
        tokio::spawn(async move {
            let result = result_future.await;
            match result {
                Ok(response) => match response {
                    ServiceResponse::Single(response) => {
                        Self::output_message(
                            stdout.as_ref(),
                            Response::into_jsonrpc_message(Ok(response), id.into()),
                        )
                        .await;
                    }
                    ServiceResponse::Multiple(stream) => {
                        notification_streams_tx
                            .send(ServerNotificationLink {
                                id,
                                stream,
                                is_complete: false,
                            })
                            .ok();
                    }
                },
                Err(e) => {
                    Self::output_message(
                        stdout.as_ref(),
                        Response::into_jsonrpc_message(Err(e.into()), id.into()),
                    )
                    .await
                }
            }
        });
    }

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
                    let id_notification = id_notification.unwrap();
                    match id_notification.result {
                        Some(result) => {
                            Self::output_message(self.stdout.as_ref(), Response::into_jsonrpc_message(result.map_err(|e| e.into()), id_notification.id.into())).await;
                        },
                        None => {
                            Self::output_message(self.stdout.as_ref(), JsonRpcNotification::new(id_notification.id.to_string(), None).into()).await;
                        }
                    }
                }
                stream = notification_stream_rx.recv() => {
                    notification_streams.push(stream.unwrap());
                }
            }
        }
        Ok(())
    }
}
