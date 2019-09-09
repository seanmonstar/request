use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use encoding_rs::{Encoding, UTF_8};
use futures::{Future, FutureExt, TryStreamExt};
use http;
use hyper::client::connect::HttpInfo;
use hyper::header::CONTENT_LENGTH;
use hyper::{HeaderMap, StatusCode, Version};
use log::debug;
use mime::Mime;
use serde::de::DeserializeOwned;
use serde_json;
use tokio::timer::Delay;
use url::Url;

use super::body::Body;
use super::Decoder;
use crate::async_impl::Chunk;
use crate::cookie;

/// https://github.com/rust-lang-nursery/futures-rs/issues/1812
type ConcatDecoder = Pin<Box<dyn Future<Output = Result<Chunk, crate::Error>> + Send>>;

/// A Response to a submitted `Request`.
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    // Boxed to save space (11 words to 1 word), and it's not accessed
    // frequently internally.
    url: Box<Url>,
    body: Decoder,
    version: Version,
    extensions: http::Extensions,
}

impl Response {
    pub(super) fn new(
        res: hyper::Response<hyper::Body>,
        url: Url,
        gzip: bool,
        timeout: Option<Delay>,
    ) -> Response {
        let (parts, body) = res.into_parts();
        let status = parts.status;
        let version = parts.version;
        let extensions = parts.extensions;

        let mut headers = parts.headers;
        let decoder = Decoder::detect(&mut headers, Body::response(body, timeout), gzip);

        debug!("Response: '{}' for {}", status, url);
        Response {
            status,
            headers,
            url: Box::new(url),
            body: decoder,
            version,
            extensions,
        }
    }

    /// Get the `StatusCode` of this `Response`.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get the `Headers` of this `Response`.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a mutable reference to the `Headers` of this `Response`.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Retrieve the cookies contained in the response.
    ///
    /// Note that invalid 'Set-Cookie' headers will be ignored.
    pub fn cookies<'a>(&'a self) -> impl Iterator<Item = cookie::Cookie<'a>> + 'a {
        cookie::extract_response_cookies(&self.headers).filter_map(Result::ok)
    }

    /// Get the final `Url` of this `Response`.
    #[inline]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Get the remote address used to get this `Response`.
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.extensions
            .get::<HttpInfo>()
            .map(|info| info.remote_addr())
    }

    /// Get the content-length of this response, if known.
    ///
    /// Reasons it may not be known:
    ///
    /// - The server didn't send a `content-length` header.
    /// - The response is gzipped and automatically decoded (thus changing
    ///   the actual decoded length).
    pub fn content_length(&self) -> Option<u64> {
        self.headers()
            .get(CONTENT_LENGTH)
            .and_then(|ct_len| ct_len.to_str().ok())
            .and_then(|ct_len| ct_len.parse().ok())
    }

    /// Consumes the response, returning the body
    pub fn into_body(self) -> Decoder {
        self.body
    }

    /// Get a reference to the response body.
    #[inline]
    pub fn body(&self) -> &Decoder {
        &self.body
    }

    /// Get a mutable reference to the response body.
    ///
    /// The chunks from the body may be decoded, depending on the `gzip`
    /// option on the `ClientBuilder`.
    #[inline]
    pub fn body_mut(&mut self) -> &mut Decoder {
        &mut self.body
    }

    /// Get the HTTP `Version` of this `Response`.
    #[inline]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Get the response text
    pub fn text(&mut self) -> impl Future<Output = Result<String, crate::Error>> {
        self.text_with_charset("utf-8")
    }

    /// Get the response text given a specific encoding
    pub fn text_with_charset(
        &mut self,
        default_encoding: &str,
    ) -> impl Future<Output = Result<String, crate::Error>> {
        let body = mem::replace(&mut self.body, Decoder::empty());
        let content_type = self
            .headers
            .get(crate::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<Mime>().ok());
        let encoding_name = content_type
            .as_ref()
            .and_then(|mime| mime.get_param("charset").map(|charset| charset.as_str()))
            .unwrap_or(default_encoding);
        let encoding = Encoding::for_label(encoding_name.as_bytes()).unwrap_or(UTF_8);
        Text {
            concat: body.try_concat().boxed(),
            encoding,
        }
    }

    /// Try to deserialize the response body as JSON using `serde`.
    #[inline]
    pub fn json<T: DeserializeOwned>(&mut self) -> impl Future<Output = Result<T, crate::Error>> {
        let body = mem::replace(&mut self.body, Decoder::empty());

        Json {
            concat: body.try_concat().boxed(),
            _marker: PhantomData,
        }
    }

    /// Turn a response into an error if the server returned an error.
    ///
    /// # Example
    ///
    /// ```
    /// # use reqwest::Response;
    /// fn on_response(res: Response) {
    ///     match res.error_for_status() {
    ///         Ok(_res) => (),
    ///         Err(err) => {
    ///             // asserting a 400 as an example
    ///             // it could be any status between 400...599
    ///             assert_eq!(
    ///                 err.status(),
    ///                 Some(reqwest::StatusCode::BAD_REQUEST)
    ///             );
    ///         }
    ///     }
    /// }
    /// # fn main() {}
    /// ```
    #[inline]
    pub fn error_for_status(self) -> crate::Result<Self> {
        if self.status.is_client_error() || self.status.is_server_error() {
            Err(crate::error::status_code(*self.url, self.status))
        } else {
            Ok(self)
        }
    }

    /// Turn a reference to a response into an error if the server returned an error.
    ///
    /// # Example
    ///
    /// ```
    /// # use reqwest::Response;
    /// fn on_response(res: &Response) {
    ///     match res.error_for_status_ref() {
    ///         Ok(_res) => (),
    ///         Err(err) => {
    ///             // asserting a 400 as an example
    ///             // it could be any status between 400...599
    ///             assert_eq!(
    ///                 err.status(),
    ///                 Some(reqwest::StatusCode::BAD_REQUEST)
    ///             );
    ///         }
    ///     }
    /// }
    /// # fn main() {}
    /// ```
    #[inline]
    pub fn error_for_status_ref(&self) -> crate::Result<&Self> {
        if self.status.is_client_error() || self.status.is_server_error() {
            Err(crate::error::status_code(*self.url.clone(), self.status))
        } else {
            Ok(self)
        }
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Response")
            .field("url", self.url())
            .field("status", &self.status())
            .field("headers", self.headers())
            .finish()
    }
}

