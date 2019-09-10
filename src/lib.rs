#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![cfg_attr(test, deny(warnings))]
#![doc(html_root_url = "https://docs.rs/reqwest/0.9.19")]

//! # reqwest
//!
//! The `reqwest` crate provides a convenient, higher-level HTTP
//! [`Client`][client].
//!
//! It handles many of the things that most people just expect an HTTP client
//! to do for them.
//!
//! - Async and [blocking](blocking) Clients
//! - Plain bodies, [JSON](#json), [urlencoded](#forms), [multipart](multipart)
//! - Customizable [redirect policy](#redirect-policy)
//! - HTTP [Proxies](#proxies)
//! - Uses system-native [TLS](#tls)
//! - Cookies
//!
//! The [`reqwest::Client`][client] is asynchronous. For applications wishing
//! to only make a few HTTP requests, the [`reqwest::blocking`](blocking) API
//! may be more convenient.
//!
//! Additional learning resources include:
//!
//! - [The Rust Cookbook](https://rust-lang-nursery.github.io/rust-cookbook/web/clients.html)
//! - [Reqwest Repository Examples](https://github.com/seanmonstar/reqwest/tree/master/examples)
//!
//! ## Making a GET request
//!
//! For a single request, you can use the [`get`][get] shortcut method.
//!
//! ```rust
//! # async fn run() -> Result<(), reqwest::Error> {
//! let body = reqwest::get("https://www.rust-lang.org")
//!     .await?
//!     .text()
//!     .await?;
//!
//! println!("body = {:?}", body);
//! # Ok(())
//! # }
//! ```
//!
//! **NOTE**: If you plan to perform multiple requests, it is best to create a
//! [`Client`][client] and reuse it, taking advantage of keep-alive connection
//! pooling.
//!
//! ## Making POST requests (or setting request bodies)
//!
//! There are several ways you can set the body of a request. The basic one is
//! by using the `body()` method of a [`RequestBuilder`][builder]. This lets you set the
//! exact raw bytes of what the body should be. It accepts various types,
//! including `String`, `Vec<u8>`, and `File`. If you wish to pass a custom
//! type, you can use the `reqwest::Body` constructors.
//!
//! ```rust
//! # use reqwest::Error;
//! #
//! # async fn run() -> Result<(), Error> {
//! let client = reqwest::Client::new();
//! let res = client.post("http://httpbin.org/post")
//!     .body("the exact body that is sent")
//!     .send()
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Forms
//!
//! It's very common to want to send form data in a request body. This can be
//! done with any type that can be serialized into form data.
//!
//! This can be an array of tuples, or a `HashMap`, or a custom type that
//! implements [`Serialize`][serde].
//!
//! ```rust
//! # use reqwest::Error;
//! #
//! # async fn run() -> Result<(), Error> {
//! // This will POST a body of `foo=bar&baz=quux`
//! let params = [("foo", "bar"), ("baz", "quux")];
//! let client = reqwest::Client::new();
//! let res = client.post("http://httpbin.org/post")
//!     .form(&params)
//!     .send()
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### JSON
//!
//! There is also a `json` method helper on the [`RequestBuilder`][builder] that works in
//! a similar fashion the `form` method. It can take any value that can be
//! serialized into JSON.
//!
//! ```rust
//! # use reqwest::Error;
//! # use std::collections::HashMap;
//! #
//! # async fn run() -> Result<(), Error> {
//! // This will POST a body of `{"lang":"rust","body":"json"}`
//! let mut map = HashMap::new();
//! map.insert("lang", "rust");
//! map.insert("body", "json");
//!
//! let client = reqwest::Client::new();
//! let res = client.post("http://httpbin.org/post")
//!     .json(&map)
//!     .send()
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Redirect Policies
//!
//! By default, a `Client` will automatically handle HTTP redirects, detecting
//! loops, and having a maximum redirect chain of 10 hops. To customize this
//! behavior, a [`RedirectPolicy`][redirect] can used with a `ClientBuilder`.
//!
//! ## Cookies
//!
//! The automatic storing and sending of session cookies can be enabled with
//! the [`cookie_store`][ClientBuilder::cookie_store] method on `ClientBuilder`.
//!
//! ## Proxies
//!
//! A `Client` can be configured to make use of HTTP proxies by adding
//! [`Proxy`](Proxy)s to a `ClientBuilder`.
//!
//! ** NOTE** System proxies will be used in the next breaking change.
//!
//! ## TLS
//!
//! By default, a `Client` will make use of system-native transport layer
//! security to connect to HTTPS destinations. This means schannel on Windows,
//! Security-Framework on macOS, and OpenSSL on Linux.
//!
//! - Additional X509 certificates can be configured on a `ClientBuilder` with the
//!   [`Certificate`](Certificate) type.
//! - Client certificates can be add to a `ClientBuilder` with the
//!   [`Identity`][Identity] type.
//! - Various parts of TLS can also be configured or even disabled on the
//!   `ClientBuilder`.
//!
//! ## Optional Features
//!
//! The following are a list of [Cargo features][cargo-features] that can be
//! enabled or disabled:
//!
//! - **default-tls** *(enabled by default)*: Provides TLS support via the
//!   `native-tls` library to connect over HTTPS.
//! - **default-tls-vendored**: Enables the `vendored` feature of `native-tls`.
//! - **rustls-tls**: Provides TLS support via the `rustls` library.
//!
//!
//! [hyper]: http://hyper.rs
//! [client]: ./struct.Client.html
//! [response]: ./struct.Response.html
//! [get]: ./fn.get.html
//! [builder]: ./struct.RequestBuilder.html
//! [serde]: http://serde.rs
//! [redirect]: ./struct.RedirectPolicy.html
//! [Proxy]: ./struct.Proxy.html
//! [cargo-features]: https://doc.rust-lang.org/stable/cargo/reference/manifest.html#the-features-section

