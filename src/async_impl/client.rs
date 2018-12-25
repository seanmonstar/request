use std::{fmt, str};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::{Async, Future, Poll};
use hyper::client::ResponseFuture;
use crate::header::{HeaderMap, HeaderValue, LOCATION, USER_AGENT, REFERER, ACCEPT,
             ACCEPT_ENCODING, RANGE, TRANSFER_ENCODING, CONTENT_TYPE, CONTENT_LENGTH, CONTENT_ENCODING};
use mime::{self};
use log::debug;
#[cfg(feature = "default-tls")]
use native_tls::TlsConnector;


use super::request::{Request, RequestBuilder};
use super::response::Response;
use crate::connect::Connector;
use crate::into_url::to_uri;
use crate::redirect::{self, RedirectPolicy, remove_sensitive_headers};
use crate::{IntoUrl, Method, Proxy, StatusCode, Url};
#[cfg(feature = "tls")]
use crate::{Certificate, Identity};
#[cfg(feature = "tls")]
use crate::tls::{ TLSBackend, inner };

static DEFAULT_USER_AGENT: &'static str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// An asynchronous `Client` to make Requests with.
///
/// The Client has various configuration values to tweak, but the defaults
/// are set to what is usually the most commonly desired value.
///
/// The `Client` holds a connection pool internally, so it is advised that
/// you create one and **reuse** it.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientRef>,
}

/// A `ClientBuilder` can be used to create a `Client` with  custom configuration:
pub struct ClientBuilder {
    config: Config,
}

struct Config {
    gzip: bool,
    headers: HeaderMap,
    #[cfg(feature = "default-tls")]
    hostname_verification: bool,
    #[cfg(feature = "tls")]
    certs_verification: bool,
    proxies: Vec<Proxy>,
    redirect_policy: RedirectPolicy,
    referer: bool,
    timeout: Option<Duration>,
    #[cfg(feature = "tls")]
    root_certs: Vec<Certificate>,
    #[cfg(feature = "tls")]
    identity: Option<Identity>,
    #[cfg(feature = "tls")]
    tls: TLSBackend,
}

impl ClientBuilder {
    /// Constructs a new `ClientBuilder`
    pub fn new() -> ClientBuilder {
        let mut headers: HeaderMap<HeaderValue> = HeaderMap::with_capacity(2);
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
        headers.insert(ACCEPT, HeaderValue::from_str(mime::STAR_STAR.as_ref()).expect("unable to parse mime"));

        ClientBuilder {
            config: Config {
                gzip: true,
                headers: headers,
                #[cfg(feature = "default-tls")]
                hostname_verification: true,
                #[cfg(feature = "tls")]
                certs_verification: true,
                proxies: Vec::new(),
                redirect_policy: RedirectPolicy::default(),
                referer: true,
                timeout: None,
                #[cfg(feature = "tls")]
                root_certs: Vec::new(),
                #[cfg(feature = "tls")]
                identity: None,
                #[cfg(feature = "tls")]
                tls: TLSBackend::default(),
            },
        }
    }

