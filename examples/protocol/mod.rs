mod convert;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct SayHelloRequest {
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SayCustomGreetingRequest {
    pub name: String,
    pub greeting: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Request {
    SayHello(SayHelloRequest),
    SayCustomGreeting(SayCustomGreetingRequest),
    SayHelloStream(SayHelloRequest),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GreetingResponse {
    pub result: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GreetingStreamResponse {
    pub character: char,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    SayHello(GreetingResponse),
    SayCustomGreeting(GreetingResponse),
    SayHelloStream(GreetingStreamResponse),
}
