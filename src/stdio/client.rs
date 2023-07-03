use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{
        mpsc::{self, UnboundedSender},
        oneshot,
    },
    time::timeout,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tower::Service;
use tracing::{error, warn};

use crate::{
    error::{ProtocolErrorType, SerializableProtocolError},
    jsonrpc::{JsonRpcMessage, JsonRpcResponse},
    service::{ServiceError, ServiceFuture, ServiceResponse},
    ProtocolError, DEFAULT_TIMEOUT_SECS,
};

use super::{serialize_payload, RequestJsonRpcConvert, ResponseJsonRpcConvert};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StdioClientConfig {
    pub bin_path: Option<String>,
    pub timeout_secs: u64,
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
    response_tx: oneshot::Sender<Result<ServiceResponse<Response>, SerializableProtocolError>>,
}

struct ClientNotificationLink<Request, Response> {
    request: Request,
    notification_tx: UnboundedSender<Result<Response, ProtocolError>>,
}

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

// impl<Request, Response> Clone for StdioClient<Request, Response>
// where
//     Request: RequestJsonRpcConvert<Request> + Send + 'static,
//     Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
// {
//     fn clone(&self) -> Self {
//         Self {
//             _child: self._child.clone(),
//             to_child_tx: self.to_child_tx.clone(),
//             config: self.config.clone()
//         }
//     }
// }

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
                .map_err(|_| SerializableProtocolError {
                    error_type: ProtocolErrorType::Internal,
                    description: "should be able to send stdio request to comm task".to_string(),
                })?;
            let response_result = timeout(timeout_duration, response_rx).await.map_err(|_| {
                SerializableProtocolError {
                    error_type: ProtocolErrorType::Internal,
                    description: "timed out".to_string(),
                }
            })?;
            Ok(response_result.map_err(|_| SerializableProtocolError {
                error_type: ProtocolErrorType::Internal,
                description: "should be able to recv response for stdio request from comm task"
                    .to_string(),
            })??)
        })
    }
}

impl<Request, Response> StdioClient<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
{
    async fn output_message(stdin: &mut ChildStdin, message: JsonRpcMessage) {
        let serialized_response = serialize_payload(&message);
        stdin.write_all(serialized_response.as_bytes()).await.ok();
    }

    fn start_comm_task(
        mut stdin: ChildStdin,
        mut stdout: BufReader<ChildStdout>,
    ) -> UnboundedSender<ClientRequestTrx<Request, Response>> {
        let (to_child_tx, mut to_child_rx) =
            mpsc::unbounded_channel::<ClientRequestTrx<Request, Response>>();
        let mut notification_links = HashMap::new();
        tokio::spawn(async move {
            let mut last_req_id = 0u64;
            let mut pending_reqs: HashMap<u64, ClientRequestTrx<Request, Response>> =
                HashMap::new();
            loop {
                let mut stdout_message = String::new();
                tokio::select! {
                    req_trx = to_child_rx.recv() => match req_trx {
                        None => return,
                        Some(req_trx) => {
                            let mut jsonrpc_request = req_trx.request.into_jsonrpc_request();
                            let id = last_req_id + 1;
                            jsonrpc_request.id = serde_json::to_value(id).unwrap();

                            last_req_id = id;
                            pending_reqs.insert(id, req_trx);

                            Self::output_message(&mut stdin, jsonrpc_request.into()).await;
                        }
                    },
                    result = stdout.read_line(&mut stdout_message) => match result {
                        Err(e) => error!("StdioClient i/o error reading line from stdout: {}" ,e),
                        Ok(bytes_read) => {
                            if bytes_read == 0 {
                                return;
                            }
                            match JsonRpcMessage::try_from(serde_json::from_str::<Value>(&stdout_message).unwrap_or_default()) {
                                Err(e) => error!("failed to parse message from server: {}", e),
                                Ok(message) => match message {
                                    JsonRpcMessage::Request(request) => Self::output_message(&mut stdin, JsonRpcResponse::new(Err(ProtocolError {
                                        error_type: ProtocolErrorType::BadRequest,
                                        error: Box::new(SerializableProtocolError {
                                            error_type: ProtocolErrorType::BadRequest,
                                            description: "client does not support serving requests".to_string()
                                        })
                                    }), request.id).into()).await,
                                    JsonRpcMessage::Response(response) => match pending_reqs.remove(&serde_json::from_value(response.id.clone()).unwrap_or_default()) {
                                        None => warn!("received response with unknown id, ignoring"),
                                        Some(trx) => {
                                            let result = match Response::from_jsonrpc_message(response.into(), &trx.request) {
                                                Ok(response) => match response {
                                                    None => {
                                                        error!("unknown json rpc notification type received");
                                                        return;
                                                    },
                                                    Some(response) => Ok(ServiceResponse::Single(response))
                                                },
                                                Err(e) => Err(e.into())
                                            };
                                            trx.response_tx.send(result).ok();
                                        }
                                    },
                                    JsonRpcMessage::Notification(notification) => {
                                        let id = notification.method.parse::<u64>().unwrap_or_default();
                                        if let Some(trx) = pending_reqs.remove(&id) {
                                            let (notification_tx, notification_rx) = mpsc::unbounded_channel();
                                            trx.response_tx.send(Ok(ServiceResponse::Multiple(UnboundedReceiverStream::new(notification_rx).boxed()))).ok();
                                            notification_links.insert(id, ClientNotificationLink {
                                                request: trx.request,
                                                notification_tx
                                            });
                                        }
                                        match notification_links.get(&id) {
                                            None => warn!("received notification with unknown id, ignoring"),
                                            Some(link) => match notification.params.is_some() {
                                                true => {
                                                    let result = match Response::from_jsonrpc_message(notification.into(), &link.request) {
                                                        Ok(notification) => match notification {
                                                            None => {
                                                                error!("unknown json rpc notification type received");
                                                                return;
                                                            },
                                                            Some(notification) => Ok(notification)
                                                        },
                                                        Err(e) => Err(e.into())
                                                    };
                                                    link.notification_tx.send(result).ok();
                                                },
                                                false => {
                                                    notification_links.remove(&id);
                                                    pending_reqs.remove(&id);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
        to_child_tx
    }

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
        let to_child_tx = Self::start_comm_task(stdin, stdout);
        Ok(Self {
            _child: Arc::new(child),
            to_child_tx,
            config,
        })
    }
}
