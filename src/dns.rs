use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::io;
use std::net::IpAddr;

use hyper::client::connect::dns as hyper_dns;
use hyper::service::Service;
use tokio::sync::Mutex;
use trust_dns_resolver::{
    config::{ResolverConfig, ResolverOpts},
    lookup_ip::LookupIpIntoIter,
    system_conf, AsyncResolver, TokioConnection, TokioConnectionProvider,
};

use crate::error::BoxError;

type SharedResolver = Arc<AsyncResolver<TokioConnection, TokioConnectionProvider>>;

lazy_static! {
    static ref SYSTEM_CONF: io::Result<(ResolverConfig, ResolverOpts)> = system_conf::read_system_conf().map_err(io::Error::from);
}

#[derive(Clone)]
pub(crate) struct TrustDnsResolver {
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
pub(crate) struct TrustDnsConfig {
    pub(crate) ips: Vec<IpAddr>,
    pub(crate) port: u16,
    pub(crate) transport: DnsTransport,
    pub(crate) domain: Option<String>,
    pub(crate) search_domains: Vec<String>,
}

/// Transport protocol for DNS
#[derive(Debug)]
pub(crate) enum DnsTransport {
    /// This will create UDP and TCP connections, using the same port.
    /// Queries and responses are sent unencrypted.
    UdpAndTcp,
    #[cfg(feature = "trust-dns-over-tls")]
    /// DNS over TLS.
    ///
    /// This requires `trust-dns-over-openssl` or `trust-dns-over-native-tls`
    /// or `trust-dns-over-rustls` feature.
    Tls {
        /// Simple public key infrastructure (SPKI) name.
        tls_dns_name: String,
    },
    #[cfg(feature = "trust-dns-over-https")]
    /// DNS over HTTPS.
    ///
    /// This requires `trust-dns-over-https-rustls` feature.
    Https {
        /// Simple public key infrastructure (SPKI) name.
        tls_dns_name: String,
    },
}

#[derive(Debug)]
pub (crate) struct TrustDnsError {
    pub(crate) error: BoxError
}

enum State {
    Init(Option<ResolverConfig>),
    Ready(SharedResolver),
}

impl TrustDnsConfig {
    pub(crate) fn new() -> Self {
        TrustDnsConfig {
            ips: vec![],
            port: 53,
            transport: DnsTransport::UdpAndTcp,
            domain: None,
            search_domains: vec![],
        }
    }
}


impl TrustDnsResolver {
    pub(crate) fn new(config: Option<ResolverConfig>) -> io::Result<Self> {
        SYSTEM_CONF.as_ref().map_err(|e| {
            io::Error::new(e.kind(), format!("error reading DNS system conf: {}", e))
        })?;

        // At this stage, we might not have been called in the context of a
        // Tokio Runtime, so we must delay the actual construction of the
        // resolver.
        Ok(TrustDnsResolver {
            state: Arc::new(Mutex::new(State::Init(config))),
        })
    }
}

impl Service<hyper_dns::Name> for TrustDnsResolver {
    type Response = LookupIpIntoIter;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: hyper_dns::Name) -> Self::Future {
        let resolver = self.clone();
        Box::pin(async move {
            let mut lock = resolver.state.lock().await;

            let resolver = match &*lock {
                State::Init(config) => {
                    let resolver =
                        new_resolver(tokio::runtime::Handle::current(), config.clone()).await?;
                    *lock = State::Ready(resolver.clone());
                    resolver
                },
                State::Ready(resolver) => resolver.clone(),
            };

            // Don't keep lock once the resolver is constructed, otherwise
            // only one lookup could be done at a time.
            drop(lock);

            let lookup = resolver.lookup_ip(name.as_str()).await?;
            Ok(lookup.into_iter())
        })
    }
}

/// Takes a `Handle` argument as an indicator that it must be called from
/// within the context of a Tokio runtime.
async fn new_resolver(
    handle: tokio::runtime::Handle,
    config: Option<ResolverConfig>,
) -> Result<SharedResolver, BoxError> {
    let (config_system, opts) = SYSTEM_CONF
        .as_ref()
        .expect("can't construct TrustDnsResolver if SYSTEM_CONF is error")
        .clone();
    let resolver = AsyncResolver::new(config.unwrap_or(config_system), opts, handle).await?;
    Ok(Arc::new(resolver))
}
