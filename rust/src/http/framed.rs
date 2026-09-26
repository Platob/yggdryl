//! Which version one request speaks, for a client built with HTTP/2 or
//! HTTP/3 beside HTTP/1.1.
//!
//! The choice is the options' [`HttpOptions::http_version`], read against
//! what the client already learned of the origin:
//!
//! | asked | `https` origin | plain `http` origin |
//! | --- | --- | --- |
//! | nothing (negotiate) | HTTP/3 where `Alt-Svc` advertised it, else ALPN `h2`, else HTTP/1.1 | HTTP/1.1 |
//! | `Http2` | ALPN `h2`, else HTTP/1.1 | `h2c` by prior knowledge, else HTTP/1.1 |
//! | `Http3` | HTTP/3, else as `Http2` | as `Http2` |
//! | `Http11`, `Http10` | HTTP/1.1 | HTTP/1.1 |
//!
//! "Else" is a fallback in the same attempt, remembered for the origin: an
//! ALPN answer of `http/1.1` or a plain origin that does not read the HTTP/2
//! preface is sent HTTP/1.1 from then on, and an origin QUIC cannot reach
//! is sent the other way for five minutes. A request through a proxy never
//! comes here: it speaks HTTP/1.1.

use std::sync::{Arc, OnceLock};

use super::client::{Answer, Payload, Wire};
use super::h2::{self, Origin};
use super::tls::ClientConfigs;
use super::{Headers, HttpOptions, HttpVersion};
use crate::{Result, Url};

/// The HTTP/2 and HTTP/3 transports of one client.
#[derive(Debug)]
pub(crate) struct Framed {
    asked: Option<HttpVersion>,
    h2: h2::Pool,
    #[cfg(feature = "http3")]
    h3: super::h3::Pool,
}

impl Framed {
    /// The transports `options` call for; `None` when they ask for
    /// HTTP/1.1.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] when they ask for HTTP/3 of a build without
    /// the `http3` feature; as [`ClientConfigs::for_options`] otherwise.
    pub(crate) fn for_options(options: &HttpOptions) -> Result<Option<Arc<Self>>> {
        let asked = options.http_version();
        match asked {
            Some(HttpVersion::Http10 | HttpVersion::Http11) => return Ok(None),
            #[cfg(not(feature = "http3"))]
            Some(HttpVersion::Http3) => {
                return Err(crate::Error::unsupported(
                    "HTTP/3, which this build was made without (the `http3` feature)",
                    "http_version",
                ));
            }
            _ => {}
        }
        let configs = ClientConfigs::for_options(options)?;
        Ok(Some(Arc::new(Self {
            asked,
            h2: h2::Pool::new(&configs, options.connect_timeout()),
            #[cfg(feature = "http3")]
            h3: super::h3::Pool::new(&configs, options.connect_timeout())?,
        })))
    }

    /// The transports of every default-configured client, shared as their
    /// HTTP/1.1 pool is; `None` when they could not be set up, which leaves
    /// those clients speaking HTTP/1.1.
    pub(crate) fn shared() -> Option<Arc<Self>> {
        static SHARED: OnceLock<Option<Arc<Framed>>> = OnceLock::new();
        SHARED
            .get_or_init(|| Self::for_options(&HttpOptions::default()).ok().flatten())
            .clone()
    }

    /// Send `wire` over HTTP/3 or HTTP/2 when its origin speaks one, and
    /// read the answer's head; `None` when it goes over HTTP/1.1.
    pub(crate) fn exchange(
        &self,
        wire: &Wire<'_>,
        payload: &mut Option<Payload<'_>>,
    ) -> Option<std::result::Result<Answer, ureq::Error>> {
        let origin = Origin::of(wire.url)?;
        #[cfg(feature = "http3")]
        if let Some(port) = self
            .h3
            .port_for(&origin, self.asked == Some(HttpVersion::Http3))
        {
            match self
                .h3
                .exchange(&origin, port, wire, payload.as_mut().map(Payload::reborrow))
            {
                Ok(answer) => return Some(Ok(answer)),
                Err(super::h3::Declined::Failed(error)) => return Some(Err(error)),
                Err(super::h3::Declined::Fallback) => {}
            }
        }
        let prior_knowledge = matches!(self.asked, Some(HttpVersion::Http2 | HttpVersion::Http3));
        match self.h2.exchange(
            &origin,
            prior_knowledge,
            wire,
            payload.as_mut().map(Payload::reborrow),
        ) {
            Ok(answer) => Some(Ok(answer)),
            Err(h2::Declined::Failed(error)) => Some(Err(error)),
            Err(h2::Declined::Http1) => None,
        }
    }

    /// Take up what the `Alt-Svc` of an answer from `url` advertises.
    #[cfg_attr(not(feature = "http3"), allow(clippy::unused_self))]
    pub(crate) fn learn(&self, url: &Url, headers: &Headers) {
        #[cfg(feature = "http3")]
        if matches!(self.asked, None | Some(HttpVersion::Http3)) {
            let Some(value) = headers.get("alt-svc") else {
                return;
            };
            let Some(origin) = Origin::of(url).filter(|origin| origin.secure) else {
                return;
            };
            if let Some(advice) = super::alt_svc::http3(value, &origin.host) {
                self.h3.advise(&origin, advice);
            }
        }
        #[cfg(not(feature = "http3"))]
        let _ = (url, headers);
    }
}
