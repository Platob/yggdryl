//! The **effective endpoint** helpers layered over the RFC 3986 split: scheme default-port
//! fallback ([`Uri::port_or_default`] / [`Uri::default_port`] and the free
//! [`default_port`](yggdryl_core::uri::default_port)) and IPv6 host unbracketing
//! ([`Uri::host_is_ipv6`] / [`Uri::host_unbracketed`]). Together they answer "what host and
//! port would this URI actually dial?" — all derived on read, so the stored URI, its
//! canonical bytes, and its value semantics are untouched.

use yggdryl_core::uri::{default_port, Authority, Uri, Url};

// -------------------------------------------------------------------------------------
// The scheme -> default-port table
// -------------------------------------------------------------------------------------

#[test]
fn default_port_covers_common_schemes() {
    for (scheme, port) in [
        ("http", 80),
        ("https", 443),
        ("ws", 80),
        ("wss", 443),
        ("ftp", 21),
        ("ssh", 22),
        ("postgres", 5432),
        ("postgresql", 5432),
        ("redis", 6379),
        ("mongodb", 27017),
    ] {
        assert_eq!(default_port(scheme), Some(port), "scheme {scheme:?}");
    }
}

#[test]
fn default_port_is_case_insensitive() {
    assert_eq!(default_port("HTTPS"), Some(443));
    assert_eq!(default_port("Https"), Some(443));
    assert_eq!(default_port("WSS"), Some(443));
}

#[test]
fn default_port_is_none_for_unknown_or_portless_schemes() {
    // A scheme with no network default (or one the table doesn't carry) is `None`, never a
    // guessed value.
    for scheme in ["s3", "mailto", "file", "urn", "made-up", ""] {
        assert_eq!(default_port(scheme), None, "scheme {scheme:?}");
    }
}

// -------------------------------------------------------------------------------------
// port_or_default / default_port on Uri
// -------------------------------------------------------------------------------------

#[test]
fn port_or_default_falls_back_to_the_scheme_default() {
    let implicit = Uri::parse_str("https://example.com/path").unwrap();
    assert_eq!(implicit.port(), None); // nothing was written
    assert_eq!(implicit.default_port(), Some(443));
    assert_eq!(implicit.port_or_default(), Some(443));
}

#[test]
fn explicit_port_wins_over_the_default() {
    let explicit = Uri::parse_str("https://example.com:8443/path").unwrap();
    assert_eq!(explicit.port(), Some(8443));
    assert_eq!(explicit.default_port(), Some(443)); // the scheme's default, unchanged
    assert_eq!(explicit.port_or_default(), Some(8443)); // but the explicit one is effective
}

#[test]
fn port_or_default_is_none_without_a_scheme_or_default() {
    // Scheme-less: no way to know a default.
    let relative = Uri::parse_str("//host/path").unwrap();
    assert_eq!(relative.default_port(), None);
    assert_eq!(relative.port_or_default(), None);

    // Scheme with no registered default and no explicit port.
    let s3 = Uri::parse_str("s3://bucket/key").unwrap();
    assert_eq!(s3.default_port(), None);
    assert_eq!(s3.port_or_default(), None);

    // ...but an explicit port is still returned even when the scheme has no default.
    let s3_port = Uri::parse_str("s3://bucket:9000/key").unwrap();
    assert_eq!(s3_port.port_or_default(), Some(9000));
}

// -------------------------------------------------------------------------------------
// Default-port fallback does NOT mutate the stored URI (round-trip preserved)
// -------------------------------------------------------------------------------------

#[test]
fn default_port_does_not_change_the_canonical_form() {
    let uri = Uri::parse_str("https://example.com/path").unwrap();
    // The effective port is 443, but it was never written into the URI.
    assert_eq!(uri.port_or_default(), Some(443));
    assert_eq!(uri.to_string(), "https://example.com/path"); // no ":443" appears
    assert_eq!(uri.serialize_bytes(), b"https://example.com/path");
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);

    // An implicit-port URI and the same URI written with its default port stay DISTINCT —
    // filling the default in on parse would have collapsed them.
    let explicit = Uri::parse_str("https://example.com:443/path").unwrap();
    assert_ne!(uri, explicit);
    assert_eq!(uri.port_or_default(), explicit.port_or_default()); // yet dial the same port
}

// -------------------------------------------------------------------------------------
// IPv6 host: is-ipv6 + unbracketing
// -------------------------------------------------------------------------------------

#[test]
fn ipv6_host_is_detected_and_unbracketed() {
    let uri = Uri::parse_str("http://[2001:db8::1]:8080/p").unwrap();
    assert!(uri.host_is_ipv6());
    assert_eq!(uri.host(), Some("[2001:db8::1]")); // stored bracketed
    assert_eq!(uri.host_unbracketed(), Some("2001:db8::1")); // bare address to dial
    assert_eq!(uri.port_or_default(), Some(8080));
}

#[test]
fn ipv6_loopback_default_port_and_unbracket() {
    let uri = Uri::parse_str("https://[::1]/status").unwrap();
    assert!(uri.host_is_ipv6());
    assert_eq!(uri.host_unbracketed(), Some("::1"));
    assert_eq!(uri.port_or_default(), Some(443)); // scheme default, host is IPv6
}

#[test]
fn reg_name_and_ipv4_hosts_are_not_ipv6_and_pass_through_unbracketed() {
    for host in ["example.com", "192.168.0.1", "localhost"] {
        let uri = Uri::parse_str(&format!("http://{host}/p")).unwrap();
        assert!(!uri.host_is_ipv6(), "host {host:?}");
        assert_eq!(uri.host_unbracketed(), Some(host), "host {host:?}");
    }
}

#[test]
fn unterminated_ipv6_bracket_is_not_treated_as_ipv6() {
    // The parser keeps `"[::1"` verbatim as a plain host; the accessors must agree and not
    // strip a non-existent closing bracket.
    let uri = Uri::parse_str("http://[::1/p").unwrap();
    assert!(!uri.host_is_ipv6());
    assert_eq!(uri.host_unbracketed(), Some("[::1"));
}

#[test]
fn host_accessors_are_none_without_an_authority() {
    let uri = Uri::parse_str("mailto:person@example.com").unwrap();
    assert!(!uri.host_is_ipv6());
    assert_eq!(uri.host_unbracketed(), None);
}

// -------------------------------------------------------------------------------------
// Authority-level accessors
// -------------------------------------------------------------------------------------

#[test]
fn authority_ipv6_accessors() {
    let bracketed = Authority::from_host("[fe80::1]");
    assert!(bracketed.host_is_ipv6());
    assert_eq!(bracketed.host_unbracketed(), "fe80::1");

    let plain = Authority::new(Some("u"), None, "db.internal", Some(5432));
    assert!(!plain.host_is_ipv6());
    assert_eq!(plain.host_unbracketed(), "db.internal");
}

// -------------------------------------------------------------------------------------
// Url mirrors every helper
// -------------------------------------------------------------------------------------

#[test]
fn url_mirrors_the_endpoint_helpers() {
    let url = Url::parse_str("wss://[::1]/socket").unwrap();
    assert_eq!(url.default_port(), Some(443));
    assert_eq!(url.port_or_default(), Some(443));
    assert!(url.host_is_ipv6());
    assert_eq!(url.host_unbracketed(), Some("::1"));

    let pg = Url::parse_str("postgres://svc@db:6000/app").unwrap();
    assert_eq!(pg.default_port(), Some(5432));
    assert_eq!(pg.port_or_default(), Some(6000)); // explicit wins
    assert!(!pg.host_is_ipv6());
}
