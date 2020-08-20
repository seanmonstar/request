use crate::async_impl;
use crate::header::CONTENT_TYPE;

use super::body::{self, Body};
use super::multipart;

/// A request which can be executed with `Client::execute()`.
pub type Request = crate::core::request::Request<super::Body>;
/// A builder to construct the properties of a `Request`.
pub type RequestBuilder = crate::core::request::RequestBuilder<super::Client, super::Body>;

impl Request {
    /// Attempts to clone the `Request`.
    ///
    /// None is returned if a body is which can not be cloned. This can be because the body is a
    /// stream.
    pub fn try_clone(&self) -> Option<Request> {
        let body = if let Some(ref body) = self.body.as_ref() {
            if let Some(body) = body.try_clone() {
                Some(body)
            } else {
                return None;
            }
        } else {
            None
        };
        let mut req = Request::new(self.method().clone(), self.url().clone());
        *req.headers_mut() = self.headers().clone();
        req.body = body;
        Some(req)
    }

    pub(crate) fn into_async(self) -> (async_impl::Request, Option<body::Sender>) {
        use crate::header::CONTENT_LENGTH;

        if let Some((tx, body, len)) = self.body.map(|body| body.into_async()) {
            let mut new_headers = self.headers;
            if let Some(len) = len {
                new_headers.insert(CONTENT_LENGTH, len.into());
            }
            let req_async = async_impl::Request {
                method: self.method,
                url: self.url,
                headers: new_headers,
                body: Some(body),
                timeout: self.timeout,
                cors: self.cors,
            };
            (req_async, tx)
        } else {
            let req_async = async_impl::Request {
                method: self.method,
                url: self.url,
                headers: self.headers,
                body: None,
                timeout: self.timeout,
                cors: self.cors,
            };
            (req_async, None)
        }
    }
}