    /// Returns a `Client` that uses this `ClientBuilder` configuration.
    ///
    /// # Errors
    ///
    /// This method fails if TLS backend cannot be initialized.
    pub fn build(self) -> crate::Result<Client> {
        let config = self.config;
        let proxies = Arc::new(config.proxies);

        let connector = {
            #[cfg(feature = "tls")]
            match config.tls {
                #[cfg(feature = "default-tls")]
                TLSBackend::Default => {
                    let mut tls = TlsConnector::builder();
                    tls.danger_accept_invalid_hostnames(!config.hostname_verification);
                    tls.danger_accept_invalid_certs(!config.certs_verification);

                    for cert in config.root_certs {
                        let cert = match cert.inner {
                            inner::Certificate::Der(buf) =>
                                try_!(::native_tls::Certificate::from_der(&buf)),
                            inner::Certificate::Pem(buf) =>
                                try_!(::native_tls::Certificate::from_pem(&buf))
                        };
                        tls.add_root_certificate(cert);
                    }

                    if let Some(id) = config.identity {
                        let id = match id.inner {
                            inner::Identity::Pkcs12(buf, passwd) =>
                                try_!(::native_tls::Identity::from_pkcs12(&buf, &passwd)),
                            #[cfg(feature = "rustls-tls")]
                            _ => return Err(crate::error::from(crate::error::Kind::Incompatible))
                        };
                        tls.identity(id);
                    }

                    Connector::new_default_tls(tls, proxies.clone())?
                },
                #[cfg(feature = "rustls-tls")]
                TLSBackend::Rustls => {
                    use std::io::Cursor;
                    use rustls::TLSError;
                    use rustls::internal::pemfile;
                    use crate::tls::NoVerifier;

                    let mut tls = ::rustls::ClientConfig::new();
                    tls.root_store.add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);

                    if !config.certs_verification {
                        tls.dangerous().set_certificate_verifier(Arc::new(NoVerifier));
                    }

                    for cert in config.root_certs {
                        match cert.inner {
                            inner::Certificate::Der(buf) => try_!(tls.root_store.add(&::rustls::Certificate(buf))
                                .map_err(TLSError::WebPKIError)),
                            inner::Certificate::Pem(buf) => {
                                let mut pem = Cursor::new(buf);
                                let certs = try_!(pemfile::certs(&mut pem)
                                    .map_err(|_| TLSError::General(String::from("No valid certificate was found"))));
                                for c in certs {
                                    try_!(tls.root_store.add(&c)
                                        .map_err(TLSError::WebPKIError));
                                }
                            }
                        }
                    }

                    if let Some(id) = config.identity {
                        let (key, certs) = match id.inner {
                            inner::Identity::Pem(buf) => {
                                let mut pem = Cursor::new(buf);
                                let certs = try_!(pemfile::certs(&mut pem)
                                    .map_err(|_| TLSError::General(String::from("No valid certificate was found"))));
                                pem.set_position(0);
                                let mut sk = try_!(pemfile::pkcs8_private_keys(&mut pem)
                                    .or_else(|_| {
                                        pem.set_position(0);
                                        pemfile::rsa_private_keys(&mut pem)
                                    })
                                    .map_err(|_| TLSError::General(String::from("No valid private key was found"))));
                                if let (Some(sk), false) = (sk.pop(), certs.is_empty()) {
                                    (sk, certs)
                                } else {
                                    return Err(crate::error::from(TLSError::General(String::from("private key or certificate not found"))));
                                }
                            },
                            #[cfg(feature = "default-tls")]
                            _ => return Err(crate::error::from(crate::error::Kind::Incompatible))
                        };
                        tls.set_single_client_cert(certs, key);
                    }

                    Connector::new_rustls_tls(tls, proxies.clone())?
                }
            }

            #[cfg(not(feature = "tls"))]
            Connector::new(proxies.clone())
        };

        let hyper_client = ::hyper::Client::builder()
            .build(connector);

