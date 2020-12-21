//! `cargo run --example big_file --features=stream`
//! This example illustrates the way to send and receive big file by chunks.
#![deny(warnings)]

use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Body,
};
use tokio::{
    fs::File,
    io::{self, AsyncWriteExt},
    stream::StreamExt,
};
use tokio_util::codec::{BytesCodec, FramedRead};

static FILE_PATH: &str = "examples/big_file.json";

// This is using the `tokio` runtime. You'll need the following dependency:
//
// `tokio = { version = "0.2", features = ["fs", "io-std", "macros"] }`
//
// for reading file by chunks this example uses tokio-util dependency:
//
// `tokio-util = { version = "0.3", features = ["codec"] }`
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(FILE_PATH).await?;
    let stream = FramedRead::new(file, BytesCodec::new());
    let body = Body::wrap_stream(stream);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut response_stream = reqwest::Client::new()
        .post("https://jsonplaceholder.typicode.com/posts")
        .headers(headers)
        .body(body)
        .send()
        .await?
        .bytes_stream();
    while let Some(chunk) = response_stream.next().await {
        let chunk = chunk?;
        io::stdout().write_all(&chunk).await?;
    }
    Ok(())
}
