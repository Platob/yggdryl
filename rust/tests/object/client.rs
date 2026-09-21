//! `rust/src/object/client.rs`: the addressing and the retry schedule.
//!
//! Where a request is sent, how its path is spelled, which region signs it, and
//! how long the client waits before trying again are all settled before a
//! single request goes out - so they are pinned here, with no store to answer.
//! What the requests themselves look like on the wire is
//! `rust/tests/object/protocol.rs`.

use yggdryl::Url;
use yggdryl::internals::object_client::{
    Client, DEFAULT_REGION, Endpoint, RETRY_BACKOFF, backoff, bucket_region_of,
    total_of_content_range,
};
use yggdryl::object::ObjectOptions;

fn url(text: &str) -> Url {
    Url::from_str(text).expect("a valid location")
}

/// Options that consult nothing outside the test.
fn sealed() -> ObjectOptions {
    ObjectOptions::default().with_environment(false)
}

#[test]
fn an_aws_location_is_addressed_virtual_hosted_and_signed_for_its_region() {
    let client = Client::new(
        &url("s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet"),
        sealed(),
    )
    .expect("a client");
    assert_eq!(client.region(), "eu-west-3");
    assert_eq!(
        client.host_header("trades"),
        "trades.s3.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        client.path("trades", "lake/part.parquet"),
        "/lake/part.parquet"
    );
}

#[test]
fn a_bucket_only_location_defaults_its_endpoint_from_the_region() {
    let client = Client::new(&url("s3://trades/lake/part.parquet"), sealed()).expect("a client");
    assert_eq!(client.region(), DEFAULT_REGION);
    assert_eq!(
        client.host_header("trades"),
        "trades.s3.us-east-1.amazonaws.com"
    );

    // A dot in the bucket would break a TLS wildcard, so it stays in the path.
    let dotted = Client::new(&url("s3://my.trades/part.parquet"), sealed()).expect("a client");
    assert_eq!(
        dotted.host_header("my.trades"),
        "s3.us-east-1.amazonaws.com"
    );
    assert_eq!(
        dotted.path("my.trades", "part.parquet"),
        "/my.trades/part.parquet"
    );
}

#[test]
fn a_local_endpoint_is_addressed_path_style_over_its_own_scheme_and_port() {
    let client = Client::new(
        &url("s3://localhost:9000/trades/lake/part.parquet"),
        sealed().with_endpoint("http://localhost:9000"),
    )
    .expect("a client");
    assert_eq!(client.scheme(), "http");
    assert_eq!(client.host_header("trades"), "localhost:9000");
    assert_eq!(
        client.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );
}

#[test]
fn a_key_is_encoded_once_and_its_separators_survive() {
    let endpoint = Endpoint::new("https", "s3.example.io", None, true, None, false);
    assert_eq!(
        endpoint.path("trades", "year=2026/a b/c+d.parquet"),
        "/trades/year%3D2026/a%20b/c%2Bd.parquet"
    );
    // The bucket root has no trailing separator to sign over.
    assert_eq!(endpoint.path("trades", ""), "/trades");
}

#[test]
fn explicit_credentials_beat_a_url_that_carries_its_own() {
    let carried = Client::url_credentials(&url("s3://key:s3cr3t@trades/part.parquet"))
        .expect("credentials off the URL");
    assert_eq!(carried.access_key_id(), "key");

    let explicit = Client::new(
        &url("s3://key:s3cr3t@trades/part.parquet"),
        sealed().with_credentials(yggdryl::object::Credentials::new("other", "secret")),
    )
    .expect("a client");
    let signer = explicit
        .signer_access_key_id(std::time::SystemTime::UNIX_EPOCH)
        .expect("a signer")
        .expect("credentials");
    assert_eq!(signer, "other");
}

#[test]
fn a_content_range_states_the_total_and_a_redirect_states_the_region() {
    assert_eq!(total_of_content_range(Some("bytes 0-9/1024")), Some(1024));
    assert_eq!(total_of_content_range(Some("bytes */1024")), Some(1024));
    assert_eq!(total_of_content_range(Some("bytes 0-9/*")), None);
    assert_eq!(total_of_content_range(None), None);

    let region = [("x-amz-bucket-region".to_owned(), "eu-west-3".to_owned())];
    assert_eq!(bucket_region_of(301, &region), Some("eu-west-3".to_owned()));

    // A 200 is never a redirect, whatever headers it carries.
    assert_eq!(bucket_region_of(200, &region), None);
}

#[test]
fn the_backoff_doubles_and_stops_doubling() {
    assert_eq!(backoff(1), RETRY_BACKOFF);
    assert_eq!(backoff(2), RETRY_BACKOFF * 2);
    assert_eq!(backoff(3), RETRY_BACKOFF * 4);
    assert_eq!(backoff(20), RETRY_BACKOFF * 64);
}

#[test]
fn an_endpoint_splits_into_scheme_host_and_port() {
    assert_eq!(
        Client::split_endpoint("http://localhost:9000").expect("a split endpoint"),
        ("http".to_owned(), "localhost".to_owned(), Some(9000))
    );
    assert_eq!(
        Client::split_endpoint("s3.example.io").expect("a split endpoint"),
        ("https".to_owned(), "s3.example.io".to_owned(), None)
    );
    assert_eq!(
        Client::split_endpoint("https://[::1]:9000").expect("a split endpoint"),
        ("https".to_owned(), "[::1]".to_owned(), Some(9000))
    );
    Client::split_endpoint("https://host:notaport").expect_err("a refused port");
}