        Ok(Client {
            inner: Arc::new(ClientRef {
                gzip: config.gzip,
                hyper: hyper_client,
                headers: config.headers,
                redirect_policy: config.redirect_policy,
                referer: config.referer,
            }),
        })
    }

    /// Use native TLS backend.
    #[cfg(feature = "default-tls")]
    pub fn use_default_tls(mut self) -> ClientBuilder {
        self.config.tls = TLSBackend::Default;
        self
    }

    /// Use rustls TLS backend.
    #[cfg(feature = "rustls-tls")]
    pub fn use_rustls_tls(mut self) -> ClientBuilder {
        self.config.tls = TLSBackend::Rustls;
        self
    }

    /// Add a custom root certificate.
    ///
    /// This can be used to connect to a server that has a self-signed
    /// certificate for example.
    #[cfg(feature = "tls")]
    pub fn add_root_certificate(mut self, cert: Certificate) -> ClientBuilder {
        self.config.root_certs.push(cert);
        self
    }

    /// Sets the identity to be used for client certificate authentication.
    #[cfg(feature = "tls")]
    pub fn identity(mut self, identity: Identity) -> ClientBuilder {
        self.config.identity = Some(identity);
        self
    }

    /// Controls the use of hostname verification.
    ///
    /// Defaults to `false`.
    ///
    /// # Warning
    ///
    /// You should think very carefully before you use this method. If
    /// hostname verification is not used, any valid certificate for any
    /// site will be trusted for use from any other. This introduces a
    /// significant vulnerability to man-in-the-middle attacks.
    #[cfg(feature = "default-tls")]
    pub fn danger_accept_invalid_hostnames(mut self, accept_invalid_hostname: bool) -> ClientBuilder {
        self.config.hostname_verification = !accept_invalid_hostname;
        self
    }

    /// Controls the use of certificate validation.
    ///
    /// Defaults to `false`.
    ///
    /// # Warning
    ///
    /// You should think very carefully before using this method. If
    /// invalid certificates are trusted, *any* certificate for *any* site
    /// will be trusted for use. This includes expired certificates. This
    /// introduces significant vulnerabilities, and should only be used
    /// as a last resort.
    #[cfg(feature = "tls")]
    pub fn danger_accept_invalid_certs(mut self, accept_invalid_certs: bool) -> ClientBuilder {
        self.config.certs_verification = !accept_invalid_certs;
        self
    }


    /// Sets the default headers for every request.
    pub fn default_headers(mut self, headers: HeaderMap) -> ClientBuilder {
        for (key, value) in headers.iter() {
            self.config.headers.insert(key, value.clone());
        }
        self
    }

    /// Enable auto gzip decompression by checking the ContentEncoding response header.
    ///
    /// If auto gzip decompresson is turned on:
    /// - When sending a request and if the request's headers do not already contain
    ///   an `Accept-Encoding` **and** `Range` values, the `Accept-Encoding` header is set to `gzip`.
    ///   The body is **not** automatically inflated.
    /// - When receiving a response, if it's headers contain a `Content-Encoding` value that
    ///   equals to `gzip`, both values `Content-Encoding` and `Content-Length` are removed from the
    ///   headers' set. The body is automatically deinflated.
    ///
    /// Default is enabled.
    pub fn gzip(mut self, enable: bool) -> ClientBuilder {
        self.config.gzip = enable;
        self
    }

    /// Add a `Proxy` to the list of proxies the `Client` will use.
    pub fn proxy(mut self, proxy: Proxy) -> ClientBuilder {
        self.config.proxies.push(proxy);
        self
    }

    /// Set a `RedirectPolicy` for this client.
    ///
    /// Default will follow redirects up to a maximum of 10.
    pub fn redirect(mut self, policy: RedirectPolicy) -> ClientBuilder {
        self.config.redirect_policy = policy;
        self
    }

    /// Enable or disable automatic setting of the `Referer` header.
    ///
    /// Default is `true`.
    pub fn referer(mut self, enable: bool) -> ClientBuilder {
        self.config.referer = enable;
        self
    }

    /// Set a timeout for both the read and write operations of a client.
    pub fn timeout(mut self, timeout: Duration) -> ClientBuilder {
        self.config.timeout = Some(timeout);
        self
    }

    #[doc(hidden)]
    #[deprecated(note = "DNS no longer uses blocking threads")]
    pub fn dns_threads(self, _threads: usize) -> ClientBuilder {
        self
    }
}

type HyperClient = ::hyper::Client<Connector>;

impl Client {
    /// Constructs a new `Client`.
    ///
    /// # Panics
    ///
    /// This method panics if TLS backend cannot be created or
    /// initialized. Use `Client::builder()` if you wish to handle the failure
    /// as an `Error` instead of panicking.
    pub fn new() -> Client {
        ClientBuilder::new()
            .build()
            .expect("TLS failed to initialize")
    }