////! - **socks**: Provides SOCKS5 proxy support.
////! - **trust-dns**: Enables a trust-dns async resolver instead of default
////!   threadpool using `getaddrinfo`.

extern crate cookie as cookie_crate;

#[cfg(test)]
#[macro_use]
extern crate doc_comment;

#[cfg(test)]
doctest!("../README.md");

pub use hyper::header;
pub use hyper::Method;
pub use hyper::{StatusCode, Version};
pub use url::ParseError as UrlError;
pub use url::Url;

pub use self::async_impl::{
    multipart, Body, Client, ClientBuilder, Request, RequestBuilder, Response,
};
//pub use self::body::Body;
//pub use self::client::{Client, ClientBuilder};
pub use self::error::{Error, Result};
pub use self::into_url::IntoUrl;
pub use self::proxy::Proxy;
pub use self::redirect::{RedirectAction, RedirectAttempt, RedirectPolicy};
//pub use self::request::{Request, RequestBuilder};
//pub use self::response::Response;
#[cfg(feature = "tls")]
pub use self::tls::{Certificate, Identity};

// this module must be first because of the `try_` macro
#[macro_use]
mod error;

mod async_impl;
pub mod blocking;
mod connect;
pub mod cookie;
//#[cfg(feature = "trust-dns")]
//mod dns;
mod into_url;
mod proxy;
mod redirect;
#[cfg(feature = "tls")]
mod tls;

//pub mod multipart;

#[doc(hidden)]
#[deprecated(note = "types moved to top of crate")]
pub mod r#async {
    pub use crate::async_impl::{
        multipart, Body, Client, ClientBuilder, Request, RequestBuilder, Response,
    };
}

/// Shortcut method to quickly make a `GET` request.
///
/// See also the methods on the [`reqwest::Response`](./struct.Response.html)
/// type.
///
/// **NOTE**: This function creates a new internal `Client` on each call,
/// and so should not be used if making many requests. Create a
/// [`Client`](./struct.Client.html) instead.
///
/// # Examples
///
/// ```rust
/// # async fn run() -> Result<(), reqwest::Error> {
/// let body = reqwest::get("https://www.rust-lang.org").await?
///     .text().await?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// This function fails if:
///
/// - native TLS backend cannot be initialized
/// - supplied `Url` cannot be parsed
/// - there was an error while sending request
/// - redirect loop was detected
/// - redirect limit was exhausted
pub async fn get<T: IntoUrl>(url: T) -> crate::Result<Response> {
    Client::builder().build()?.get(url).send().await
}

fn _assert_impls() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    fn assert_clone<T: Clone>() {}

    assert_send::<Client>();
    assert_sync::<Client>();
    assert_clone::<Client>();

    assert_send::<Request>();
    assert_send::<RequestBuilder>();

    assert_send::<Response>();

    assert_send::<Error>();
    assert_sync::<Error>();
}
