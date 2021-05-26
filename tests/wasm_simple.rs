#![cfg(target_arch = "wasm32")]

use std::time::{Duration};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen]
extern "C" {
    // Use `js_namespace` here to bind `console.log(..)` instead of just
    // `log(..)`
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

}
#[wasm_bindgen_test]
async fn simple_example() {
    let res = reqwest::get("https://hyper.rs")
        .await
        .expect("http get example");

    log(&format!("Status: {}", res.status()));

    let body = res.text().await.expect("response to utf-8 text");
    log(&format!("Body:\n\n{}", body));
}

#[wasm_bindgen_test]
async fn client_with_timeout() {
    let client = reqwest::Client::new();
    let err = client.get("https://hyper.rs")
        .timeout(Duration::from_millis(10))
        .send()
        .await
        .expect_err("Expected error from aborted request");

    assert_eq!(err.is_request(), true);
    assert_eq!(format!("{:?}", err).contains("The user aborted a request."), true);
}
