//! `rust/src/object/client.rs`: the addressing and the retry schedule.
//!
//! Where a request is sent, how its path is spelled, which region signs it, and
//! how long the client waits before trying again are all settled before a
//! single request goes out - so they are pinned here, with no store to answer,
//! beside what the requests themselves look like on the wire.

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

mod accounting {
    use yggdryl::IOBase;

    use crate::mod_::{BUCKET, folder, payload, store};

    #[test]
    fn a_scan_that_reads_a_header_out_of_each_object_keeps_one_connection() {
        let store = store();
        for part in 0..6 {
            store.put(
                BUCKET,
                &format!("lake/{part:03}.parquet"),
                &payload(64 * 1024),
            );
        }
        let lake = folder(&store, "lake/");
        for leaf in lake.ls(false, false) {
            let leaf = leaf.expect("an entry");
            let mut stream = leaf.pstream_bytes(0, 4096).expect("a stream");
            let first = stream.next().expect("a chunk").expect("bytes");
            assert_eq!(first.len(), 4096);
            // The rest of the body is abandoned, which is what a header read does.
            drop(stream);
        }
        assert_eq!(
            store.connection_count(),
            1,
            "an abandoned body gives its connection back rather than burning it"
        );
    }
}

mod wire {

    use yggdryl::{Error, IOBase};

    use crate::mod_::{BUCKET, file, folder, path, payload, store};

    #[test]
    fn a_refused_signature_is_the_stores_own_verdict() {
        let store = store();
        store.require_access_key(Some("SOMEONEELSE"));
        let handle = file(&store, "lake/part.parquet");

        let error = handle.read_all_bytes().expect_err("a refusal");
        match &error {
            Error::Remote {
                service,
                operation,
                status,
                code,
                path,
                ..
            } => {
                assert_eq!(*service, "s3");
                assert_eq!(*operation, "GetObject");
                assert_eq!(*status, 403);
                assert_eq!(code.as_str(), "InvalidAccessKeyId");
                assert_eq!(path.as_str(), "s3://trades/lake/part.parquet");
            }
            other => panic!("expected a remote refusal, got {other:?}"),
        }
        // A refusal is neither an absence nor a conflict, so a caller repairing
        // one of those is never sent down the wrong branch.
        assert!(!error.is_absent());
        assert!(!error.is_conflict());
    }

    #[test]
    fn a_throttled_request_is_retried_and_a_refusal_is_not() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let handle = file(&store, "lake/part.parquet");

        // Two throttles, then the real answer: the default budget is three tries.
        store.fail_next(503, "SlowDown", 2);
        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("the object"), b"PAR1");
        assert_eq!(store.request_count(), 3, "two retries and the answer");

        // A refusal is the store's verdict, not a hiccup, so it is not retried.
        store.fail_next(403, "AccessDenied", 1);
        store.clear_requests();
        handle.read_all_bytes().expect_err("a refusal");
        assert_eq!(
            store.request_count(),
            1,
            "a refusal stands on the first answer"
        );
    }

    #[test]
    fn many_reads_share_one_connection_rather_than_reconnecting() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(4096));
        let handle = file(&store, "lake/part.bin");

        // Ranged reads, whole reads, and streams in turn: every one of them has
        // to leave its connection reusable.
        for _ in 0..10 {
            assert_eq!(handle.read_range_bytes(0, 64).expect("a range").len(), 64);
            assert_eq!(handle.read_all_bytes().expect("the object").len(), 4096);
            assert_eq!(handle.pstream_bytes(0, 1024).expect("a stream").count(), 4);
        }

        assert_eq!(store.request_count(), 30);
        // The number that matters: on a real store each extra connection is a TCP
        // and TLS handshake, which would dwarf the transfer a footer read makes.
        assert_eq!(
            store.connection_count(),
            1,
            "thirty requests went down one connection"
        );
    }

    #[test]
    fn a_refusal_is_never_read_as_an_empty_prefix_or_an_absent_object() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", b"AAPL,187.23");

        // A listing nobody was allowed to see says nothing about what is there,
        // so a non-recursive removal must refuse rather than report success.
        let mut lake = folder(&store, "lake/");
        store.fail_next(403, "AccessDenied", 1);
        let refused = lake.remove(false).expect_err("a refusal");
        assert!(!refused.is_absent(), "{refused}");
        assert_eq!(
            store.keys(BUCKET),
            vec!["lake/part.bin".to_owned()],
            "and nothing was deleted"
        );

        // The same for a location: absence reads as emptiness, a refusal does not.
        let handle = path(&store, "lake/part.bin");
        store.fail_next(403, "AccessDenied", 1);
        let refused = handle.read_all_bytes().expect_err("a refusal");
        assert!(
            matches!(refused, Error::Remote { status: 403, .. }),
            "{refused}"
        );

        let missing = path(&store, "lake/absent.bin");
        assert_eq!(
            missing.read_all_bytes().expect("absence reads empty"),
            Vec::<u8>::new()
        );
    }
}
