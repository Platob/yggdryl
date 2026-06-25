//! Opt-in **integration tests against real public endpoints**, covering the HTTP
//! versions the client speaks and the `Auto` ALPN negotiation/fallback.
//!
//! These hit the network and are deliberately **non-hermetic**, so every test is
//! `#[ignore]`d — the normal `cargo test` stays offline. Run them explicitly:
//!
//! ```text
//! # HTTP/1.1 + HTTP/3-pin (no extra feature needed):
//! cargo test -p yggdryl-http --test integration -- --ignored
//! # add the HTTP/2 transport for the h2 / Auto-negotiation cases:
//! cargo test -p yggdryl-http --features http2 --test integration -- --ignored
//! ```
//!
//! They use small `GET` requests to fast, highly-available CDNs and assert on the
//! **negotiated protocol version** rather than body contents, so they stay quick
//! and stable. The HTTP/2 transport connects directly (no HTTP `CONNECT` proxy
//! support yet), so these require direct outbound egress.

use std::time::Duration;

use yggdryl_http::{HttpRequest, HttpSession, HttpVersion};

/// A session with a short retry budget so a flaky hop fails fast rather than
/// hanging the suite.
fn session() -> HttpSession {
    HttpSession::new()
        .with_user_agent("yggdryl-http-integration")
        .with_retry(yggdryl_http::RetryConfig {
            max_retries: 1,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        })
}

/// Pinned HTTP/1.1 over TLS reports `Http11` and a real status.
#[test]
#[ignore = "hits the network"]
fn real_http11_over_tls() {
    let request = HttpRequest::get("https://example.com/")
        .unwrap()
        .with_http_version(HttpVersion::Http11);
    let response = session().send(request, false, false, false).unwrap();
    assert!(response.status() >= 200);
    assert_eq!(response.negotiated_version(), HttpVersion::Http11);
}

/// Pinned HTTP/2 over TLS negotiates `h2` against an h2-capable CDN and reports it.
#[cfg(feature = "http2")]
#[test]
#[ignore = "hits the network"]
fn real_http2_over_tls() {
    let request = HttpRequest::get("https://www.cloudflare.com/")
        .unwrap()
        .with_http_version(HttpVersion::Http2);
    let response = session().send(request, false, false, false).unwrap();
    assert!(response.status() >= 200);
    assert_eq!(response.negotiated_version(), HttpVersion::Http2);
}

/// `Auto` over TLS offers `h2`,`http/1.1` and adopts whichever the server picks —
/// h2 for an h2 CDN, http/1.1 as the fallback — reported accurately either way.
#[cfg(feature = "http2")]
#[test]
#[ignore = "hits the network"]
fn real_auto_negotiates_and_falls_back() {
    // An h2-capable host: Auto should land on HTTP/2.
    let h2 = session()
        .send(
            HttpRequest::get("https://www.google.com/").unwrap(),
            false,
            false,
            false,
        )
        .unwrap();
    assert_eq!(h2.negotiated_version(), HttpVersion::Http2);

    // A host that declines h2 falls back to HTTP/1.1 over the same TLS connection;
    // either outcome is valid, but it must be reported truthfully.
    let any = session()
        .send(
            HttpRequest::get("https://example.com/").unwrap(),
            false,
            false,
            false,
        )
        .unwrap();
    assert!(matches!(
        any.negotiated_version(),
        HttpVersion::Http2 | HttpVersion::Http11
    ));
}

/// HTTP/3 is not implemented: pinning it errors rather than silently downgrading,
/// whatever the server supports (Cloudflare advertises h3, but we do not speak it).
#[test]
#[ignore = "hits the network"]
fn real_http3_pin_is_unsupported() {
    let request = HttpRequest::get("https://www.cloudflare.com/")
        .unwrap()
        .with_http_version(HttpVersion::Http3);
    assert!(session().send(request, false, false, false).is_err());
}
