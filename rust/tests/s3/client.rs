//! `rust/src/s3/client.rs`: the addressing and the retry schedule.
//!
//! Where a request is sent, how its path is spelled, which region signs it, and
//! how long the client waits before trying again are all settled before a
//! single request goes out - so they are pinned here, with no store to answer,
//! beside what the requests themselves look like on the wire.
//!
//! On Amazon S3 the region, the endpoint and the addressing style also come
//! from the [`Session`] the options carry - its profile, its environment, its
//! FIPS and dual-stack switches - so those readings are pinned here too, each
//! over a session whose files are text and whose environment is handed over,
//! so nothing of the machine's own decides.

use yggdryl::Url;
use yggdryl::aws::Session;
use yggdryl::internals::s3_client::{
    Client, DEFAULT_REGION, Endpoint, RETRY_BACKOFF, backoff, bucket_region_of,
    total_of_content_range,
};
use yggdryl::s3::S3Options;

fn url(text: &str) -> Url {
    Url::from_str(text).expect("a valid location")
}

/// Options that consult nothing outside the test.
fn sealed() -> S3Options {
    S3Options::default().with_environment(false)
}

/// A session that reads nothing but the configuration file `config`, and
/// consults no environment.
///
/// The options a client is built with seal a session they carry anyway when
/// they consult no environment themselves; text needs no environment, so the
/// profile it spells is read either way.
fn profiled(config: &str) -> Session {
    Session::new()
        .with_environment(false)
        .with_config_text(config)
}

/// Options that consult an environment, carrying a session whose environment
/// is exactly `variables` and whose shared files are `config` and nothing -
/// so what the AWS tools would read from the machine is what the test says.
///
/// The options sweep no prefix, so no variable of this process becomes a
/// knob; and they are anonymous, so the Google and Azure halves of the client
/// look for no identity of their own on the machine. Who signs plays no part
/// in where a request goes.
fn ambient(variables: &[(&str, &str)], config: &str) -> S3Options {
    let session = Session::new()
        .with_variables(variables.iter().copied())
        .with_config_text(config)
        .with_credentials_text("")
        .with_metadata_disabled(true);
    S3Options::default()
        .with_environment_prefixes(std::iter::empty::<String>())
        .with_anonymous(true)
        .with_session(session)
}

/// The client `location` and `options` describe.
fn client(location: &str, options: S3Options) -> Client {
    Client::new(&url(location), options).expect("a client")
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
        sealed().with_credentials(yggdryl::s3::Credentials::new("other", "secret")),
    )
    .expect("a client");
    let signer = explicit
        .signer_access_key_id(std::time::SystemTime::UNIX_EPOCH)
        .expect("a signer")
        .expect("credentials");
    assert_eq!(signer, "other");
}

#[test]
fn a_profile_that_asks_for_path_style_addresses_an_aws_location_in_the_path() {
    let session =
        profiled("[profile trading]\nregion = eu-west-3\ns3 =\n  addressing_style = path\n")
            .with_profile("trading");
    let path_style = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session.clone()),
    );
    assert_eq!(path_style.region(), "eu-west-3");
    assert_eq!(
        path_style.host_header("trades"),
        "s3.eu-west-3.amazonaws.com",
        "the bucket leaves the host"
    );
    assert_eq!(
        path_style.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );

    // The options' own choice wins over the profile's.
    let explicit = client(
        "s3://trades/lake/part.parquet",
        sealed().with_path_style(false).with_session(session),
    );
    assert_eq!(
        explicit.host_header("trades"),
        "trades.s3.eu-west-3.amazonaws.com"
    );

    // And `virtual` is a choice too: it holds even for the dotted bucket that
    // is otherwise kept in the path.
    let virtual_hosted = client(
        "s3://my.trades/part.parquet",
        sealed().with_session(profiled(
            "[default]\nregion = eu-west-3\ns3 =\n  addressing_style = virtual\n",
        )),
    );
    assert_eq!(
        virtual_hosted.host_header("my.trades"),
        "my.trades.s3.eu-west-3.amazonaws.com"
    );
}