impl RequestBuilder {
    /// Sends a multipart/form-data body.
    ///
    /// ```
    /// # use reqwest::Error;
    ///
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = reqwest::blocking::Client::new();
    /// let form = reqwest::blocking::multipart::Form::new()
    ///     .text("key3", "value3")
    ///     .file("file", "/path/to/field")?;
    ///
    /// let response = client.post("your url")
    ///     .multipart(form)
    ///     .send()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`multipart`](multipart/) for more examples.
    pub fn multipart(self, mut multipart: multipart::Form) -> RequestBuilder {
        let mut builder = self.header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", multipart.boundary()).as_str(),
        );
        if let Ok(ref mut req) = builder.request {
            *req.body_mut() = Some(match multipart.compute_length() {
                Some(length) => Body::sized(multipart.reader(), length),
                None => Body::new(multipart.reader()),
            })
        }
        builder
    }

    /// Constructs the Request and sends it the target URL, returning a Response.
    ///
    /// # Errors
    ///
    /// This method fails if there was an error while sending request,
    /// redirect loop was detected or redirect limit was exhausted.
    pub fn send(self) -> crate::Result<super::Response> {
        self.client.execute(self.request?)
    }

    /// Attempts to clone the `RequestBuilder`.
    ///
    /// None is returned if a body is which can not be cloned. This can be because the body is a
    /// stream.
    ///
    /// # Examples
    ///
    /// With a static body
    ///
    /// ```rust
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = reqwest::blocking::Client::new();
    /// let builder = client.post("http://httpbin.org/post")
    ///     .body("from a &str!");
    /// let clone = builder.try_clone();
    /// assert!(clone.is_some());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Without a body
    ///
    /// ```rust
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = reqwest::blocking::Client::new();
    /// let builder = client.get("http://httpbin.org/get");
    /// let clone = builder.try_clone();
    /// assert!(clone.is_some());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// With a non-clonable body
    ///
    /// ```rust
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = reqwest::blocking::Client::new();
    /// let builder = client.get("http://httpbin.org/get")
    ///     .body(reqwest::blocking::Body::new(std::io::empty()));
    /// let clone = builder.try_clone();
    /// assert!(clone.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_clone(&self) -> Option<RequestBuilder> {
        self.request
            .as_ref()
            .ok()
            .and_then(|req| req.try_clone())
            .map(|req| RequestBuilder {
                client: self.client.clone(),
                request: Ok(req),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::convert::TryFrom;

    use http::Request as HttpRequest;
    use serde::Serialize;
    #[cfg(feature = "json")]
    use serde_json;
    use serde_urlencoded;

    use crate::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, HOST};
    use crate::Method;

    use super::Request;
    use super::super::{body, Client};

    #[test]
    fn basic_get_request() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.get(some_url).build().unwrap();

        assert_eq!(r.method(), &Method::GET);
        assert_eq!(r.url().as_str(), some_url);
    }

    #[test]
    fn basic_head_request() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.head(some_url).build().unwrap();

        assert_eq!(r.method(), &Method::HEAD);
        assert_eq!(r.url().as_str(), some_url);
    }

    #[test]
    fn basic_post_request() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url).build().unwrap();

        assert_eq!(r.method(), &Method::POST);
        assert_eq!(r.url().as_str(), some_url);
    }

    #[test]
    fn basic_put_request() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.put(some_url).build().unwrap();

        assert_eq!(r.method(), &Method::PUT);
        assert_eq!(r.url().as_str(), some_url);
    }

    #[test]
    fn basic_patch_request() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.patch(some_url).build().unwrap();

        assert_eq!(r.method(), &Method::PATCH);
        assert_eq!(r.url().as_str(), some_url);
    }

    #[test]
    fn basic_delete_request() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.delete(some_url).build().unwrap();

        assert_eq!(r.method(), &Method::DELETE);
        assert_eq!(r.url().as_str(), some_url);
    }

    #[test]
    fn add_header() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);

        let header = HeaderValue::from_static("google.com");

        // Add a copy of the header to the request builder
        let r = r.header(HOST, header.clone()).build().unwrap();

        // then check it was actually added
        assert_eq!(r.headers().get(HOST), Some(&header));
    }

    #[test]
    fn add_headers() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);

        let header = HeaderValue::from_static("google.com");

        let mut headers = HeaderMap::new();
        headers.insert(HOST, header);

        // Add a copy of the headers to the request builder
        let r = r.headers(headers.clone()).build().unwrap();

        // then make sure they were added correctly
        assert_eq!(r.headers(), &headers);
    }

    #[test]
    fn add_headers_multi() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);

        let header_json = HeaderValue::from_static("application/json");
        let header_xml = HeaderValue::from_static("application/xml");

        let mut headers = HeaderMap::new();
        headers.append(ACCEPT, header_json);
        headers.append(ACCEPT, header_xml);

        // Add a copy of the headers to the request builder
        let r = r.headers(headers.clone()).build().unwrap();

        // then make sure they were added correctly
        assert_eq!(r.headers(), &headers);
        let mut all_values = r.headers().get_all(ACCEPT).iter();
        assert_eq!(all_values.next().unwrap(), &"application/json");
        assert_eq!(all_values.next().unwrap(), &"application/xml");
        assert_eq!(all_values.next(), None);
    }

    #[test]
    fn add_body() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);

        let body = "Some interesting content";

        let mut r = r.body(body).build().unwrap();

        let buf = body::read_to_string(r.body_mut().take().unwrap()).unwrap();

        assert_eq!(buf, body);
    }

    #[test]
    fn add_query_append() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let mut r = client.get(some_url);

        r = r.query(&[("foo", "bar")]);
        r = r.query(&[("qux", 3)]);

        let req = r.build().expect("request is valid");
        assert_eq!(req.url().query(), Some("foo=bar&qux=3"));
    }

    #[test]
    fn add_query_append_same() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let mut r = client.get(some_url);

        r = r.query(&[("foo", "a"), ("foo", "b")]);

        let req = r.build().expect("request is valid");
        assert_eq!(req.url().query(), Some("foo=a&foo=b"));
    }

    #[test]
    fn add_query_struct() {
        #[derive(Serialize)]
        struct Params {
            foo: String,
            qux: i32,
        }

        let client = Client::new();
        let some_url = "https://google.com/";
        let mut r = client.get(some_url);

        let params = Params {
            foo: "bar".into(),
            qux: 3,
        };

        r = r.query(&params);

        let req = r.build().expect("request is valid");
        assert_eq!(req.url().query(), Some("foo=bar&qux=3"));
    }

    #[test]
    fn add_query_map() {
        let mut params = BTreeMap::new();
        params.insert("foo", "bar");
        params.insert("qux", "three");

        let client = Client::new();
        let some_url = "https://google.com/";
        let mut r = client.get(some_url);

        r = r.query(&params);

        let req = r.build().expect("request is valid");
        assert_eq!(req.url().query(), Some("foo=bar&qux=three"));
    }

    #[test]
    fn add_form() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);

        let mut form_data = HashMap::new();
        form_data.insert("foo", "bar");

        let mut r = r.form(&form_data).build().unwrap();

        // Make sure the content type was set
        assert_eq!(
            r.headers().get(CONTENT_TYPE).unwrap(),
            &"application/x-www-form-urlencoded"
        );

        let buf = body::read_to_string(r.body_mut().take().unwrap()).unwrap();

        let body_should_be = serde_urlencoded::to_string(&form_data).unwrap();
        assert_eq!(buf, body_should_be);
    }

    #[test]
    #[cfg(feature = "json")]
    fn add_json() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);

        let mut json_data = HashMap::new();
        json_data.insert("foo", "bar");

        let mut r = r.json(&json_data).build().unwrap();

        // Make sure the content type was set
        assert_eq!(r.headers().get(CONTENT_TYPE).unwrap(), &"application/json");

        let buf = body::read_to_string(r.body_mut().take().unwrap()).unwrap();

        let body_should_be = serde_json::to_string(&json_data).unwrap();
        assert_eq!(buf, body_should_be);
    }

    #[test]
    #[cfg(feature = "json")]
    fn add_json_fail() {
        use serde::ser::Error as _;
        use serde::{Serialize, Serializer};
        use std::error::Error as _;
        struct MyStruct;
        impl Serialize for MyStruct {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
            {
                Err(S::Error::custom("nope"))
            }
        }

        let client = Client::new();
        let some_url = "https://google.com/";
        let r = client.post(some_url);
        let json_data = MyStruct;
        let err = r.json(&json_data).build().unwrap_err();
        assert!(err.is_builder()); // well, duh ;)
        assert!(err.source().unwrap().is::<serde_json::Error>());
    }

    #[test]
    fn test_replace_headers() {
        use http::HeaderMap;

        let mut headers = HeaderMap::new();
        headers.insert("foo", "bar".parse().unwrap());
        headers.append("foo", "baz".parse().unwrap());

        let client = Client::new();
        let req = client
            .get("https://hyper.rs")
            .header("im-a", "keeper")
            .header("foo", "pop me")
            .headers(headers)
            .build()
            .expect("request build");

        assert_eq!(req.headers()["im-a"], "keeper");

        let foo = req.headers().get_all("foo").iter().collect::<Vec<_>>();
        assert_eq!(foo.len(), 2);
        assert_eq!(foo[0], "bar");
        assert_eq!(foo[1], "baz");
    }

    #[test]
    fn normalize_empty_query() {
        let client = Client::new();
        let some_url = "https://google.com/";
        let empty_query: &[(&str, &str)] = &[];

        let req = client
            .get(some_url)
            .query(empty_query)
            .build()
            .expect("request build");

        assert_eq!(req.url().query(), None);
        assert_eq!(req.url().as_str(), "https://google.com/");
    }

    #[test]
    fn convert_url_authority_into_basic_auth() {
        let client = Client::new();
        let some_url = "https://Aladdin:open sesame@localhost/";

        let req = client
            .get(some_url)
            .build()
            .expect("request build");

        assert_eq!(req.url().as_str(), "https://localhost/");
        assert_eq!(req.headers()["authorization"], "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    }

    #[test]
    fn convert_from_http_request() {
        let http_request = HttpRequest::builder().method("GET")
            .uri("http://localhost/")
            .header("User-Agent", "my-awesome-agent/1.0")
            .body("test test test")
            .unwrap();
        let req: Request = Request::try_from(http_request).unwrap();
        assert_eq!(req.body().is_none(), false);
        let test_data = b"test test test";
        assert_eq!(req.body().unwrap().as_bytes(), Some(&test_data[..]));
        let headers = req.headers();
        assert_eq!(headers.get("User-Agent").unwrap(), "my-awesome-agent/1.0");
        assert_eq!(req.method(), Method::GET);
        assert_eq!(req.url().as_str(), "http://localhost/");
    }

    #[test]
    fn test_basic_auth_sensitive_header() {
        let client = Client::new();
        let some_url = "https://localhost/";

        let req = client
            .get(some_url)
            .basic_auth("Aladdin", Some("open sesame"))
            .build()
            .expect("request build");

        assert_eq!(req.url().as_str(), "https://localhost/");
        assert_eq!(req.headers()["authorization"], "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert_eq!(req.headers()["authorization"].is_sensitive(), true);
    }

    #[test]
    fn test_bearer_auth_sensitive_header() {
        let client = Client::new();
        let some_url = "https://localhost/";

        let req = client
            .get(some_url)
            .bearer_auth("Hold my bear")
            .build()
            .expect("request build");

        assert_eq!(req.url().as_str(), "https://localhost/");
        assert_eq!(req.headers()["authorization"], "Bearer Hold my bear");
        assert_eq!(req.headers()["authorization"].is_sensitive(), true);
    }
}
