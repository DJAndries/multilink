use std::io::{stdout, Write};

use clap::{command, Parser};
use futures::StreamExt;
use multilink::{
    http::client::HttpClientConfig, stdio::client::StdioClientConfig,
    util::service::build_service_from_config, ServiceResponse,
};
use protocol::{GreetingResponse, Request, Response, SayCustomGreetingRequest, SayHelloRequest};
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

use crate::protocol::GreetingStreamResponse;

mod protocol;

const SERVER_STDIO_COMMAND: &str = "greeting-server";
const SERVER_STDIO_COMMAND_ARGS: [&'static str; 1] = ["stdio-server"];

/// A client for the greeting server.
#[derive(Parser, Debug)]
#[command(about)]
struct Cli {
    /// Binary path for invoking the stdio server.
    #[arg(long, default_value = "./target/debug/examples")]
    stdio_bin_path: Option<String>,

    /// The HTTP base URL for sending requests to the http server.
    #[arg(long, default_value = "http://localhost:8080")]
    http_base_url: String,

    /// Send requests to HTTP server instead of invoking stdio server.
    #[arg(long, default_value_t = false)]
    use_http: bool,

    /// Custom greeting prefix that the server should use.
    #[arg(long)]
    custom_greeting: Option<String>,

    /// Stream each character of the greeting from the server. Custom greetings not supported.
    #[arg(long)]
    stream_hello: bool,

    /// The name of the person that the greeting should greet.
    #[arg(long, default_value = "Bob")]
    name: String,
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

    let stdio_config = Some(StdioClientConfig {
        bin_path: cli.stdio_bin_path,
        ..Default::default()
    });

    let http_config = match cli.use_http {
        true => Some(HttpClientConfig {
            base_url: cli.http_base_url,
            ..Default::default()
        }),
        false => None,
    };

    let mut client_service = build_service_from_config::<Request, Response>(
        SERVER_STDIO_COMMAND,
        &SERVER_STDIO_COMMAND_ARGS,
        stdio_config,
        http_config,
    )
    .await
    .expect("should be able to create client service");

    let request = match cli.custom_greeting {
        Some(greeting) => Request::SayCustomGreeting(SayCustomGreetingRequest {
            name: cli.name,
            greeting,
        }),
        None => {
            let request = SayHelloRequest { name: cli.name };
            match cli.stream_hello {
                false => Request::SayHello(request),
                true => Request::SayHelloStream(request),
            }
        }
    };

    let response = client_service
        .call(request)
        .await
        .expect("client request should succeed");

    match response {
        ServiceResponse::Single(response) => {
            let result = match response {
                Response::SayCustomGreeting(GreetingResponse { result }) => result,
                Response::SayHello(GreetingResponse { result }) => result,
                _ => panic!("response type invalid for single response"),
            };
            println!("Server says: {}", result);
        }
        ServiceResponse::Multiple(mut response_stream) => {
            let mut stdout = stdout();
            print!("Server says: ");
            stdout.flush().unwrap();
            while let Some(result) = response_stream.next().await {
                match result.expect("client stream request should succeed") {
                    Response::SayHelloStream(GreetingStreamResponse { character }) => {
                        print!("{character}");
                        stdout.flush().unwrap();
                    }
                    _ => panic!("response type invalid for streaming response"),
                }
            }
            println!();
        }
    }
}
