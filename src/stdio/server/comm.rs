use std::pin::Pin;

use futures::Future;
use serde_json::Value;
use tokio::{
    io::{AsyncWriteExt, Stdout},
    sync::Mutex,
};
use tower::{timeout::future::ResponseFuture, Service};
use tracing::error;

use crate::{
    jsonrpc::{JsonRpcMessage, JsonRpcNotification, JsonRpcResponse},
    ServiceError, ServiceFuture, ServiceResponse,
};

use super::{
    serialize_payload, IdentifiedNotification, RequestJsonRpcConvert, ResponseJsonRpcConvert,
    ServerNotificationLink, StdioServer,
};

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
    async fn output_message(stdout: &Mutex<Stdout>, message: JsonRpcMessage) {
        let serialized_message = serialize_payload(&message);
        stdout
            .lock()
            .await
            .write_all(serialized_message.as_bytes())
            .await
            .ok();
    }

    pub(super) fn handle_response_future(
        &self,
        result_future: ResponseFuture<
            Pin<Box<dyn Future<Output = Result<ServiceResponse<Response>, ServiceError>> + Send>>,
        >,
        id: u64,
    ) {
        let stdout = self.stdout.clone();
        let notification_streams_tx = self
            .notification_streams_tx
            .clone()
            .expect("notfication_streams_tx should be initialized");

        tokio::spawn(async move {
            let result = result_future.await;
            match result {
                Ok(response) => match response {
                    ServiceResponse::Single(response) => {
                        let message = Response::into_jsonrpc_message(response, id.into());
                        Self::output_message(stdout.as_ref(), message).await;
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
                        JsonRpcResponse::new(Err(e.into()), id.into()).into(),
                    )
                    .await
                }
            }
        });
    }

    pub(super) fn handle_request(&mut self, serialized_request: String) {
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
        self.handle_response_future(result_future, id)
    }

    pub(super) async fn handle_notification(
        &self,
        id_notification: IdentifiedNotification<Response>,
    ) {
        match id_notification.result {
            Some(result) => {
                let id = id_notification.id.into();
                let message = match result {
                    Ok(response) => Response::into_jsonrpc_message(response, id).into(),
                    Err(e) => {
                        JsonRpcNotification::new_with_result_params(Err(e), id.to_string()).into()
                    }
                };
                Self::output_message(self.stdout.as_ref(), message).await;
            }
            None => {
                // Send value with `None` params to let client know that the stream
                // has terminated.
                Self::output_message(
                    self.stdout.as_ref(),
                    JsonRpcNotification::new(id_notification.id.to_string(), None).into(),
                )
                .await;
            }
        }
    }
}
