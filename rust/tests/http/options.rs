//! `rust/src/http/options.rs`: every knob, its default and its property spelling.

use std::path::Path;
use std::time::Duration;

use yggdryl::http::{Authorization, HttpOptions, Pagination};
use yggdryl::{Codec, DEFAULT_STREAM_BATCH_SIZE, Error, FieldPath, Url};

fn refused(name: &str, value: &str) -> Error {
    HttpOptions::from_properties([(name, value)]).expect_err("a refusal")
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_known_property_whose_value_does_not_parse_is_refused_naming_it() {
    for (name, value, expected) in [
        ("timeout", "soon", "seconds"),
        ("connect-timeout", "1m", "seconds"),
        ("max_pause", "-1", "at least zero"),
        ("max_attempts", "three", "whole number"),
        ("max_redirects", "-1", "whole number"),
        ("concurrency", "many", "whole number"),
        ("page_limit", "1.5", "whole number"),
        ("follow_redirects", "maybe", "true/false"),
        ("read_environment", "2", "true/false"),
        ("cookies", "jar", "true/false"),
        ("max_body_size", "lots", "byte count"),
        ("stream_batch_size", "64 pages", "byte count"),
        ("accept_encoding", "gzip, br", "content codings"),
        ("pagination", "scroll", "pagination"),
        ("records", "a b", "field path"),
        ("basic_auth", "nocolon", "user:password"),
        ("header.", "x", "header name"),
    ] {
        let error = refused(name, value);
        let Error::Parse { target, reason, .. } = &error else {
            panic!("{name}={value}: {error:?}");
        };
        assert_eq!(*target, "http option", "{name}={value}");
        assert!(reason.contains(name), "{name}={value}: {reason}");
        assert!(reason.contains(expected), "{name}={value}: {reason}");
        assert!(reason.contains(value), "{name}={value}: {reason}");
    }
}

// --- defaults ----------------------------------------------------------------

#[test]
fn every_knob_has_its_documented_default() {
    let options = HttpOptions::default();
    assert_eq!(options.base_url(), None);
    assert!(options.headers().is_empty());
    assert_eq!(options.authorization(), None);
    assert_eq!(options.timeout(), Duration::from_secs(120));
    assert_eq!(options.connect_timeout(), Duration::from_secs(10));
    assert_eq!(options.max_attempts(), 3);
    assert_eq!(options.max_redirects(), 10);
    assert!(options.follow_redirects());
    assert_eq!(
        options.user_agent(),
        format!("yggdryl/{}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(options.proxy(), None);
    assert_eq!(options.ca_bundle(), None);
    assert_eq!(
        options.accept_encodings(),
        [Codec::Gzip, Codec::Deflate, Codec::Zstd]
    );
    assert!(options.read_environment());
    assert_eq!(options.max_body_size(), 256 * 1024 * 1024);
    assert_eq!(options.stream_batch_size(), DEFAULT_STREAM_BATCH_SIZE);
    let parallelism = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    assert_eq!(options.concurrency(), parallelism.min(8));
    assert!(options.concurrency() >= 1);
    assert_eq!(*options.pagination(), Pagination::Auto);
    assert_eq!(options.records(), None);
    assert_eq!(options.page_limit(), None);
    assert!(options.cookies());
    assert_eq!(options.max_pause(), Duration::from_secs(30));

    assert_eq!(HttpOptions::DEFAULT_TIMEOUT, options.timeout());
    assert_eq!(
        HttpOptions::DEFAULT_CONNECT_TIMEOUT,
        options.connect_timeout()
    );
    assert_eq!(HttpOptions::DEFAULT_MAX_ATTEMPTS, options.max_attempts());
    assert_eq!(HttpOptions::DEFAULT_MAX_REDIRECTS, options.max_redirects());
    assert_eq!(HttpOptions::DEFAULT_USER_AGENT, options.user_agent());
    assert_eq!(HttpOptions::DEFAULT_MAX_BODY_SIZE, options.max_body_size());
    assert_eq!(HttpOptions::DEFAULT_MAX_PAUSE, options.max_pause());
    assert_eq!(HttpOptions::MAX_DEFAULT_CONCURRENCY, 8);
}

// --- setters -----------------------------------------------------------------

#[test]
fn every_setter_is_read_back_by_its_getter() {
    let base = Url::from_str("https://api.example.com/v1/").expect("a URL");
    let records = FieldPath::from_str("data.items").expect("a path");
    let options = HttpOptions::default()
        .with_base_url(base.clone())
        .with_header("X-Api-Key", "k-123")
        .expect("a header")
        .with_authorization(Authorization::bearer("t"))
        .with_timeout(Duration::from_secs(5))
        .with_connect_timeout(Duration::from_millis(250))
        .with_max_attempts(7)
        .with_max_redirects(2)
        .with_follow_redirects(false)
        .with_user_agent("trader/1.0")
        .with_proxy("http://proxy.local:3128")
        .with_ca_bundle("/etc/ssl/corp.pem")
        .with_accept_encodings([Codec::Identity])
        .with_read_environment(false)
        .with_max_body_size(1024)
        .with_stream_batch_size(4096)
        .with_concurrency(3)
        .with_pagination(Pagination::Link)
        .with_records(records.clone())
        .with_page_limit(Some(10))
        .with_cookies(false)
        .with_max_pause(Duration::from_secs(1));
    assert_eq!(options.base_url(), Some(&base));
    assert_eq!(options.headers().get("x-api-key"), Some("k-123"));
    assert_eq!(options.authorization(), Some(&Authorization::bearer("t")));
    assert_eq!(options.timeout(), Duration::from_secs(5));
    assert_eq!(options.connect_timeout(), Duration::from_millis(250));
    assert_eq!(options.max_attempts(), 7);
    assert_eq!(options.max_redirects(), 2);
    assert!(!options.follow_redirects());
    assert_eq!(options.user_agent(), "trader/1.0");
    assert_eq!(options.proxy(), Some("http://proxy.local:3128"));
    assert_eq!(options.ca_bundle(), Some(Path::new("/etc/ssl/corp.pem")));
    assert_eq!(options.accept_encodings(), [Codec::Identity]);
    assert!(!options.read_environment());
    assert_eq!(options.max_body_size(), 1024);
    assert_eq!(options.stream_batch_size(), 4096);
    assert_eq!(options.concurrency(), 3);
    assert_eq!(*options.pagination(), Pagination::Link);
    assert_eq!(options.records(), Some(&records));
    assert_eq!(options.page_limit(), Some(10));
    assert!(!options.cookies());
    assert_eq!(options.max_pause(), Duration::from_secs(1));
}

#[test]
fn a_zero_count_that_would_stop_everything_is_one_or_the_default() {
    let options = HttpOptions::default()
        .with_max_attempts(0)
        .with_concurrency(0)
        .with_stream_batch_size(0);
    assert_eq!(options.max_attempts(), 1);
    assert_eq!(options.concurrency(), 1);
    assert_eq!(options.stream_batch_size(), DEFAULT_STREAM_BATCH_SIZE);
}

// --- properties --------------------------------------------------------------

#[test]
fn every_property_reaches_its_knob_in_snake_and_kebab_case() {
    let snake = HttpOptions::from_properties([
        ("timeout", "30"),
        ("connect_timeout", "2.5"),
        ("max_attempts", "5"),
        ("max_redirects", "0"),
        ("follow_redirects", "false"),
        ("user_agent", "trader/1.0"),
        ("proxy", "http://proxy.local:3128"),
        ("ca_bundle", "/etc/ssl/corp.pem"),
        ("accept_encoding", "gzip, identity"),
        ("read_environment", "no"),
        ("max_body_size", "16MiB"),
        ("stream_batch_size", "4096"),
        ("concurrency", "2"),
        ("pagination", "link"),
        ("records", "data.items"),
        ("page_limit", "3"),
        ("cookies", "0"),
        ("max_pause", "5s"),
        ("base_url", "https://api.example.com/v1/"),
    ])
    .expect("options");
    let kebab = HttpOptions::from_properties([
        ("Timeout", "30s"),
        ("connect-timeout", "2500ms"),
        ("max-attempts", "5"),
        ("max-redirects", "0"),
        ("follow-redirects", "NO"),
        ("user-agent", "trader/1.0"),
        ("proxy-url", "http://proxy.local:3128"),
        ("ca-bundle", "/etc/ssl/corp.pem"),
        ("accept-encodings", "gzip,identity"),
        ("read-environment", "False"),
        ("max-body-size", "16 MB"),
        ("stream-batch-size", "4 KiB"),
        ("concurrency", "2"),
        ("Pagination", "LINK"),
        ("records", "data.items"),
        ("page-limit", "3"),
        ("cookies", "false"),
        ("max-pause", "5"),
        ("base-url", "https://api.example.com/v1/"),
    ])
    .expect("options");
    for options in [&snake, &kebab] {
        assert_eq!(options.timeout(), Duration::from_secs(30));
        assert_eq!(options.connect_timeout(), Duration::from_millis(2500));
        assert_eq!(options.max_attempts(), 5);
        assert_eq!(options.max_redirects(), 0);
        assert!(!options.follow_redirects());
        assert_eq!(options.user_agent(), "trader/1.0");
        assert_eq!(options.proxy(), Some("http://proxy.local:3128"));
        assert_eq!(options.ca_bundle(), Some(Path::new("/etc/ssl/corp.pem")));
        assert_eq!(options.accept_encodings(), [Codec::Gzip, Codec::Identity]);
        assert!(!options.read_environment());
        assert_eq!(options.max_body_size(), 16 * 1024 * 1024);
        assert_eq!(options.stream_batch_size(), 4096);
        assert_eq!(options.concurrency(), 2);
        assert_eq!(*options.pagination(), Pagination::Link);
        assert_eq!(
            options.records(),
            Some(&FieldPath::from_str("data.items").expect("a path"))
        );
        assert_eq!(options.page_limit(), Some(3));
        assert!(!options.cookies());
        assert_eq!(options.max_pause(), Duration::from_secs(5));
        assert_eq!(
            options.base_url().map(ToString::to_string).as_deref(),
            Some("https://api.example.com/v1/")
        );
    }
}

#[test]
fn durations_read_seconds_with_an_optional_s_or_ms_suffix() {
    for (value, expected) in [
        ("0", Duration::ZERO),
        ("1", Duration::from_secs(1)),
        ("1.5", Duration::from_millis(1500)),
        ("2s", Duration::from_secs(2)),
        ("2 S", Duration::from_secs(2)),
        ("250ms", Duration::from_millis(250)),
        ("0.5ms", Duration::from_micros(500)),
    ] {
        let options = HttpOptions::from_properties([("timeout", value)]).expect(value);
        assert_eq!(options.timeout(), expected, "{value}");
    }
}

#[test]
fn booleans_read_true_false_one_zero_yes_no_in_any_case() {
    for (value, expected) in [
        ("true", true),
        ("TRUE", true),
        ("1", true),
        ("yes", true),
        ("Yes", true),
        ("false", false),
        ("False", false),
        ("0", false),
        ("no", false),
        ("NO", false),
    ] {
        let options = HttpOptions::from_properties([("cookies", value)]).expect(value);
        assert_eq!(options.cookies(), expected, "{value}");
    }
}

#[test]
fn sizes_read_a_byte_count_with_a_binary_or_decimal_unit() {
    for (value, expected) in [
        ("1024", 1024),
        ("1KiB", 1024),
        ("1 kb", 1024),
        ("2MiB", 2 * 1024 * 1024),
        ("2m", 2 * 1024 * 1024),
        ("1GiB", 1024 * 1024 * 1024),
        ("3 GB", 3 * 1024 * 1024 * 1024),
        ("7b", 7),
    ] {
        let options = HttpOptions::from_properties([("max_body_size", value)]).expect(value);
        assert_eq!(options.max_body_size(), expected, "{value}");
    }
}

#[test]
fn header_properties_set_a_default_header_keeping_the_name_as_spelled() {
    let options = HttpOptions::from_properties([
        ("header.X-Api-Key", "k-123"),
        ("headers.Accept", "application/json"),
        ("HEADER.X-Trace", "abc"),
    ])
    .expect("options");
    assert_eq!(options.headers().get("X-Api-Key"), Some("k-123"));
    assert_eq!(options.headers().get("accept"), Some("application/json"));
    assert_eq!(options.headers().get("x-trace"), Some("abc"));
    assert_eq!(options.headers().len(), 3);
}

#[test]
fn credentials_arrive_as_a_bearer_token_or_a_basic_pair() {
    let bearer = HttpOptions::from_properties([("bearer_token", "t-1")]).expect("options");
    assert_eq!(bearer.authorization(), Some(&Authorization::bearer("t-1")));
    let basic = HttpOptions::from_properties([("basic-auth", "user:pa:ss")]).expect("options");
    assert_eq!(
        basic.authorization(),
        Some(&Authorization::basic("user", "pa:ss"))
    );
}

#[test]
fn an_unknown_property_and_an_empty_value_are_ignored() {
    let options = HttpOptions::from_properties([
        ("warehouse", "s3://lake"),
        ("s3.endpoint", "http://localhost:9000"),
        ("uri", "https://catalog.example.com"),
        ("timeout", "   "),
        ("max_attempts", ""),
    ])
    .expect("options");
    assert_eq!(options.timeout(), HttpOptions::DEFAULT_TIMEOUT);
    assert_eq!(options.max_attempts(), HttpOptions::DEFAULT_MAX_ATTEMPTS);
    assert_eq!(options.base_url(), None);
}

#[test]
fn with_properties_layers_onto_what_was_set_and_a_zero_page_limit_clears_it() {
    let options = HttpOptions::default()
        .with_timeout(Duration::from_secs(1))
        .with_page_limit(Some(4))
        .with_properties([("max_attempts", "9"), ("page_limit", "0")])
        .expect("options");
    assert_eq!(options.timeout(), Duration::from_secs(1), "untouched");
    assert_eq!(options.max_attempts(), 9);
    assert_eq!(options.page_limit(), None);
}
