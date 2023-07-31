mod protocol;

use std::{
    task::{Context, Poll},
    time::Duration,
};

use async_stream::stream;
use clap::{command, Parser, Subcommand};
use futures::StreamExt;
use multilink::{
    http::server::{HttpServer, HttpServerConfig},
    stdio::server::StdioServer,
    ServiceError, ServiceFuture, ServiceResponse,
};
use protocol::{GreetingResponse, GreetingStreamResponse, Request, Response};
use tokio::time::sleep;
use tower::Service;
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a HTTP server.
    HttpServer,
    /// Run a stdio/json-rpc server.
    StdioServer,
}

/// A server that sends greetings to clients.
#[derive(Parser, Debug)]
#[command(about)]
struct Cli {
    /// The type of server to run.
    #[command(subcommand)]
    server_type: Command,

    /// The port that the HTTP server should listen on.
    #[arg(long, default_value_t = 8080)]
    http_listen_port: u16,
}

#[derive(Clone)]
struct GreetingService;

impl Service<Request> for GreetingService {
    type Response = ServiceResponse<Response>;
    type Error = ServiceError;
    type Future = ServiceFuture<ServiceResponse<Response>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        Box::pin(async move {
            Ok(match req {
                Request::SayHello(request) => {
                    ServiceResponse::Single(Response::SayHello(GreetingResponse {
                        result: format!("Hello, {}!", request.name),
                    }))
                }
                Request::SayCustomGreeting(request) => {
                    ServiceResponse::Single(Response::SayCustomGreeting(GreetingResponse {
                        result: format!("{}, {}!", request.greeting, request.name),
                    }))
                }
                Request::SayHelloStream(request) => ServiceResponse::Multiple(
                    stream! {
                        let result = format!("Hello, {}!", request.name);
                        for character in result.chars() {
                            yield Ok(Response::SayHelloStream(GreetingStreamResponse { character }));
                            sleep(Duration::from_millis(300)).await;
                        }
                    }
                    .boxed(),
                ),
            })
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()
                .unwrap(),
        )
        .init();

    let cli = Cli::parse();

    let service = GreetingService;

    match cli.server_type {
        Command::HttpServer => HttpServer::new(
            service,
            HttpServerConfig {
                port: cli.http_listen_port,
                ..Default::default()
            },
        )
        .run()
        .await
        .expect("http server should not fail"),
        Command::StdioServer => StdioServer::new(service, Default::default())
            .run()
            .await
            .expect("stdio server should not fail"),
    };
}