#[test]
fn a_session_asking_for_fips_or_dual_stack_reaches_that_published_host() {
    let host = |session: Session| {
        client(
            "s3://trades/lake/part.parquet",
            sealed().with_session(session),
        )
        .host_header("trades")
    };
    let stated = Session::new()
        .with_environment(false)
        .with_region("eu-west-3");
    assert_eq!(
        host(stated.with_use_fips_endpoint(true)),
        "trades.s3-fips.eu-west-3.amazonaws.com",
        "virtual hosted, on the FIPS host"
    );
    assert_eq!(
        host(stated.with_use_dualstack_endpoint(true)),
        "trades.s3.dualstack.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        host(
            stated
                .with_use_fips_endpoint(true)
                .with_use_dualstack_endpoint(true)
        ),
        "trades.s3-fips.dualstack.eu-west-3.amazonaws.com"
    );

    // The profile's own switches reach the same hosts: `use_fips_endpoint`
    // at its top level, and the `s3` table's dual-stack and accelerate ones.
    assert_eq!(
        host(profiled(
            "[default]\nregion = eu-west-3\nuse_fips_endpoint = true\n"
        )),
        "trades.s3-fips.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        host(profiled(
            "[default]\nregion = eu-west-3\ns3 =\n  use_dualstack_endpoint = true\n"
        )),
        "trades.s3.dualstack.eu-west-3.amazonaws.com"
    );
    let accelerated = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(profiled(
            "[default]\nregion = eu-west-3\ns3 =\n  use_accelerate_endpoint = true\n",
        )),
    );
    assert_eq!(
        accelerated.host_header("trades"),
        "trades.s3-accelerate.amazonaws.com",
        "acceleration has one host for every region"
    );
    assert_eq!(
        accelerated.region(),
        "eu-west-3",
        "and still signs for the bucket's"
    );
}

#[test]
fn a_china_region_is_addressed_on_the_china_partition() {
    let china = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(
            Session::new()
                .with_environment(false)
                .with_region("cn-north-1"),
        ),
    );
    assert_eq!(china.region(), "cn-north-1");
    assert_eq!(
        china.host_header("trades"),
        "trades.s3.cn-north-1.amazonaws.com.cn",
        "virtual hosted, because the partition's host is AWS's own"
    );
    assert_eq!(
        china.path("trades", "lake/part.parquet"),
        "/lake/part.parquet"
    );
}

#[test]
fn the_region_comes_from_the_sessions_profile_when_the_url_and_options_name_none() {
    let session = profiled("[profile trading]\nregion = ap-southeast-2\n").with_profile("trading");
    let from_profile = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session.clone()),
    );
    assert_eq!(from_profile.region(), "ap-southeast-2");
    assert_eq!(
        from_profile.host_header("trades"),
        "trades.s3.ap-southeast-2.amazonaws.com"
    );

    // A region the session states outright beats its profile's.
    let stated = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session.with_region("eu-central-1")),
    );
    assert_eq!(stated.region(), "eu-central-1");

    // The location's own region comes ahead of the session's, and an explicit
    // one ahead of both.
    let from_url = client(
        "s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet",
        sealed().with_session(session.clone()),
    );
    assert_eq!(from_url.region(), "eu-west-3");
    let explicit = client(
        "s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet",
        sealed().with_region("us-west-2").with_session(session),
    );
    assert_eq!(explicit.region(), "us-west-2");

    // A profile the session names that neither file holds names no region,
    // and the default stands.
    let unwritten = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(
            profiled("[profile other]\nregion = eu-west-1\n").with_profile("trading"),
        ),
    );
    assert_eq!(unwritten.region(), DEFAULT_REGION);
}

#[test]
fn options_that_consult_no_environment_seal_the_session_they_carry() {
    // The session would read its region and its S3 endpoint out of the
    // environment it was handed; the options say no environment decides, so
    // neither does.
    let session = Session::new()
        .with_variables([
            ("AWS_REGION", "eu-west-3"),
            ("AWS_ENDPOINT_URL_S3", "http://localhost:9000"),
        ])
        .with_config_text("")
        .with_credentials_text("");
    let sealed_client = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session),
    );
    assert_eq!(sealed_client.region(), DEFAULT_REGION);
    assert_eq!(
        sealed_client.host_header("trades"),
        "trades.s3.us-east-1.amazonaws.com"
    );
    assert_eq!(sealed_client.scheme(), "https");
}

