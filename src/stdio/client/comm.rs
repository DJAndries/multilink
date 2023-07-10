use std::collections::HashMap;

use futures::StreamExt;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, warn};

use crate::{
    jsonrpc::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse},
    stdio::StdioError,
    ServiceResponse,
};

use super::{
    serialize_payload, ClientNotificationLink, ClientRequestTrx, RequestJsonRpcConvert,
    ResponseJsonRpcConvert,
};

pub(super) struct StdioClientCommTask<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
{
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pending_reqs: HashMap<u64, ClientRequestTrx<Request, Response>>,
    notification_links: HashMap<u64, ClientNotificationLink<Request, Response>>,
    to_child_rx: UnboundedReceiver<ClientRequestTrx<Request, Response>>,
    to_child_tx: Option<UnboundedSender<ClientRequestTrx<Request, Response>>>,
    last_req_id: u64,
}

impl<Request, Response> StdioClientCommTask<Request, Response>
where
    Request: RequestJsonRpcConvert<Request> + Send + 'static,
    Response: ResponseJsonRpcConvert<Request, Response> + Send + 'static,
{
    pub(super) fn new(stdin: ChildStdin, stdout: BufReader<ChildStdout>) -> Self {
        let (to_child_tx, to_child_rx) =
            mpsc::unbounded_channel::<ClientRequestTrx<Request, Response>>();
        Self {
            stdin,
            stdout,
            pending_reqs: HashMap::new(),
            notification_links: HashMap::new(),
            to_child_rx,
            to_child_tx: Some(to_child_tx),
            last_req_id: 0,
        }
    }

    async fn output_message(&mut self, message: JsonRpcMessage) {
        let serialized_response = serialize_payload(&message);
        self.stdin
            .write_all(serialized_response.as_bytes())
            .await
            .ok();
    }

    async fn handle_outgoing_request(&mut self, req_trx: ClientRequestTrx<Request, Response>) {
        let mut jsonrpc_request = req_trx.request.into_jsonrpc_request();
        let id = self.last_req_id + 1;
        jsonrpc_request.id = serde_json::to_value(id).unwrap();

        self.last_req_id = id;
        self.pending_reqs.insert(id, req_trx);

        self.output_message(jsonrpc_request.into()).await;
    }

    async fn handle_incoming_request(&mut self, request: JsonRpcRequest) {
        self.output_message(
            JsonRpcResponse::new(Err(StdioError::ClientRequestUnsupported.into()), request.id)
                .into(),
        )
        .await
    }

    fn handle_response(&mut self, response: JsonRpcResponse) {
        match self
            .pending_reqs
            .remove(&serde_json::from_value(response.id.clone()).unwrap_or_default())
        {
            None => {
                warn!("received response with unknown id, ignoring {:?}", response)
            }
            Some(trx) => {
                let result = match Response::from_jsonrpc_message(response.into(), &trx.request) {
                    Ok(response) => match response {
                        None => {
                            error!("unknown json rpc notification type received");
                            return;
                        }
                        Some(response) => Ok(ServiceResponse::Single(response)),
                    },
                    Err(e) => Err(e.into()),
                };
                trx.response_tx.send(result).ok();
            }
        }
    }

    fn handle_notification(&mut self, notification: JsonRpcNotification) {
        let id = notification.method.parse::<u64>().unwrap_or_default();
        if let Some(trx) = self.pending_reqs.remove(&id) {
            let (notification_tx, notification_rx) = mpsc::unbounded_channel();
            trx.response_tx
                .send(Ok(ServiceResponse::Multiple(
                    UnboundedReceiverStream::new(notification_rx).boxed(),
                )))
                .ok();
            self.notification_links.insert(
                id,
                ClientNotificationLink {
                    request: trx.request,
                    notification_tx,
                },
            );
        }
        match self.notification_links.get(&id) {
            None => warn!("received notification with unknown id, ignoring"),
            Some(link) => match notification.params.is_some() {
                true => {
                    let result =
                        match Response::from_jsonrpc_message(notification.into(), &link.request) {
                            Ok(notification) => match notification {
                                None => {
                                    error!("unknown json rpc notification type received");
                                    return;
                                }
                                Some(notification) => Ok(notification),
                            },
                            Err(e) => Err(e.into()),
                        };
                    link.notification_tx.send(result).ok();
                }
                false => {
                    self.notification_links.remove(&id);
                    self.pending_reqs.remove(&id);
                }
            },
        }
    }

    async fn run(mut self) {
        loop {
            let mut stdout_message = String::new();
            tokio::select! {
                req_trx = self.to_child_rx.recv() => if let Some(req_trx) = req_trx {
                    self.handle_outgoing_request(req_trx).await;
                },
                result = self.stdout.read_line(&mut stdout_message) => match result {
                    Err(e) => error!("StdioClient i/o error reading line from stdout: {}" ,e),
                    Ok(bytes_read) => {
                        if bytes_read == 0 {
                            return;
                        }
                        match JsonRpcMessage::try_from(serde_json::from_str::<Value>(&stdout_message).unwrap_or_default()) {
                            Err(e) => error!("failed to parse message from server: {}", e),
                            Ok(message) => match message {
                                JsonRpcMessage::Request(request) => self.handle_incoming_request(request).await,
                                JsonRpcMessage::Response(response) => self.handle_response(response),
                                JsonRpcMessage::Notification(notification) => self.handle_notification(notification)
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn start(mut self) -> UnboundedSender<ClientRequestTrx<Request, Response>> {
        let to_child_tx = self.to_child_tx.take().unwrap();
        tokio::spawn(async move {
            self.run().await;
        });
        to_child_tx
    }
}