impl<T: Into<Body>> From<http::Response<T>> for Response {
    fn from(r: http::Response<T>) -> Response {
        let (mut parts, body) = r.into_parts();
        let body = body.into();
        let body = Decoder::detect(&mut parts.headers, body, false);
        let url = parts
            .extensions
            .remove::<ResponseUrl>()
            .unwrap_or_else(|| ResponseUrl(Url::parse("http://no.url.provided.local").unwrap()));
        let url = url.0;
        Response {
            status: parts.status,
            headers: parts.headers,
            url: Box::new(url),
            body,
            version: parts.version,
            extensions: parts.extensions,
        }
    }
}

/// A JSON object.
struct Json<T> {
    concat: ConcatDecoder,
    _marker: PhantomData<T>,
}

impl<T> Json<T> {
    fn concat(self: Pin<&mut Self>) -> Pin<&mut ConcatDecoder> {
        unsafe { Pin::map_unchecked_mut(self, |x| &mut x.concat) }
    }
}

impl<T: DeserializeOwned> Future for Json<T> {
    type Output = Result<T, crate::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match futures::ready!(self.concat().as_mut().poll(cx)) {
            Err(e) => Poll::Ready(Err(e)),
            Ok(chunk) => {
                let t = serde_json::from_slice(&chunk).map_err(crate::error::from);
                Poll::Ready(t)
            }
        }
    }
}

impl<T> fmt::Debug for Json<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Json").finish()
    }
}

//#[derive(Debug)]
struct Text {
    concat: ConcatDecoder,
    encoding: &'static Encoding,
}

impl Text {
    fn concat(self: Pin<&mut Self>) -> Pin<&mut ConcatDecoder> {
        unsafe { Pin::map_unchecked_mut(self, |x| &mut x.concat) }
    }
}

impl Future for Text {
    type Output = Result<String, crate::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match futures::ready!(self.as_mut().concat().as_mut().poll(cx)) {
            Err(e) => Poll::Ready(Err(e)),
            Ok(chunk) => {
                let (text, _, _) = self.as_mut().encoding.decode(&chunk);
                if let Cow::Owned(s) = text {
                    return Poll::Ready(Ok(s));
                }
                unsafe {
                    // decoding returned Cow::Borrowed, meaning these bytes
                    // are already valid utf8
                    Poll::Ready(Ok(String::from_utf8_unchecked(chunk.to_vec())))
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ResponseUrl(Url);

/// Extension trait for http::response::Builder objects
///
/// Allows the user to add a `Url` to the http::Response
pub trait ResponseBuilderExt {
    /// A builder method for the `http::response::Builder` type that allows the user to add a `Url`
    /// to the `http::Response`
    fn url(&mut self, url: Url) -> &mut Self;
}

impl ResponseBuilderExt for http::response::Builder {
    fn url(&mut self, url: Url) -> &mut Self {
        self.extension(ResponseUrl(url))
    }
}

#[cfg(test)]
mod tests {
    use super::{Response, ResponseBuilderExt, ResponseUrl};
    use http::response::Builder;
    use url::Url;

    #[test]
    fn test_response_builder_ext() {
        let url = Url::parse("http://example.com").unwrap();
        let response = Builder::new()
            .status(200)
            .url(url.clone())
            .body(())
            .unwrap();

        assert_eq!(
            response.extensions().get::<ResponseUrl>(),
            Some(&ResponseUrl(url))
        );
    }

    #[test]
    fn test_from_http_response() {
        let url = Url::parse("http://example.com").unwrap();
        let response = Builder::new()
            .status(200)
            .url(url.clone())
            .body("foo")
            .unwrap();
        let response = Response::from(response);

        assert_eq!(response.status, 200);
        assert_eq!(response.url, Box::new(url));
    }
}
