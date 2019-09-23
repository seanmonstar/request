mod support;
use support::*;

use reqwest::Client;

#[tokio::test]
async fn auto_headers() {
    let server = server::http(move |req| {
        async move {
            assert_eq!(req.method(), "GET");

            assert_eq!(req.headers()["accept"], "*/*");
            assert_eq!(req.headers()["user-agent"], DEFAULT_USER_AGENT);
            assert_eq!(req.headers()["accept-encoding"], "gzip");

            http::Response::default()
        }
    });

    let url = format!("http://{}/1", server.addr());
    let res = reqwest::get(&url).await.unwrap();

    assert_eq!(res.url().as_str(), &url);
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    assert_eq!(res.remote_addr(), Some(server.addr()));
}

#[tokio::test]
async fn response_text() {
    let _ = env_logger::try_init();

    let server = server::http(move |_req| async { http::Response::new("Hello".into()) });

    let client = Client::new();

    let res = client
        .get(&format!("http://{}/text", server.addr()))
        .send()
        .await
        .expect("Failed to get");
    let text = res.text().await.expect("Failed to get text");
    assert_eq!("Hello", text);
}

#[tokio::test]
async fn response_json() {
    let _ = env_logger::try_init();

    let server = server::http(move |_req| async { http::Response::new("\"Hello\"".into()) });

    let client = Client::new();

    let res = client
        .get(&format!("http://{}/json", server.addr()))
        .send()
        .await
        .expect("Failed to get");
    let text = res.json::<String>().await.expect("Failed to get json");
    assert_eq!("Hello", text);
}

#[tokio::test]
async fn body_pipe_response() {
    let _ = env_logger::try_init();

    let server = server::http(move |mut req| {
        async move {
            if req.uri() == "/get" {
                http::Response::new("pipe me".into())
            } else {
                assert_eq!(req.uri(), "/pipe");
                assert_eq!(req.headers()["transfer-encoding"], "chunked");

                let mut full: Vec<u8> = Vec::new();
                while let Some(item) = req.body_mut().next().await {
                    full.extend(&*item.unwrap());
                }

                assert_eq!(full, b"pipe me");

                http::Response::default()
            }
        }
    });

    let client = Client::new();

    let res1 = client
        .get(&format!("http://{}/get", server.addr()))
        .send()
        .await
        .expect("get1");

    assert_eq!(res1.status(), reqwest::StatusCode::OK);
    assert_eq!(res1.content_length(), Some(7));

    // and now ensure we can "pipe" the response to another request
    let res2 = client
        .post(&format!("http://{}/pipe", server.addr()))
        .body(res1)
        .send()
        .await
        .expect("res2");

    assert_eq!(res2.status(), reqwest::StatusCode::OK);
}