#[test]
fn the_s3_endpoint_the_sessions_environment_names_is_the_one_addressed() {
    // The service's own variable beats the generic one, as the AWS tools
    // read them, and an endpoint that is not AWS's is addressed path style.
    let named = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[
                ("AWS_ENDPOINT_URL_S3", "http://localhost:9000"),
                ("AWS_ENDPOINT_URL", "http://localhost:9999"),
                ("AWS_REGION", "eu-west-3"),
            ],
            "",
        ),
    );
    assert_eq!(named.scheme(), "http");
    assert_eq!(named.host_header("trades"), "localhost:9000");
    assert_eq!(
        named.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );
    assert_eq!(named.region(), "eu-west-3");

    // The generic one, when the service names none of its own.
    let generic = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_ENDPOINT_URL", "http://localhost:9999")], ""),
    );
    assert_eq!(generic.host_header("trades"), "localhost:9999");

    // `AWS_DEFAULT_REGION` when `AWS_REGION` is unset, and
    // `AWS_S3_FORCE_PATH_STYLE` out of the same environment.
    let regional = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[
                ("AWS_DEFAULT_REGION", "ap-south-1"),
                ("AWS_S3_FORCE_PATH_STYLE", "true"),
            ],
            "",
        ),
    );
    assert_eq!(regional.region(), "ap-south-1");
    assert_eq!(
        regional.host_header("trades"),
        "s3.ap-south-1.amazonaws.com"
    );
    assert_eq!(
        regional.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );

    // An explicit endpoint on the options wins over the environment's.
    let explicit = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_ENDPOINT_URL_S3", "http://localhost:9000")], "")
            .with_endpoint("http://localhost:9500"),
    );
    assert_eq!(explicit.host_header("trades"), "localhost:9500");
}

#[test]
fn an_endpoint_stated_on_a_session_is_honoured_by_options_that_consult_no_environment() {
    let stated = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(
            Session::new()
                .with_environment(false)
                .with_service_endpoint_url("s3", "http://localhost:9300/"),
        ),
    );
    assert_eq!(stated.host_header("trades"), "localhost:9300");
    assert_eq!(
        stated.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet",
        "a bare host is addressed path style"
    );
}

#[test]
fn the_profiles_services_section_names_the_s3_endpoint_unless_configured_ones_are_ignored() {
    const CONFIG: &str = "[default]\nregion = ap-southeast-2\nendpoint_url = http://localhost:9100\n\
                          services = lake\n\n[services lake]\ns3 =\n  endpoint_url = http://localhost:9200\n";

    // The `[services]` entry for S3 beats the profile's `endpoint_url`.
    let serviced = client("s3://trades/lake/part.parquet", ambient(&[], CONFIG));
    assert_eq!(serviced.host_header("trades"), "localhost:9200");
    assert_eq!(serviced.region(), "ap-southeast-2");

    // Without one, the profile's `endpoint_url` serves every service.
    let profiled_endpoint = client(
        "s3://trades/lake/part.parquet",
        ambient(&[], "[default]\nendpoint_url = http://localhost:9100\n"),
    );
    assert_eq!(profiled_endpoint.host_header("trades"), "localhost:9100");

    // The environment's variable beats both files.
    let variable = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_ENDPOINT_URL_S3", "http://localhost:9000")], CONFIG),
    );
    assert_eq!(variable.host_header("trades"), "localhost:9000");

    // And a process that ignores configured endpoints reaches the published
    // host, whichever of the two says so.
    let ignored = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", "true")], CONFIG),
    );
    assert_eq!(
        ignored.host_header("trades"),
        "trades.s3.ap-southeast-2.amazonaws.com"
    );
    let ignored_by_profile = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[],
            "[default]\nregion = ap-southeast-2\nendpoint_url = http://localhost:9100\n\
             ignore_configured_endpoint_urls = true\n",
        ),
    );
    assert_eq!(
        ignored_by_profile.host_header("trades"),
        "trades.s3.ap-southeast-2.amazonaws.com"
    );
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
