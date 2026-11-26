use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use http::{Extensions, HeaderMap};

/// The visitor's address, as worked out by the layer. Read it in handlers from request
/// extensions instead of parsing headers yourself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientIp(pub IpAddr);

type PeerFn = Arc<dyn Fn(&Extensions) -> Option<IpAddr> + Send + Sync>;

/// Works out the client address for a request.
#[derive(Clone)]
pub struct ClientIpResolver {
    trust_forwarded: bool,
    peer: PeerFn,
}

impl Default for ClientIpResolver {
    fn default() -> Self {
        ClientIpResolver {
            trust_forwarded: false,
            peer: Arc::new(|ext: &Extensions| ext.get::<SocketAddr>().map(SocketAddr::ip)),
        }
    }
}

impl fmt::Debug for ClientIpResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientIpResolver").field("trust_forwarded", &self.trust_forwarded).finish_non_exhaustive()
    }
}

impl ClientIpResolver {
    /// Use the last `X-Forwarded-For` entry: the one your proxy appended, which a client
    /// cannot forge. Earlier entries are whatever the client sent and are ignored.
    #[must_use]
    pub fn trust_forwarded(mut self, on: bool) -> Self {
        self.trust_forwarded = on;
        self
    }

    /// Where the connection's own address lives in request extensions.
    #[must_use]
    pub fn peer(mut self, f: impl Fn(&Extensions) -> Option<IpAddr> + Send + Sync + 'static) -> Self {
        self.peer = Arc::new(f);
        self
    }

    /// The client address, if one can be found.
    pub fn resolve(&self, headers: &HeaderMap, ext: &Extensions) -> Option<IpAddr> {
        if self.trust_forwarded {
            let last = headers
                .get_all("x-forwarded-for")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .last();
            if let Some(ip) = last.and_then(|s| s.trim().parse::<IpAddr>().ok()) {
                return Some(ip);
            }
        }
        (self.peer)(ext)
    }
}
