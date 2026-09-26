//! `rust/src/http/cookie.rs`: RFC 6265 cookies as a client keeps them.

use yggdryl::http::{Cookie, CookieJar, Headers};
use yggdryl::{Error, Url};

/// 2021-06-09T10:18:14Z in UTC nanoseconds.
const JUNE_9_2021: i64 = 1_623_233_894_000_000_000;
const SECOND: i64 = 1_000_000_000;

fn url(text: &str) -> Url {
    Url::from_str(text).expect("a URL")
}

fn cookie(header: &str, at: &str) -> Cookie {
    Cookie::from_set_cookie(header, &url(at), JUNE_9_2021).expect("a cookie")
}

fn refused(header: &str, at: &str) -> Error {
    Cookie::from_set_cookie(header, &url(at), JUNE_9_2021).expect_err("a refusal")
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_set_cookie_without_a_pair_is_refused_naming_the_position() {
    for header in ["", "session", "; Path=/", "=abc"] {
        let error = refused(header, "https://api.example.com/v1");
        match error {
            Error::Parse {
                target,
                position,
                reason,
            } => {
                assert_eq!(target, "http header", "{header:?}");
                assert_eq!(position, 0, "{header:?}");
                assert!(reason.starts_with("Set-Cookie:"), "{header:?}: {reason}");
            }
            other => panic!("{header:?}: {other:?}"),
        }
    }
}

#[test]
fn a_cookie_name_that_is_not_a_token_is_refused_at_the_byte() {
    let error = refused("ses sion=abc", "https://api.example.com/");
    let Error::Parse {
        position, reason, ..
    } = error
    else {
        panic!("{error:?}");
    };
    assert_eq!(position, 3);
    assert!(reason.contains("token"), "{reason}");
}

#[test]
fn a_cookie_value_with_a_control_byte_is_refused_at_the_byte() {
    let error = refused("session=ab\x01c", "https://api.example.com/");
    let Error::Parse {
        position, reason, ..
    } = error
    else {
        panic!("{error:?}");
    };
    assert_eq!(position, 10);
    assert!(reason.contains("control"), "{reason}");
}

#[test]
fn a_domain_that_does_not_cover_the_host_is_refused() {
    for (header, at) in [
        ("session=abc; Domain=other.com", "https://api.example.com/"),
        ("session=abc; Domain=ample.com", "https://api.example.com/"),
        ("session=abc; Domain=example.com", "http://127.0.0.1:8080/"),
    ] {
        let error = refused(header, at);
        let Error::Parse {
            position, reason, ..
        } = error
        else {
            panic!("{header:?}: {error:?}");
        };
        assert_eq!(position, "session=abc;".len(), "{header:?}");
        assert!(reason.contains("Domain"), "{header:?}: {reason}");
    }
}

#[test]
fn a_domain_naming_a_public_suffix_is_refused_so_no_origin_plants_a_supercookie() {
    for (header, at) in [
        ("session=abc; Domain=com", "https://evil.example.com/"),
        ("session=abc; Domain=.co.uk", "https://evil.example.co.uk/"),
        ("session=abc; Domain=github.io", "https://evil.github.io/"),
        // A name the list does not know is its own suffix, by the `*` rule.
        ("session=abc; Domain=internal", "https://app.internal/"),
    ] {
        let error = refused(header, at);
        let Error::Parse {
            position, reason, ..
        } = error
        else {
            panic!("{header:?}: {error:?}");
        };
        assert_eq!(position, "session=abc;".len(), "{header:?}");
        assert!(reason.contains("public suffix"), "{header:?}: {reason}");
    }
    // Refused, so a sibling origin under that suffix never receives it.
    let mut jar = CookieJar::new();
    let mut answer = Headers::new();
    answer
        .append("Set-Cookie", "sid=planted; Domain=com")
        .unwrap();
    let stored = jar.set_from_headers(&url("https://evil.example.com/"), &answer, JUNE_9_2021);
    assert_eq!(stored, 0);
    assert_eq!(
        jar.header_for(&url("https://victim-bank.com/"), JUNE_9_2021),
        None
    );
}

#[test]
fn a_host_that_is_itself_a_public_suffix_keeps_its_cookie_to_itself() {
    let kept = cookie("k=v; Domain=github.io", "https://github.io/");
    assert_eq!(kept.domain, "github.io");
    assert!(kept.host_only);
    assert!(kept.matches(&url("https://github.io/"), JUNE_9_2021));
    assert!(!kept.matches(&url("https://victim.github.io/"), JUNE_9_2021));
    // A registrable domain under a suffix still covers its subdomains.
    let cover = cookie("k=v; Domain=example.co.uk", "https://api.example.co.uk/");
    assert_eq!(cover.domain, "example.co.uk");
    assert!(!cover.host_only);
}

// --- reading -----------------------------------------------------------------

#[test]
fn a_bare_pair_is_a_host_only_session_cookie_at_the_default_path() {
    let cookie = cookie("session=abc123", "https://api.example.com/v1/orders/42");
    assert_eq!(cookie.name, "session");
    assert_eq!(cookie.value, "abc123");
    assert_eq!(cookie.domain, "api.example.com");
    assert!(cookie.host_only);
    assert_eq!(cookie.path, "/v1/orders");
    assert_eq!(cookie.expires, None);
    assert!(!cookie.secure);
    assert!(!cookie.http_only);
    assert_eq!(cookie.header_pair(), "session=abc123");
}

#[test]
fn the_default_path_is_the_directory_of_the_request_path() {
    for (at, path) in [
        ("https://h.example/", "/"),
        ("https://h.example", "/"),
        ("https://h.example/a", "/"),
        ("https://h.example/a/", "/a"),
        ("https://h.example/a/b/c", "/a/b"),
    ] {
        assert_eq!(cookie("k=v", at).path, path, "{at}");
    }
}

#[test]
fn every_attribute_is_read_by_name_in_any_case_and_an_unknown_one_is_ignored() {
    let cookie = cookie(
        "k=\"quoted\"; DOMAIN=.Example.COM; path=/v1; secure; HttpOnly; SameSite=Lax; Priority=High",
        "https://api.example.com/",
    );
    assert_eq!(cookie.value, "quoted");
    assert_eq!(cookie.domain, "example.com");
    assert!(!cookie.host_only);
    assert_eq!(cookie.path, "/v1");
    assert!(cookie.secure);
    assert!(cookie.http_only);
}

#[test]
fn a_path_not_opening_with_a_slash_and_an_empty_domain_fall_back_to_the_defaults() {
    let cookie = cookie(
        "k=v; Path=relative; Domain=",
        "https://api.example.com/v1/x",
    );
    assert_eq!(cookie.path, "/v1");
    assert_eq!(cookie.domain, "api.example.com");
    assert!(cookie.host_only);
}

#[test]
fn max_age_wins_over_expires_and_zero_or_negative_lapses_at_once() {
    let at = "https://api.example.com/";
    let both = cookie("k=v; Expires=Wed, 09 Jun 2031 10:18:14 GMT; Max-Age=90", at);
    assert_eq!(both.expires, Some(JUNE_9_2021 + 90 * SECOND));
    assert!(!both.is_expired(JUNE_9_2021 + 89 * SECOND));
    assert!(both.is_expired(JUNE_9_2021 + 90 * SECOND));

    for lapsed in ["k=v; Max-Age=0", "k=v; Max-Age=-1"] {
        let cookie = cookie(lapsed, at);
        assert!(cookie.is_expired(i64::MIN), "{lapsed}");
        assert!(!cookie.matches(&url(at), JUNE_9_2021), "{lapsed}");
    }

    // A Max-Age that is not an integer is ignored, as the section asks.
    assert_eq!(cookie("k=v; Max-Age=soon", at).expires, None);
    assert_eq!(cookie("k=v; Max-Age=1.5", at).expires, None);
}

#[test]
fn expires_reads_the_cookie_date_algorithm_in_every_shape_it_accepts() {
    let at = "https://api.example.com/";
    for header in [
        "k=v; Expires=Wed, 09 Jun 2021 10:18:14 GMT",
        "k=v; Expires=Wednesday, 09-Jun-21 10:18:14 GMT",
        "k=v; Expires=Wed Jun  9 10:18:14 2021",
        "k=v; Expires=9 june 2021 10:18:14",
        "k=v; expires=10:18:14 09 JUN 2021 +0000",
    ] {
        assert_eq!(cookie(header, at).expires, Some(JUNE_9_2021), "{header}");
    }
    // A two-digit year folds into 1970-2069.
    assert_eq!(
        cookie("k=v; Expires=Wed, 09 Jun 21 10:18:14 GMT", at).expires,
        Some(JUNE_9_2021)
    );
    assert_eq!(
        cookie("k=v; Expires=Thu, 01 Jan 70 00:00:00 GMT", at).expires,
        Some(0)
    );
}

#[test]
fn an_unreadable_expires_is_ignored_and_the_cookie_lasts_the_session() {
    let at = "https://api.example.com/";
    for header in [
        "k=v; Expires=tomorrow",
        "k=v; Expires=Wed, 31 Feb 2021 10:18:14 GMT",
        "k=v; Expires=Wed, 09 Jun 1600 10:18:14 GMT",
        "k=v; Expires=Wed, 09 Jun 2021 25:18:14 GMT",
        "k=v; Expires=09 Jun 2021",
        "k=v; Expires=Wed, 09 Jun 2021 10:18:14 GMT; Expires=Wed, 09 Jun 2021 10:18:14 GMT",
    ] {
        let cookie = cookie(header, at);
        let expected = if header.matches("Expires=").count() == 2 {
            Some(JUNE_9_2021)
        } else {
            None
        };
        assert_eq!(cookie.expires, expected, "{header}");
    }
}

// --- matching ----------------------------------------------------------------

#[test]
fn a_host_only_cookie_reaches_its_host_and_no_other() {
    let cookie = cookie("k=v", "https://api.example.com/");
    assert!(cookie.matches(&url("https://api.example.com/x"), JUNE_9_2021));
    assert!(cookie.matches(&url("http://API.EXAMPLE.COM/x"), JUNE_9_2021));
    assert!(!cookie.matches(&url("https://www.api.example.com/"), JUNE_9_2021));
    assert!(!cookie.matches(&url("https://example.com/"), JUNE_9_2021));
}

#[test]
fn a_domain_cookie_reaches_the_domain_and_every_host_under_it() {
    let cookie = cookie("k=v; Domain=example.com", "https://api.example.com/");
    for reached in [
        "https://example.com/",
        "https://api.example.com/",
        "https://deep.api.example.com/",
    ] {
        assert!(cookie.matches(&url(reached), JUNE_9_2021), "{reached}");
    }
    for unreached in [
        "https://notexample.com/",
        "https://example.com.evil/",
        "https://example.org/",
    ] {
        assert!(!cookie.matches(&url(unreached), JUNE_9_2021), "{unreached}");
    }
}

#[test]
fn an_address_host_matches_only_itself() {
    let cookie = cookie("k=v", "http://127.0.0.1:9000/");
    assert!(cookie.matches(&url("http://127.0.0.1:9000/x"), JUNE_9_2021));
    assert!(cookie.matches(&url("http://127.0.0.1/x"), JUNE_9_2021));
    assert!(!cookie.matches(&url("http://localhost:9000/x"), JUNE_9_2021));
}

#[test]
fn the_path_match_is_the_section_5_1_4_rule() {
    let at = "https://h.example/";
    let scoped = cookie("k=v; Path=/v1", at);
    for reached in [
        "https://h.example/v1",
        "https://h.example/v1/",
        "https://h.example/v1/orders",
    ] {
        assert!(scoped.matches(&url(reached), JUNE_9_2021), "{reached}");
    }
    for unreached in [
        "https://h.example/v10",
        "https://h.example/",
        "https://h.example/v",
    ] {
        assert!(!scoped.matches(&url(unreached), JUNE_9_2021), "{unreached}");
    }
    let slashed = cookie("k=v; Path=/v1/", at);
    assert!(slashed.matches(&url("https://h.example/v1/x"), JUNE_9_2021));
    assert!(!slashed.matches(&url("https://h.example/v1"), JUNE_9_2021));
    let root = cookie("k=v; Path=/", at);
    assert!(root.matches(&url("https://h.example"), JUNE_9_2021));
    assert!(root.matches(&url("https://h.example/anything"), JUNE_9_2021));
}

#[test]
fn a_secure_cookie_travels_over_https_alone() {
    let secure = cookie("k=v; Secure", "https://h.example/");
    assert!(secure.matches(&url("https://h.example/"), JUNE_9_2021));
    assert!(!secure.matches(&url("http://h.example/"), JUNE_9_2021));
    let plain = cookie("k=v", "https://h.example/");
    assert!(plain.matches(&url("http://h.example/"), JUNE_9_2021));
}

#[test]
fn an_expired_cookie_matches_nothing() {
    let cookie = cookie("k=v; Max-Age=10", "https://h.example/");
    assert!(cookie.matches(&url("https://h.example/"), JUNE_9_2021 + 9 * SECOND));
    assert!(!cookie.matches(&url("https://h.example/"), JUNE_9_2021 + 10 * SECOND));
}

// --- the jar -----------------------------------------------------------------

#[test]
fn a_jar_holds_one_cookie_per_name_domain_and_path_keeping_the_first_ones_place() {
    let mut jar = CookieJar::new();
    assert!(jar.is_empty());
    let at = "https://h.example/";
    jar.set(cookie("a=1", at));
    jar.set(cookie("b=2", at));
    jar.set(cookie("a=3", at));
    jar.set(cookie("a=4; Path=/v1", at));
    // The key is (name, domain, path): a domain cookie replaces the host-only
    // one of the same domain, in its place.
    jar.set(cookie("a=5; Domain=h.example", at));
    assert_eq!(jar.len(), 3);
    let values: Vec<String> = jar.iter().map(Cookie::header_pair).collect();
    assert_eq!(values, ["a=5", "b=2", "a=4"]);
    assert!(!jar.iter().next().expect("a=5").host_only);
    assert_eq!(
        jar.header_for(&url("https://h.example/"), JUNE_9_2021),
        Some("a=5; b=2".to_owned())
    );
    assert_eq!(
        jar.header_for(&url("https://h.example/v1/x"), JUNE_9_2021),
        Some("a=4; a=5; b=2".to_owned())
    );
    jar.clear();
    assert!(jar.is_empty());
}

#[test]
fn the_cookie_header_orders_longer_paths_first_then_older_cookies() {
    let mut jar = CookieJar::new();
    let at = "https://h.example/";
    jar.set(cookie("root=1", at));
    jar.set(cookie("deep=2; Path=/v1/orders", at));
    jar.set(cookie("mid=3; Path=/v1", at));
    jar.set(cookie("root2=4", at));
    jar.set(cookie("other=5; Path=/other", at));
    jar.set(cookie("secure=6; Secure", at));
    assert_eq!(
        jar.header_for(&url("https://h.example/v1/orders/42"), JUNE_9_2021),
        Some("deep=2; mid=3; root=1; root2=4; secure=6".to_owned())
    );
    assert_eq!(
        jar.header_for(&url("http://h.example/v1/orders/42"), JUNE_9_2021),
        Some("deep=2; mid=3; root=1; root2=4".to_owned())
    );
    assert_eq!(
        jar.header_for(&url("https://h.example/v2"), JUNE_9_2021),
        Some("root=1; root2=4; secure=6".to_owned())
    );
    assert_eq!(
        jar.header_for(&url("https://elsewhere.example/"), JUNE_9_2021),
        None
    );
}

#[test]
fn set_from_headers_stores_every_readable_set_cookie_and_evicts_what_lapsed() {
    let mut jar = CookieJar::new();
    let at = url("https://h.example/v1/");
    let headers = Headers::from_entries([
        ("Content-Type", "application/json"),
        ("Set-Cookie", "session=abc; Path=/; HttpOnly"),
        ("Set-Cookie", "broken"),
        ("Set-Cookie", "theme=dark; Max-Age=3600"),
        ("Set-Cookie", "old=gone; Max-Age=0"),
    ])
    .expect("headers");
    let stored = jar.set_from_headers(&at, &headers, JUNE_9_2021);
    assert_eq!(stored, 3, "the readable ones, the deletion included");
    let values: Vec<String> = jar.iter().map(Cookie::header_pair).collect();
    assert_eq!(
        values,
        ["session=abc", "theme=dark"],
        "the lapsed one evicted"
    );
    assert_eq!(
        jar.header_for(&at, JUNE_9_2021),
        Some("theme=dark; session=abc".to_owned()),
        "/v1 before /"
    );
    assert_eq!(jar.set_from_headers(&at, &Headers::new(), JUNE_9_2021), 0);
}

#[test]
fn remove_expired_evicts_by_the_clock_it_is_handed() {
    let mut jar = CookieJar::new();
    let at = "https://h.example/";
    jar.set(cookie("short=1; Max-Age=10", at));
    jar.set(cookie("long=2; Max-Age=1000", at));
    jar.set(cookie("session=3", at));
    assert_eq!(jar.remove_expired(JUNE_9_2021 + 5 * SECOND), 0);
    assert_eq!(jar.remove_expired(JUNE_9_2021 + 10 * SECOND), 1);
    assert_eq!(jar.remove_expired(JUNE_9_2021 + 5000 * SECOND), 1);
    assert_eq!(jar.len(), 1, "a session cookie never lapses");
    assert_eq!(
        jar.header_for(&url(at), i64::MAX),
        Some("session=3".to_owned())
    );
}

#[test]
fn a_cookie_built_by_hand_is_a_domain_cookie_over_every_path() {
    let mut jar = CookieJar::new();
    jar.set(Cookie::new("token", "t-1", ".Example.com"));
    let built = jar.iter().next().expect("one cookie");
    assert_eq!(built.domain, "example.com");
    assert!(!built.host_only);
    assert_eq!(built.path, "/");
    assert_eq!(
        jar.header_for(&url("http://api.example.com/v1"), JUNE_9_2021),
        Some("token=t-1".to_owned())
    );
}