    /// Creates a `ClientBuilder` to configure a `Client`.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Convenience method to make a `GET` request to a URL.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::GET, url)
    }

    /// Convenience method to make a `POST` request to a URL.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::POST, url)
    }

    /// Convenience method to make a `PUT` request to a URL.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn put<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PUT, url)
    }

    /// Convenience method to make a `PATCH` request to a URL.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PATCH, url)
    }

    /// Convenience method to make a `DELETE` request to a URL.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn delete<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::DELETE, url)
    }

    /// Convenience method to make a `HEAD` request to a URL.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn head<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::HEAD, url)
    }

    /// Start building a `Request` with the `Method` and `Url`.
    ///
    /// Returns a `RequestBuilder`, which will allow setting headers and
    /// request body before sending.
    ///
    /// # Errors
    ///
    /// This method fails whenever supplied `Url` cannot be parsed.
    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder {
        let req = url
            .into_url()
            .map(move |url| Request::new(method, url));
        RequestBuilder::new(self.clone(), req)
    }

    /// Executes a `Request`.
    ///
    /// A `Request` can be built manually with `Request::new()` or obtained
    /// from a RequestBuilder with `RequestBuilder::build()`.
    ///
    /// You should prefer to use the `RequestBuilder` and
    /// `RequestBuilder::send()`.
    ///
    /// # Errors
    ///
    /// This method fails if there was an error while sending request,
    /// redirect loop was detected or redirect limit was exhausted.
    pub fn execute(&self, request: Request) -> Pending {
        self.execute_request(request)
    }


    fn execute_request(&self, req: Request) -> Pending {
        let (
            method,
            url,
            user_headers,
            body
        ) = req.pieces();

        let mut headers = self.inner.headers.clone(); // default headers
        for (key, value) in user_headers.iter() {
            headers.insert(key, value.clone());
        }

        if self.inner.gzip &&
            !headers.contains_key(ACCEPT_ENCODING) &&
            !headers.contains_key(RANGE) {
            headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        }

        let uri = to_uri(&url);

        let (reusable, body) = match body {
            Some(body) => {
                let (reusable, body) = body.into_hyper();
                (Some(reusable), body)
            },
            None => {
                (None, ::hyper::Body::empty())
            }
        };

        let mut req = ::hyper::Request::builder()
            .method(method.clone())
            .uri(uri.clone())
            .body(body)
            .expect("valid request parts");

        *req.headers_mut() = headers.clone();

        let in_flight = self.inner.hyper.request(req);

        Pending {
            inner: PendingInner::Request(PendingRequest {
                method: method,
                url: url,
                headers: headers,
                body: reusable,

                urls: Vec::new(),

                client: self.inner.clone(),

                in_flight: in_flight,
            }),
        }
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Client")
            .field("gzip", &self.inner.gzip)
            .field("redirect_policy", &self.inner.redirect_policy)
            .field("referer", &self.inner.referer)
            .finish()
    }
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .finish()
    }
}

struct ClientRef {
    gzip: bool,
    headers: HeaderMap,
    hyper: HyperClient,
    redirect_policy: RedirectPolicy,
    referer: bool,
}

pub struct Pending {
    inner: PendingInner,
}

enum PendingInner {
    Request(PendingRequest),
    Error(Option<crate::Error>),
}

struct PendingRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Option<Option<Bytes>>,

    urls: Vec<Url>,

    client: Arc<ClientRef>,

    in_flight: ResponseFuture,
}

impl Pending {
    pub(super) fn new_err(err: crate::Error) -> Pending {
        Pending {
            inner: PendingInner::Error(Some(err)),
        }
    }
}

impl Future for Pending {
    type Item = Response;
    type Error = crate::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner {
            PendingInner::Request(ref mut req) => req.poll(),
            PendingInner::Error(ref mut err) => Err(err.take().expect("Pending error polled more than once")),
        }
    }
}

