#![cfg(feature="unstable")]

extern crate env_logger;
extern crate futures;
extern crate tokio_core;
extern crate reqwest;
extern crate libflate;

#[macro_use]
mod support;

use reqwest::unstable::async::Client;
use futures::{Future, Stream};
use tokio_core::reactor::Core;
use std::io::Write;
use std::time::Duration;

#[test]
fn async_test_gzip_response() {
    test_gzip(10_000, 4096);
}

#[test]
fn async_test_gzip_single_byte_chunks() {
    test_gzip(10, 1);
}

fn test_gzip(response_size: usize, chunk_size: usize) {
    let content: String = (0..response_size).into_iter().map(|i| format!("test {}", i)).collect();
    let mut encoder = ::libflate::gzip::Encoder::new(Vec::new()).unwrap();
    match encoder.write(content.as_bytes()) {
        Ok(n) => assert!(n > 0, "Failed to write to encoder."),
        _ => panic!("Failed to gzip encode string."),
    };

    let gzipped_content = encoder.finish().into_result().unwrap();

    let mut response = format!("\
            HTTP/1.1 200 OK\r\n\
            Server: test-accept\r\n\
            Content-Encoding: gzip\r\n\
            Content-Length: {}\r\n\
            \r\n", &gzipped_content.len())
        .into_bytes();
    response.extend(&gzipped_content);

    let server = server! {
        request: b"\
            GET /gzip HTTP/1.1\r\n\
            Host: $HOST\r\n\
            User-Agent: $USERAGENT\r\n\
            Accept: */*\r\n\
            Accept-Encoding: gzip\r\n\
            \r\n\
            ",
        chunk_size: chunk_size,
        write_timeout: Duration::from_millis(10),
        response: response
    };

    let mut core = Core::new().unwrap();

    let client = Client::new(&core.handle());

    let res_future = client.get(&format!("http://{}/gzip", server.addr()))
        .send()
        .and_then(|res| {
            let body = res.into_body();
            body.concat2()
        })
        .and_then(|buf| {
            let body = ::std::str::from_utf8(&buf).unwrap();

            assert_eq!(body, &content);

            Ok(())
        });

    core.run(res_future).unwrap();
}

#[test]
fn test_multipart() {
    let _ = env_logger::try_init();

    let form = reqwest::multipart::Form::new()
        .text("foo", "bar");

    let expected_body = format!("\
        --{0}\r\n\
        Content-Disposition: form-data; name=\"foo\"\r\n\r\n\
        bar\r\n\
        --{0}--\
    ", form.boundary());

    let server = server! {
        request: format!("\
            POST /multipart/1 HTTP/1.1\r\n\
            Host: $HOST\r\n\
            User-Agent: $USERAGENT\r\n\
            Accept: */*\r\n\
            Content-Type: multipart/form-data; boundary={}\r\n\
            Accept-Encoding: gzip\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            7B\r\n\
            {}\
            \r\n\
            0\r\n\
            \r\n\
            \
            ", form.boundary(), expected_body),
        response: b"\
            HTTP/1.1 200 OK\r\n\
            Server: multipart\r\n\
            Content-Length: 0\r\n\
            \r\n\
            "
    };

    let url = format!("http://{}/multipart/1", server.addr());

    let mut core = Core::new().unwrap();

    let res_future = reqwest::unstable::async::Client::new(&core.handle())
        .post(&url)
        .multipart(form)
        .send()
        .and_then(|res| {
            assert_eq!(res.url().as_str(), &url);
            assert_eq!(res.status(), reqwest::StatusCode::Ok);

            Ok(())
        });

    core.run(res_future).unwrap();
}
