use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};

use hyper::client::connect::dns as hyper_dns;
use hyper::service::Service;
use tokio::sync::Mutex;
use trust_dns_resolver::config::NameServerConfigGroup;
use trust_dns_resolver::{
    config::{ResolverConfig, ResolverOpts},
    lookup_ip::LookupIpIntoIter,
    system_conf, AsyncResolver, TokioConnection, TokioConnectionProvider,
};

use crate::error::BoxError;

type SharedResolver = Arc<AsyncResolver<TokioConnection, TokioConnectionProvider>>;

lazy_static! {
    static ref SYSTEM_CONF: io::Result<(ResolverConfig, ResolverOpts)> =
        system_conf::read_system_conf().map_err(io::Error::from);
}

#[derive(Clone)]
pub(crate) struct TrustDnsResolver {
    state: Arc<Mutex<State>>,
    dns_provider: Option<DnsProvider>,
}

enum State {
    Init,
    Ready(SharedResolver),
}

/// Use to override your system's DNS provider
#[derive(Clone, Debug)]
pub enum DnsProvider {
    /// Use google's DNS
    Google,
    /// Use cloudflare's DNS
    Cloudflare,
    /// Use quad9's DNS
    Quad9,
    /// Custom DNS IPs
    Custom(Vec<IpAddr>),
}

impl TrustDnsResolver {
    pub(crate) fn new(dns_provider: Option<DnsProvider>) -> io::Result<Self> {
        SYSTEM_CONF.as_ref().map_err(|e| {
            io::Error::new(e.kind(), format!("error reading DNS system conf: {}", e))
        })?;

        // At this stage, we might not have been called in the context of a
        // Tokio Runtime, so we must delay the actual construction of the
        // resolver.
        Ok(TrustDnsResolver {
            state: Arc::new(Mutex::new(State::Init)),
            dns_provider,
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
        let dns_provider = self.dns_provider.clone();
        Box::pin(async move {
            let mut lock = resolver.state.lock().await;

            let resolver = match &*lock {
                State::Init => {
                    let resolver =
                        new_resolver(tokio::runtime::Handle::current(), dns_provider).await?;
                    *lock = State::Ready(resolver.clone());
                    resolver
                }
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
    dns_provider: Option<DnsProvider>,
) -> Result<SharedResolver, BoxError> {
    let (mut config, opts) = SYSTEM_CONF
        .as_ref()
        .expect("can't construct TrustDnsResolver if SYSTEM_CONF is error")
        .clone();
    if let Some(dns_servers) = dns_provider {
        config = match dns_servers {
            DnsProvider::Cloudflare => ResolverConfig::cloudflare(),
            DnsProvider::Google => ResolverConfig::google(),
            DnsProvider::Quad9 => ResolverConfig::quad9(),
            DnsProvider::Custom(addrs) => ResolverConfig::from_parts(
                config.domain().cloned(),
                config.search().to_vec(),
                NameServerConfigGroup::from_ips_clear(&addrs, 54),
            ),
        }
    }
    let resolver = AsyncResolver::new(config, opts, handle).await?;
    Ok(Arc::new(resolver))
}