impl Future for PendingRequest {
    type Item = Response;
    type Error = crate::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let res = match try_!(self.in_flight.poll(), &self.url) {
                Async::Ready(res) => res,
                Async::NotReady => return Ok(Async::NotReady),
            };
            let should_redirect = match res.status() {
                StatusCode::MOVED_PERMANENTLY |
                StatusCode::FOUND |
                StatusCode::SEE_OTHER => {
                    self.body = None;
                    for header in &[TRANSFER_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, CONTENT_LENGTH] {
                        self.headers.remove(header);
                    }

                    match self.method {
                        Method::GET | Method::HEAD => {},
                        _ => {
                            self.method = Method::GET;
                        }
                    }
                    true
                },
                StatusCode::TEMPORARY_REDIRECT |
                StatusCode::PERMANENT_REDIRECT => match self.body {
                    Some(Some(_)) | None => true,
                    Some(None) => false,
                },
                _ => false,
            };
            if should_redirect {
                let loc = res.headers()
                    .get(LOCATION)
                    .and_then(|val| {
                        let loc = (|| -> Option<Url> {
                            // Some sites may send a utf-8 Location header,
                            // even though we're supposed to treat those bytes
                            // as opaque, we'll check specifically for utf8.
                            self.url.join(str::from_utf8(val.as_bytes()).ok()?).ok()
                        })();

                        if loc.is_none() {
                            debug!("Location header had invalid URI: {:?}", val);
                        }
                        loc
                    });
                if let Some(loc) = loc {
                    if self.client.referer {
                        if let Some(referer) = make_referer(&loc, &self.url) {
                            self.headers.insert(REFERER, referer);
                        }
                    }
                    self.urls.push(self.url.clone());
                    let action = self.client.redirect_policy.check(
                        res.status(),
                        &loc,
                        &self.urls,
                    );

                    match action {
                        redirect::Action::Follow => {
                            self.url = loc;

                            remove_sensitive_headers(&mut self.headers, &self.url, &self.urls);
                            debug!("redirecting to {:?} '{}'", self.method, self.url);
                            let uri = to_uri(&self.url);
                            let body = match self.body {
                                Some(Some(ref body)) => ::hyper::Body::from(body.clone()),
                                _ => ::hyper::Body::empty(),
                            };
                            let mut req = ::hyper::Request::builder()
                                .method(self.method.clone())
                                .uri(uri.clone())
                                .body(body)
                                .expect("valid request parts");

                            *req.headers_mut() = self.headers.clone();
                            self.in_flight = self.client.hyper.request(req);
                            continue;
                        },
                        redirect::Action::Stop => {
                            debug!("redirect_policy disallowed redirection to '{}'", loc);
                        },
                        redirect::Action::LoopDetected => {
                            return Err(crate::error::loop_detected(self.url.clone()));
                        },
                        redirect::Action::TooManyRedirects => {
                            return Err(crate::error::too_many_redirects(self.url.clone()));
                        }
                    }
                }
            }
            let res = Response::new(res, self.url.clone(), self.client.gzip);
            return Ok(Async::Ready(res));
        }
    }
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.inner {
            PendingInner::Request(ref req) => {
                f.debug_struct("Pending")
                    .field("method", &req.method)
                    .field("url", &req.url)
                    .finish()
            },
            PendingInner::Error(ref err) => {
                f.debug_struct("Pending")
                    .field("error", err)
                    .finish()
            }
        }
    }
}

fn make_referer(next: &Url, previous: &Url) -> Option<HeaderValue> {
    if next.scheme() == "http" && previous.scheme() == "https" {
        return None;
    }

    let mut referer = previous.clone();
    let _ = referer.set_username("");
    let _ = referer.set_password(None);
    referer.set_fragment(None);
    referer.as_str().parse().ok()
}
