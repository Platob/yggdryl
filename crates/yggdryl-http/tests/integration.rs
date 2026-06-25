//! Opt-in **integration tests against real public endpoints**, covering every HTTP
//! version the client speaks and the `Auto` ALPN negotiation/fallback.
//!
//! These hit the network and are deliberately **non-hermetic**, so every test is
//! `#[ignore]`d — the normal `cargo test` stays offline. Run them explicitly:
//!
//! ```text
//! # HTTP/1.1 + the HTTP/3-pin-unsupported case (no extra feature):
//! cargo test -p yggdryl-http --test integration -- --ignored
//! # the HTTP/2 and Auto-negotiation cases:
//! cargo test -p yggdryl-http --features http2 --test integration -- --ignored
//! # the HTTP/3-over-QUIC case:
//! cargo test -p yggdryl-http --features http3 --test integration -- --ignored
//! # everything at once:
//! cargo test -p yggdryl-http --features http2,http3 --test integration -- --ignored
//! ```
//!
//! They are **fast**: each uses a single `HEAD` request (no body transfer) to a
//! highly-available CDN and asserts on the **negotiated protocol version**, with a
//! short retry budget so a flaky hop fails quickly. The HTTP/2 and HTTP/3 transports
//! connect directly (no HTTP `CONNECT` proxy support yet), so these require direct
//! outbound egress.

use std::time::Duration;

use yggdryl_http::{HttpRequest, HttpSession, HttpVersion};

/// A session with a short retry budget so a flaky hop fails fast rather than
/// hanging the suite. If the standard `SSL_CERT_FILE` env var points at a CA
/// bundle, it is installed — so the suite also runs in a TLS-intercepting
/// environment (a corporate / sandbox MITM proxy) by trusting that proxy's CA.
fn session() -> HttpSession {
    let session = HttpSession::new()
        .with_user_agent("yggdryl-http-integration")
        .with_retry(yggdryl_http::RetryConfig {
            max_retries: 1,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        });
    match std::env::var("SSL_CERT_FILE") {
        Ok(path) if !path.is_empty() => session.with_ca_cert_file(&path).unwrap(),
        _ => session,
    }
}

/// Sends a `HEAD` request to `url` pinned to `version` and returns the protocol
/// version the response was actually delivered over (panicking on a transport
/// error). `HEAD` keeps it fast — headers only, no body.
fn negotiated_head(version: HttpVersion, url: &str) -> HttpVersion {
    let request = HttpRequest::head(url).unwrap().with_http_version(version);
    session()
        .send(request, false, false, false)
        .unwrap()
        .negotiated_version()
}

/// Pinned HTTP/1.1 over TLS reports `Http11`.
#[test]
#[ignore = "hits the network"]
fn real_http11_over_tls() {
    assert_eq!(
        negotiated_head(HttpVersion::Http11, "https://example.com/"),
        HttpVersion::Http11
    );
}

/// Pinned HTTP/2 over TLS negotiates `h2` against an h2-capable CDN and reports it.
#[cfg(feature = "http2")]
#[test]
#[ignore = "hits the network"]
fn real_http2_over_tls() {
    assert_eq!(
        negotiated_head(HttpVersion::Http2, "https://www.cloudflare.com/"),
        HttpVersion::Http2
    );
}

/// `Auto` over TLS offers `h2`,`http/1.1` and adopts whichever the server picks —
/// h2 for an h2 CDN, http/1.1 as the fallback — reported accurately either way.
#[cfg(feature = "http2")]
#[test]
#[ignore = "hits the network"]
fn real_auto_negotiates_and_falls_back() {
    // An h2-capable host: Auto lands on HTTP/2.
    assert_eq!(
        negotiated_head(HttpVersion::Auto, "https://www.google.com/"),
        HttpVersion::Http2
    );
    // Whatever a second host negotiates, Auto reports it truthfully (h2 or the
    // http/1.1 fallback over the same TLS connection).
    assert!(matches!(
        negotiated_head(HttpVersion::Auto, "https://example.com/"),
        HttpVersion::Http2 | HttpVersion::Http11
    ));
}

/// Pinned HTTP/3 over QUIC negotiates `h3` against an h3-capable CDN and reports it.
/// (Cloudflare/Google advertise and serve HTTP/3.)
#[cfg(feature = "http3")]
#[test]
#[ignore = "hits the network"]
fn real_http3_over_quic() {
    assert_eq!(
        negotiated_head(HttpVersion::Http3, "https://www.cloudflare.com/"),
        HttpVersion::Http3
    );
}

/// Without the `http3` feature, pinning HTTP/3 errors rather than silently
/// downgrading, whatever the server supports.
#[cfg(not(feature = "http3"))]
#[test]
#[ignore = "hits the network"]
fn real_http3_pin_is_unsupported() {
    let request = HttpRequest::head("https://www.cloudflare.com/")
        .unwrap()
        .with_http_version(HttpVersion::Http3);
    assert!(session().send(request, false, false, false).is_err());
}
