use std::fmt;
use std::mem;
use std::marker::PhantomData;
use std::net::SocketAddr;

use futures::{Async, Future, Poll, Stream};
use futures::stream::Concat2;
use hyper::{HeaderMap, StatusCode, Version};
use hyper::client::connect::HttpInfo;
use hyper::header::{CONTENT_LENGTH};
use serde::de::DeserializeOwned;
use serde_json;
use url::Url;
use http;

use cookie;
use super::Decoder;
use super::body::Body;


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
    pub(super) fn new(res: ::hyper::Response<::hyper::Body>, url: Url, gzip: bool) -> Response {
        let (parts, body) = res.into_parts();
        let status = parts.status;
        let version = parts.version;
        let extensions = parts.extensions;

        let mut headers = parts.headers;
        let decoder = Decoder::detect(&mut headers, Body::wrap(body), gzip);

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
    pub fn cookies<'a>(&'a self) -> impl Iterator<
        Item = Result<cookie::Cookie<'a>, cookie::CookieParseError>
    > + 'a {
        cookie::extract_response_cookies(&self.headers)
    }

    /// Get the final `Url` of this `Response`.
    #[inline]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Get the remote address used to get this `Response`.
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self
            .extensions
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
        self
            .headers()
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


    /// Try to deserialize the response body as JSON using `serde`.
    #[inline]
    pub fn json<T: DeserializeOwned>(&mut self) -> impl Future<Item = T, Error = ::Error> {
        let body = mem::replace(&mut self.body, Decoder::empty());

        Json {
            concat: body.concat2(),
            _marker: PhantomData,
        }
    }

    /// Turn a response into an error if the server returned an error.
    ///
    /// # Example
    ///
    /// ```
    /// # use reqwest::async::Response;
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
    pub fn error_for_status(self) -> ::Result<Self> {
        if self.status.is_client_error() {
            Err(::error::client_error(*self.url, self.status))
        } else if self.status.is_server_error() {
            Err(::error::server_error(*self.url, self.status))
        } else {
            Ok(self)
        }
    }

    /// Turn a reference to a response into an error if the server returned an error.
    ///
    /// # Example
    ///
    /// ```
    /// # use reqwest::async::Response;
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
    pub fn error_for_status_ref(&self) -> ::Result<&Self> {
        if self.status.is_client_error() {
            Err(::error::client_error(*self.url.clone(), self.status))
        } else if self.status.is_server_error() {
            Err(::error::server_error(*self.url.clone(), self.status))
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
        let url = parts.extensions
            .remove::<ResponseUrl>()
            .unwrap_or_else(|| ResponseUrl(Url::parse("http://no.url.provided.local").unwrap()));
        let url = url.0;
        Response {
            status: parts.status,
            headers: parts.headers,
            url: Box::new(url),
            body: body,
            version: parts.version,
            extensions: parts.extensions,
        }
    }
}

/// A JSON object.
pub struct Json<T> {
    concat: Concat2<Decoder>,
    _marker: PhantomData<T>,
}

impl<T: DeserializeOwned> Future for Json<T> {
    type Item = T;
    type Error = ::Error;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let bytes = try_ready!(self.concat.poll());
        let t = try_!(serde_json::from_slice(&bytes));
        Ok(Async::Ready(t))
    }
}

impl<T> fmt::Debug for Json<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Json")
            .finish()
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
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate url;
    /// # extern crate http;
    /// # extern crate reqwest;
    /// # use std::error::Error;
    /// use url::Url;
    /// use http::response::Builder;
    /// use reqwest::async::ResponseBuilderExt;
    /// # fn main() -> Result<(), Box<Error>> {
    /// let response = Builder::new()
    ///     .status(200)
    ///     .url(Url::parse("http://example.com")?)
    ///     .body(())?;
    ///
    /// #   Ok(())
    /// # }
    fn url(&mut self, url: Url) -> &mut Self;
}

impl ResponseBuilderExt for http::response::Builder {
    fn url(&mut self, url: Url) -> &mut Self {
        self.extension(ResponseUrl(url))
    }
}

#[cfg(test)]
mod tests {
    use url::Url;
    use http::response::Builder;
    use super::{Response, ResponseUrl, ResponseBuilderExt};

    #[test]
    fn test_response_builder_ext() {
        let url = Url::parse("http://example.com").unwrap();
        let response = Builder::new()
            .status(200)
            .url(url.clone())
            .body(())
            .unwrap();

        assert_eq!(response.extensions().get::<ResponseUrl>(), Some(&ResponseUrl(url)));
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
