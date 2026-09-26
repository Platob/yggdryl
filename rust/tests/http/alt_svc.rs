//! `rust/src/http/alt_svc.rs`: what an `Alt-Svc` field says of HTTP/3 for its
//! origin.

use std::time::Duration;

use yggdryl::internals::http_alt_svc::http3;

const DAY: Duration = Duration::from_secs(86_400);

#[test]
fn an_h3_entry_on_the_same_host_is_taken_with_its_port_and_age() {
    assert_eq!(http3("h3=\":443\"", "example.com"), Some(Some((443, DAY))));
    assert_eq!(
        http3("h3=\":8443\"; ma=3600", "example.com"),
        Some(Some((8443, Duration::from_secs(3600))))
    );
    assert_eq!(
        http3("h3=\"example.com:444\"; ma=60; persist=1", "EXAMPLE.com"),
        Some(Some((444, Duration::from_secs(60))))
    );
    // The first usable entry of a list, drafts and other protocols passed.
    assert_eq!(
        http3(
            "h3-29=\":443\", h2=\":443\", h3=\":4433\"; ma=10",
            "example.com"
        ),
        Some(Some((4433, Duration::from_secs(10))))
    );
    assert_eq!(http3("h3=\"[::1]:443\"", "[::1]"), Some(Some((443, DAY))));
}

#[test]
fn clear_withdraws_and_anything_unusable_is_passed_over() {
    assert_eq!(http3("clear", "example.com"), Some(None));
    assert_eq!(http3(" CLEAR ", "example.com"), Some(None));
    for value in [
        "h3=\"other.example:443\"",
        "h2=\":443\"",
        "h3=:443",
        "h3=\":0\"",
        "h3=\":port\"",
        "h3=\":443\"; ma=soon",
        "",
    ] {
        assert_eq!(http3(value, "example.com"), None, "{value:?}");
    }
}
