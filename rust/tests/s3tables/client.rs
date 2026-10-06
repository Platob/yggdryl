//! `rust/src/s3tables/client.rs`: the one door every request leaves
//! through - where the service is, who signs, what goes on the wire, when a
//! request goes again, and what a refusal means.
//!
//! How a request is signed, and when a refused key earns a second send, is
//! `Request::with_sigv4`'s and pinned in `rust/tests/aws/request.rs`; here
//! the fake recomputes every signature on its own, so what is pinned is that
//! the client's requests are the ones the service would answer.

use serde_json::json;
use yggdryl::aws::{Credentials, Session};
use yggdryl::s3tables::S3Tables;
use yggdryl::{Error, Url};

use crate::fake::{ACCESS_KEY, S3TablesFake, SECRET_KEY, sha256_hex};
use crate::mod_::{LAKE_LABEL, arn, client, lake, refusal, scratch, session};

/// SHA-256 of the empty payload, which a request without a body signs.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// A session that reads `pairs` as its whole environment and no file.
fn offline(pairs: &[(&str, &str)]) -> Session {
    Session::new()
        .with_variables(pairs.iter().copied())
        .with_directory(scratch("offline"))
        .with_metadata_disabled(true)
}

/// A client of `session` sending to `fake`.
fn at_fake(session: Session, fake: &S3TablesFake) -> S3Tables {
    S3Tables::new(session)
        .try_with_endpoint_url(fake.endpoint())
        .expect("the fake's endpoint")
}

#[test]
fn the_region_is_what_was_stated_then_the_arns_then_the_sessions() {
    let paris = arn("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake");
    let tables = S3Tables::new(session().with_region("us-east-1"));

    // The session's region answers when nothing nearer does.
    assert_eq!(tables.region_of(None).expect("a region"), "us-east-1");
    // A table bucket is where its ARN says it is.
    assert_eq!(
        tables.region_of(Some(&paris)).expect("a region"),
        "eu-west-3"
    );
    // What the caller stated on the client wins over both.
    let stated = tables.clone().with_region(" ap-southeast-1 ");
    assert_eq!(
        stated.region_of(Some(&paris)).expect("a region"),
        "ap-southeast-1"
    );
    assert_eq!(stated.region_of(None).expect("a region"), "ap-southeast-1");
    // A blank region states none.
    let blank = stated.with_region("  ");
    assert_eq!(blank.region_of(None).expect("a region"), "us-east-1");

    // The environment's region is the session's.
    let from_environment = S3Tables::new(offline(&[("AWS_REGION", "sa-east-1")]));
    assert_eq!(
        from_environment.region_of(None).expect("a region"),
        "sa-east-1"
    );
}

#[test]
fn no_region_is_refused_naming_how_to_state_one_with_no_request() {
    let fake = S3TablesFake::start();
    let tables = at_fake(
        Session::new()
            .with_environment(false)
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY)),
        &fake,
    );

    let message = tables.region_of(None).expect_err("no region").to_string();
    for way in [
        "S3Tables::with_region",
        "Session::with_region",
        "AWS_REGION",
        "by its ARN",
    ] {
        assert!(message.contains(way), "{message}");
    }
    // The one verb that names no table bucket has no ARN to read one from,
    // and its refusal says which verb it was.
    let error = tables
        .create_table_bucket("lake")
        .expect_err("no region to sign for");
    assert!(matches!(&error, Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidInput));
    assert!(
        error
            .to_string()
            .contains("s3tables CreateTableBucket at \"lake\": expected a region"),
        "{error}"
    );
    assert_eq!(
        tables.table_buckets().count(),
        1,
        "one refusal, then nothing"
    );
    assert_eq!(fake.request_count(), 0);
}

#[test]
fn a_region_that_is_no_host_label_is_refused_naming_where_it_came_from_with_no_request() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);

    // Each region would be spelled into the host a signed request goes to:
    // `x.evil.test/` would send it - and its session token - elsewhere.
    for (tables, bucket, region, source) in [
        (
            client(&fake).with_region("x.evil.test/"),
            None,
            "x.evil.test/",
            "S3Tables::with_region",
        ),
        (
            client(&fake),
            Some(arn("arn:aws:s3tables:*:123456789012:bucket/lake")),
            "*",
            "the table bucket's ARN",
        ),
        (
            at_fake(session().with_region("user@host"), &fake),
            None,
            "user@host",
            "the session",
        ),
        (
            client(&fake).with_region("-east-1"),
            None,
            "-east-1",
            "S3Tables::with_region",
        ),
        (
            client(&fake).with_region("123"),
            None,
            "123",
            "S3Tables::with_region",
        ),
    ] {
        let error = tables
            .region_of(bucket.as_ref())
            .expect_err("no host label");
        match &error {
            // The one region rule every AWS host is built under.
            Error::Parse { target, reason, .. } => {
                assert_eq!(*target, "region");
                assert!(
                    reason.contains(&format!("the region {source} states"))
                        && reason.contains(region),
                    "{reason}"
                );
            }
            other => panic!("expected a refused region, got {other:?}"),
        }
        // Every verb resolves the region the same way, before it sends.
        let verb = match &bucket {
            Some(bucket) => tables.get_table_bucket(bucket).map(drop),
            None => tables.create_table_bucket("staging").map(drop),
        };
        assert!(matches!(verb, Err(Error::Parse { .. })), "{verb:?}");
    }
    // The bucket's own region is a host label: it wins over the session's.
    at_fake(session().with_region("user@host"), &fake)
        .get_table_bucket(&lake)
        .expect("the ARN's region");
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn the_endpoint_is_what_was_stated_then_the_sessions_then_the_partitions_host() {
    let endpoint =
        |tables: &S3Tables, region: &str| tables.endpoint_url(region).expect("an endpoint");
    let published = S3Tables::new(session());
    assert_eq!(
        endpoint(&published, "us-east-1"),
        "https://s3tables.us-east-1.amazonaws.com"
    );
    // The partition is the region's, so the suffix follows it.
    assert_eq!(
        endpoint(&published, "cn-north-1"),
        "https://s3tables.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        endpoint(&published, "us-gov-west-1"),
        "https://s3tables.us-gov-west-1.amazonaws.com"
    );

    // The session's switches name the service's other three hosts.
    let fips = S3Tables::new(session().with_use_fips_endpoint(true));
    assert_eq!(
        endpoint(&fips, "us-east-1"),
        "https://s3tables-fips.us-east-1.amazonaws.com"
    );
    let dualstack = S3Tables::new(session().with_use_dualstack_endpoint(true));
    assert_eq!(
        endpoint(&dualstack, "eu-west-3"),
        "https://s3tables.eu-west-3.api.aws"
    );
    let both = S3Tables::new(
        session()
            .with_use_fips_endpoint(true)
            .with_use_dualstack_endpoint(true),
    );
    assert_eq!(
        endpoint(&both, "us-west-2"),
        "https://s3tables-fips.us-west-2.api.aws"
    );
    assert_eq!(
        endpoint(&both, "cn-north-1"),
        "https://s3tables-fips.cn-north-1.api.amazonwebservices.com.cn"
    );
    let switched = S3Tables::new(offline(&[
        ("AWS_USE_FIPS_ENDPOINT", "true"),
        ("AWS_USE_DUALSTACK_ENDPOINT", "true"),
    ]));
    assert_eq!(
        endpoint(&switched, "us-east-1"),
        "https://s3tables-fips.us-east-1.api.aws"
    );

    // An endpoint the session names for the service wins over the host.
    let gateway =
        S3Tables::new(session().with_service_endpoint_url("s3tables", "http://localhost:4566/"));
    assert_eq!(endpoint(&gateway, "us-east-1"), "http://localhost:4566");
    let named = S3Tables::new(offline(&[
        ("AWS_ENDPOINT_URL", "http://everything.test"),
        ("AWS_ENDPOINT_URL_S3TABLES", "http://tables.test"),
    ]));
    assert_eq!(endpoint(&named, "us-east-1"), "http://tables.test");
    let shared = S3Tables::new(offline(&[("AWS_ENDPOINT_URL", "http://everything.test")]));
    assert_eq!(endpoint(&shared, "us-east-1"), "http://everything.test");

    // What the caller stated on the client wins over all of it, and a blank
    // one states none.
    let stated = gateway
        .try_with_endpoint_url("http://127.0.0.1:9000/")
        .expect("an endpoint");
    assert_eq!(endpoint(&stated, "us-east-1"), "http://127.0.0.1:9000");
    let blank = stated.try_with_endpoint_url(" ").expect("none stated");
    assert_eq!(endpoint(&blank, "us-east-1"), "http://localhost:4566");
    // The brackets of an IPv6 literal are the host's own.
    let literal = S3Tables::new(session())
        .try_with_endpoint_url("http://[::1]:4566/gateway/")
        .expect("an endpoint");
    assert_eq!(endpoint(&literal, "us-east-1"), "http://[::1]:4566/gateway");
}

// --- where a request arrives -------------------------------------------------
//
// Each test below reaches the fake through one source of the endpoint, and
// the source a wrong reading would take instead points at a loopback port
// nothing listens on. The fake answers as the service of a region no AWS
// partition publishes a host for, and every table bucket's ARN names it, so a
// client that dropped the endpoint altogether would ask for
// `s3tables[-fips].zz-nowhere-1.amazonaws.com` - a name that resolves to
// nothing - and fail on this machine rather than send a fixture-signed
// request to the real service.

/// A region no AWS partition publishes a host for.
const NOWHERE_REGION: &str = "zz-nowhere-1";

/// A loopback URL nothing listens on.
fn nowhere() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port");
    let address = listener.local_addr().expect("a bound address");
    drop(listener);
    format!("http://{address}")
}

/// A fake answering as the service of [`NOWHERE_REGION`], its table bucket
/// `lake`, and the label a request below that bucket names it by.
fn nowhere_lake() -> (S3TablesFake, yggdryl::Arn, String) {
    let fake = S3TablesFake::start();
    fake.set_region(NOWHERE_REGION);
    let lake = lake(&fake);
    let label = LAKE_LABEL.replace("us-east-1", NOWHERE_REGION);
    (fake, lake, label)
}

/// `tables` reaches `fake` for `lake`: its endpoint is the fake's, and a
/// reading of the bucket is one request there.
fn reaches(tables: &S3Tables, fake: &S3TablesFake, lake: &yggdryl::Arn, label: &str) {
    assert_eq!(
        tables.endpoint_url(NOWHERE_REGION).expect("an endpoint"),
        fake.endpoint()
    );
    fake.clear_requests();
    tables.get_table_bucket(lake).expect("the bucket");
    assert_eq!(fake.lines(), [format!("GET /buckets/{label}")]);
}

#[test]
fn a_stated_or_configured_endpoint_is_used_whatever_the_fips_and_dualstack_switches_say() {
    // botocore turns both switches off when an endpoint is given: they choose
    // among the published hosts, and a caller who names a gateway reaches it.
    let (fake, lake, label) = nowhere_lake();

    // An endpoint the client states, and the switch the session turns on.
    reaches(
        &at_fake(session().with_use_fips_endpoint(true), &fake),
        &fake,
        &lake,
        &label,
    );
    reaches(
        &at_fake(session().with_use_dualstack_endpoint(true), &fake),
        &fake,
        &lake,
        &label,
    );
    // An endpoint the session configures - a global AWS_ENDPOINT_URL - beside
    // the switch the environment turns on, and one it states for the service.
    reaches(
        &S3Tables::new(
            offline(&[
                ("AWS_ENDPOINT_URL", fake.endpoint().as_str()),
                ("AWS_USE_FIPS_ENDPOINT", "true"),
                ("AWS_USE_DUALSTACK_ENDPOINT", "true"),
            ])
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY)),
        ),
        &fake,
        &lake,
        &label,
    );
    reaches(
        &S3Tables::new(
            session()
                .with_use_dualstack_endpoint(true)
                .with_service_endpoint_url("s3tables", fake.endpoint()),
        ),
        &fake,
        &lake,
        &label,
    );
}

#[test]
fn aws_endpoint_url_s3tables_alone_is_where_a_request_arrives() {
    let (fake, lake, label) = nowhere_lake();
    let (endpoint, decoy) = (fake.endpoint(), nowhere());
    // A sealed session: these variables are its whole environment, its
    // `~/.aws` an empty directory of its own.
    let tables = S3Tables::new(
        offline(&[
            ("AWS_ENDPOINT_URL_S3TABLES", endpoint.as_str()),
            ("AWS_ENDPOINT_URL", decoy.as_str()),
        ])
        .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY)),
    );
    reaches(&tables, &fake, &lake, &label);
}

#[test]
fn the_services_section_s_s3tables_entry_is_where_a_request_arrives() {
    let (fake, lake, label) = nowhere_lake();
    let (endpoint, decoy) = (fake.endpoint(), nowhere());
    // The profile's own endpoint and another service's entry point nowhere.
    let tables = S3Tables::new(
        offline(&[])
            .with_config_text(format!(
                "[default]\nservices = local\nendpoint_url = {decoy}\n\n\
                 [services local]\ns3 =\n  endpoint_url = {decoy}\n\
                 s3tables =\n  endpoint_url = {endpoint}\n"
            ))
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY)),
    );
    reaches(&tables, &fake, &lake, &label);
}

#[test]
fn an_endpoint_no_request_can_go_to_is_refused_where_it_is_stated() {
    for (endpoint, position) in [
        ("ftp://tables.test", 0),
        // Read as a scheme and a path: the URL grammar refuses its port.
        ("tables.test:9000", 12),
        ("https://tables.test/?region=x", 20),
        ("https://tables.test/#part", 20),
    ] {
        match S3Tables::new(session()).try_with_endpoint_url(endpoint) {
            Err(Error::Parse {
                target,
                position: at,
                ..
            }) => {
                assert_eq!(target, "s3tables endpoint");
                assert_eq!(at, position, "{endpoint}");
            }
            other => panic!("expected a refused endpoint for {endpoint}, got {other:?}"),
        }
    }
}

#[test]
fn an_endpoint_carrying_user_information_is_refused_and_never_repeated() {
    let long = "p".repeat(60);
    for endpoint in [
        "http://user:hunter2@tables.test".to_owned(),
        "https://hunter2@tables.test:8443/gateway".to_owned(),
        // The `@` past the bytes a refusal quotes: masked before it is cut.
        format!("ftp://svc:hunter2{long}@tables.test"),
        format!("https://svc:hunter2{long}@tables.test"),
    ] {
        let error = S3Tables::new(session())
            .try_with_endpoint_url(&endpoint)
            .expect_err("user information in an endpoint");
        assert!(matches!(&error, Error::Parse { .. }), "{error:?}");
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(
                !rendered.contains("hunter2") && !rendered.contains("svc"),
                "{rendered}"
            );
        }
    }

    // One the session configures is refused the same way, by every request,
    // and neither the client nor its session renders it.
    let tables = S3Tables::new(
        session().with_service_endpoint_url("s3tables", "http://user:hunter2@tables.test"),
    );
    let error = tables
        .endpoint_url("us-east-1")
        .expect_err("user information in an endpoint");
    assert!(
        !format!("{error} {error:?}").contains("hunter2"),
        "{error:?}"
    );
    let tables = S3Tables::new(session().with_endpoint_url("http://user:hunter2@tables.test"));
    let lake = arn("arn:aws:s3tables:us-east-1:123456789012:bucket/lake");
    let error = tables.get_table_bucket(&lake).expect_err("refused");
    assert!(
        !format!("{error} {error:?}").contains("hunter2"),
        "{error:?}"
    );
    assert!(!format!("{tables:?}").contains("hunter2"), "{tables:?}");
}

#[test]
fn a_request_is_signed_for_the_region_its_table_bucket_is_in() {
    let fake = S3TablesFake::start();
    fake.set_region("eu-west-3");
    // The session is in another region; the bucket's own decides.
    let tables = at_fake(session().with_region("us-east-1"), &fake);
    let lake = lake(&fake);
    assert_eq!(lake.region(), Some("eu-west-3"));

    tables.get_table_bucket(&lake).expect("a bucket");
    let recorded = fake.requests();
    assert_eq!(recorded.len(), 1);
    let authorization = recorded[0].header("authorization").expect("a signature");
    assert!(
        authorization.contains("/eu-west-3/s3tables/aws4_request"),
        "{authorization}"
    );
}

#[test]
fn an_unsigned_session_is_refused_before_any_request() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);
    for session in [
        // Nothing stated and nothing consulted: the chain finds no source.
        Session::new().with_environment(false),
        // Signing turned off by name.
        session().with_anonymous(true),
    ] {
        let tables = at_fake(session.with_region("us-east-1"), &fake);
        let error = tables
            .get_table_bucket(&lake)
            .expect_err("no anonymous access");
        assert!(
            matches!(&error, Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("s3tables GetTableBucket at ")
                && message.contains("no credential source answered"),
            "{message}"
        );
        assert_eq!(tables.namespaces(&lake).count(), 1, "one refusal");
    }
    assert_eq!(fake.request_count(), 0);
}

#[test]
fn a_request_carries_its_scope_its_payload_hash_and_a_label_encoded_once() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    // A request with a body: the body's own hash, and its content type
    // signed beside the headers every request signs.
    tables
        .create_namespace(&lake, "trial")
        .expect("a namespace");
    // A request without one: the empty payload's hash.
    tables.get_namespace(&lake, "trial").expect("a namespace");

    let recorded = fake.requests();
    assert_eq!(recorded.len(), 2);
    let put = &recorded[0];
    assert_eq!(put.target, format!("/namespaces/{LAKE_LABEL}"));
    assert_eq!(put.body, r#"{"namespace":["trial"]}"#);
    assert_eq!(put.header("content-type"), Some("application/json"));
    assert_eq!(
        put.header("x-amz-content-sha256"),
        Some(sha256_hex(put.body.as_bytes()).as_str())
    );
    let authorization = put.header("authorization").expect("a signature");
    assert!(
        authorization.starts_with(&format!("AWS4-HMAC-SHA256 Credential={ACCESS_KEY}/")),
        "{authorization}"
    );
    assert!(
        authorization.contains("/us-east-1/s3tables/aws4_request, "),
        "{authorization}"
    );
    assert!(
        authorization.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, "),
        "{authorization}"
    );
    assert_eq!(put.header("host"), Some(fake.host().as_str()));
    assert!(!authorization.contains(SECRET_KEY));

    let get = &recorded[1];
    assert_eq!(get.target, format!("/namespaces/{LAKE_LABEL}/trial"));
    assert_eq!(get.header("x-amz-content-sha256"), Some(EMPTY_SHA256));
    assert_eq!(get.header("content-type"), None);
    assert!(
        get.header("authorization")
            .expect("a signature")
            .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date, ")
    );
    assert_eq!(get.header("x-amz-security-token"), None);
}

#[test]
fn a_temporary_set_sends_and_signs_its_session_token() {
    let fake = S3TablesFake::start();
    fake.accept_key(
        "ASIATEMPORARYSET",
        "temporary-secret",
        Some("temporary-token"),
    );
    let lake = lake(&fake);
    let tables = at_fake(
        Session::new().with_environment(false).with_credentials(
            Credentials::new("ASIATEMPORARYSET", "temporary-secret")
                .with_session_token("temporary-token"),
        ),
        &fake,
    );

    tables.get_table_bucket(&lake).expect("a bucket");
    let recorded = fake.requests();
    assert_eq!(recorded[0].signer(), "ASIATEMPORARYSET");
    assert_eq!(
        recorded[0].header("x-amz-security-token"),
        Some("temporary-token")
    );
    assert!(
        recorded[0]
            .header("authorization")
            .expect("a signature")
            .contains("x-amz-security-token")
    );
}

#[test]
fn an_endpoint_that_carries_a_path_keeps_it_on_the_wire_and_in_the_signature() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);
    // A gateway that mounts the service under a prefix: the fake has no
    // such operation, so it refuses the path - after checking a signature
    // that can only match if the prefix was signed as it was sent.
    let tables = S3Tables::new(session())
        .try_with_endpoint_url(format!("{}/gateway/", fake.endpoint()))
        .expect("an endpoint");
    let error = tables.get_table_bucket(&lake).expect_err("no such path");
    assert_eq!(refusal(&error), (404, "UnknownOperationException"));
    assert_eq!(fake.lines(), [format!("GET /gateway/buckets/{LAKE_LABEL}")]);
    let violations = fake.take_violations();
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert!(
        violations[0].contains("no operation of the model"),
        "the signature held, and only the path was unknown: {violations:#?}"
    );

    // That 404 is not the service's `NotFoundException`, so it is no
    // absence: a removal that reached no operation removed nothing, and
    // says so rather than succeeding.
    assert!(!error.is_absent(), "{error:?}");
    let error = tables
        .remove_table_bucket(&lake)
        .expect_err("a 404 that says nothing of the bucket");
    assert_eq!(refusal(&error), (404, "UnknownOperationException"));
    assert!(fake.has_bucket("lake"));
    assert_eq!(fake.take_violations().len(), 1);
}

#[test]
fn a_503_is_sent_again_for_a_read_and_a_removal_and_never_for_a_create() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");
    fake.seed_namespace("lake", "spare");

    // A read is safe to send again.
    fake.refuse_next(503, "ServiceUnavailableException", "busy", 1);
    fake.clear_requests();
    tables.get_table_bucket(&lake).expect("the second answer");
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| request.status)
            .collect::<Vec<_>>(),
        [503, 200]
    );

    // So is a throttled one - by its status, or by the error type botocore
    // reads as throttling under a `400`.
    for (status, error_type) in [
        (429, "TooManyRequestsException"),
        (400, "ThrottlingException"),
    ] {
        fake.refuse_next(status, error_type, "slow down", 1);
        fake.clear_requests();
        tables
            .get_namespace(&lake, "trial")
            .expect("the second answer");
        assert_eq!(fake.request_count(), 2, "{error_type}");
    }
    // A `400` that is not a throttle is the answer.
    fake.refuse_next(400, "BadRequestException", "no", 1);
    fake.clear_requests();
    tables
        .get_namespace(&lake, "trial")
        .expect_err("the one answer");
    assert_eq!(fake.request_count(), 1);

    // A removal is what the model marks idempotent.
    fake.refuse_next(503, "ServiceUnavailableException", "busy", 1);
    fake.clear_requests();
    tables
        .remove_namespace(&lake, "spare")
        .expect("the second answer");
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| (request.method.clone(), request.status))
            .collect::<Vec<_>>(),
        [("DELETE".to_owned(), 503), ("DELETE".to_owned(), 204)]
    );
    assert!(!fake.has_namespace("lake", "spare"));

    // A create is sent once: the service may have acted on the one it did
    // not answer - a throttled one included.
    for (status, error_type) in [
        (503, "ServiceUnavailableException"),
        (400, "ThrottlingException"),
    ] {
        for (operation, verb) in [
            (
                "CreateTable",
                &(|| {
                    tables
                        .create_table(&lake, "trial", "events", None)
                        .map(drop)
                }) as &dyn Fn() -> yggdryl::Result<()>,
            ),
            ("CreateNamespace", &|| {
                tables.create_namespace(&lake, "desk")
            }),
            ("CreateTableBucket", &|| {
                tables.create_table_bucket("staging").map(drop)
            }),
        ] {
            fake.refuse_next(status, error_type, "busy", 1);
            fake.clear_requests();
            let error = verb().expect_err("the one answer");
            assert_eq!(refusal(&error), (status, error_type));
            assert!(
                matches!(&error, Error::Remote { operation: named, .. } if *named == operation),
                "{error:?}"
            );
            assert_eq!(fake.request_count(), 1, "{operation} went out once");
        }
    }
    assert!(fake.table("lake", "trial", "events").is_none());
    assert!(!fake.has_namespace("lake", "desk"));
    assert!(!fake.has_bucket("staging"));

    // And so is a commit and a rename.
    let seeded = fake.seed_table("lake", "trial", "orders");
    let next = Url::from_str(&format!(
        "{}/metadata/00001-next.metadata.json",
        seeded.warehouse_location
    ))
    .expect("a location");
    fake.refuse_next(500, "InternalServerErrorException", "failed", 1);
    fake.clear_requests();
    let error = tables
        .update_table_metadata_location(&lake, "trial", "orders", &seeded.version_token, &next)
        .expect_err("the one answer");
    assert_eq!(refusal(&error), (500, "InternalServerErrorException"));
    assert_eq!(fake.request_count(), 1);
    fake.refuse_next(503, "ServiceUnavailableException", "busy", 1);
    fake.clear_requests();
    tables
        .rename_table(&lake, "trial", "orders", None, Some("fills"), None)
        .expect_err("the one answer");
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn the_sessions_attempts_bound_how_often_a_read_is_sent() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);
    let tables = at_fake(
        offline(&[("AWS_MAX_ATTEMPTS", "2")])
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY)),
        &fake,
    );

    fake.refuse_next(503, "ServiceUnavailableException", "busy", 2);
    let error = tables
        .get_table_bucket(&lake)
        .expect_err("two attempts, both refused");
    assert_eq!(refusal(&error), (503, "ServiceUnavailableException"));
    assert_eq!(fake.request_count(), 2);
}

/// A temporary set for `key`, as a credentials file holds one.
fn dumped(key: &str) -> String {
    format!(
        "[default]\naws_access_key_id = {key}\naws_secret_access_key = {key}-secret\n\
         aws_session_token = {key}-token\n"
    )
}

/// A client of `fake` signing as a session that reads the credentials file
/// under `directory` and nothing else: its variables are the test's own,
/// and the metadata services are off.
fn reading_files(fake: &S3TablesFake, directory: &std::path::Path) -> S3Tables {
    at_fake(
        Session::new()
            .with_variables([(
                "BOTO_CONFIG",
                directory.join("no-such-boto.cfg").display().to_string(),
            )])
            .with_directory(directory)
            .with_metadata_disabled(true),
        fake,
    )
}

/// Who signed each recorded request, and how it was answered.
fn signers(fake: &S3TablesFake) -> Vec<(String, u16)> {
    fake.requests()
        .iter()
        .map(|request| (request.signer(), request.status))
        .collect()
}

const EXPIRED: &str = "The security token included in the request is expired";

#[test]
fn a_set_the_service_calls_expired_is_signed_again_once_when_the_session_answers_another() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);
    for key in ["ASIAOLDDUMP", "ASIANEWDUMPED", "ASIATHIRDDUMP"] {
        fake.accept_key(key, &format!("{key}-secret"), Some(&format!("{key}-token")));
    }
    let directory = scratch("redumped");
    let credentials = directory.join("credentials");
    std::fs::write(&credentials, dumped("ASIAOLDDUMP")).expect("a dumped set");
    let tables = reading_files(&fake, &directory);
    tables.get_table_bucket(&lake).expect("a bucket");

    // A fresh set is dumped; the process still holds the old one until the
    // service says it lapsed - then the file is read again and the same
    // request goes once more, signed with what it says now.
    std::fs::write(&credentials, dumped("ASIANEWDUMPED")).expect("a set dumped anew");
    fake.refuse_next(403, "ExpiredTokenException", EXPIRED, 1);
    fake.clear_requests();
    tables.get_table_bucket(&lake).expect("a bucket");
    assert_eq!(
        signers(&fake),
        [
            ("ASIAOLDDUMP".to_owned(), 403),
            ("ASIANEWDUMPED".to_owned(), 200)
        ]
    );

    // A create is signed again the same way: the refusal says the service
    // took nothing, so the second send creates once.
    std::fs::write(&credentials, dumped("ASIATHIRDDUMP")).expect("a third set");
    fake.refuse_next(403, "ExpiredTokenException", EXPIRED, 1);
    fake.clear_requests();
    tables
        .create_namespace(&lake, "trial")
        .expect("a namespace");
    assert_eq!(
        signers(&fake),
        [
            ("ASIANEWDUMPED".to_owned(), 403),
            ("ASIATHIRDDUMP".to_owned(), 200)
        ]
    );
    assert!(fake.has_namespace("lake", "trial"));
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_set_the_service_calls_expired_is_not_sent_again_when_nothing_else_answers() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);

    // A pair the caller stated is what the session answers again, and the
    // same set would be refused the same way: the refusal stands after the
    // one request.
    let tables = client(&fake);
    fake.refuse_next(403, "ExpiredTokenException", EXPIRED, 1);
    let error = tables
        .get_table_bucket(&lake)
        .expect_err("the service's refusal");
    assert_eq!(refusal(&error), (403, "ExpiredTokenException"));
    assert_eq!(
        fake.request_count(),
        1,
        "no second request with the same set"
    );

    // A dumped set with nothing behind it: the session's own refusal names
    // the file and the way out, and the refused set is not sent again.
    fake.accept_key(
        "ASIASTALEDUMP",
        "ASIASTALEDUMP-secret",
        Some("ASIASTALEDUMP-token"),
    );
    let directory = scratch("stale-dump");
    std::fs::write(directory.join("credentials"), dumped("ASIASTALEDUMP")).expect("a dumped set");
    let tables = reading_files(&fake, &directory);
    fake.refuse_next(403, "ExpiredTokenException", EXPIRED, 1);
    fake.clear_requests();
    let message = tables
        .get_table_bucket(&lake)
        .expect_err("a refusal")
        .to_string();
    assert!(
        message.contains("shared credentials file") && message.contains("ASIA...DUMP"),
        "{message}"
    );
    assert_eq!(fake.request_count(), 1);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_key_the_service_does_not_recognize_is_signed_again_once_with_the_set_dumped_since() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);
    for key in ["ASIAROTATEDOUT", "ASIAROTATEDIN"] {
        fake.accept_key(key, &format!("{key}-secret"), Some(&format!("{key}-token")));
    }
    let directory = scratch("unrecognized");
    let credentials = directory.join("credentials");
    std::fs::write(&credentials, dumped("ASIAROTATEDOUT")).expect("a dumped set");
    let tables = reading_files(&fake, &directory);
    tables.get_table_bucket(&lake).expect("a bucket");

    // The key is rotated and the file dumped anew; the service no longer
    // recognizes the old one - the JSON services' spelling of an unknown
    // key - so the file is read again and the request goes once more.
    std::fs::write(&credentials, dumped("ASIAROTATEDIN")).expect("a set dumped anew");
    fake.refuse_next(
        403,
        "UnrecognizedClientException",
        "The security token included in the request is invalid.",
        1,
    );
    fake.clear_requests();
    tables.get_table_bucket(&lake).expect("a bucket");
    assert_eq!(
        signers(&fake),
        [
            ("ASIAROTATEDOUT".to_owned(), 403),
            ("ASIAROTATEDIN".to_owned(), 200)
        ]
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_refusal_of_the_request_rather_than_the_key_is_never_signed_again() {
    let fake = S3TablesFake::start();
    let lake = lake(&fake);
    let tables = client(&fake);

    // Each of these says something of the request - its signature, what it
    // may reach - and nothing of the key: the same request with another key
    // would be refused the same way, so it goes once.
    for code in [
        "InvalidSignatureException",
        "AccessDeniedException",
        "ForbiddenException",
    ] {
        fake.refuse_next(403, code, "refused", 1);
        fake.clear_requests();
        let error = tables.get_table_bucket(&lake).expect_err("a refusal");
        assert_eq!(refusal(&error), (403, code));
        assert_eq!(signers(&fake), [(ACCESS_KEY.to_owned(), 403)], "{code}");
    }
}

#[test]
fn every_error_type_is_the_services_refusal_as_it_said_it() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_table("lake", "trial", "events");

    // A rename is none of a read, a removal or a create, so nothing about
    // its refusal is read as anything but what the service said - and a
    // `PUT` the model does not mark idempotent is sent once whatever the
    // status.
    for (status, error_type) in [
        (400, "BadRequestException"),
        (403, "ForbiddenException"),
        (403, "AccessDeniedException"),
        (404, "NotFoundException"),
        (409, "ConflictException"),
        (429, "TooManyRequestsException"),
        (500, "InternalServerErrorException"),
    ] {
        let said = format!("the service said {error_type}");
        fake.refuse_next(status, error_type, &said, 1);
        fake.clear_requests();
        let error = tables
            .rename_table(&lake, "trial", "events", None, Some("fills"), None)
            .expect_err("a refusal");
        match &error {
            Error::Remote {
                service,
                operation,
                status: answered,
                code,
                message,
                path,
            } => {
                assert_eq!(*service, "s3tables");
                assert_eq!(*operation, "RenameTable");
                assert_eq!(*answered, status);
                // The type is what precedes the colon the service qualifies
                // it after.
                assert_eq!(code.as_str(), error_type);
                // The service's own words, then where the request went and
                // the region it was signed for, with where that came from.
                assert_eq!(
                    message.as_str(),
                    format!(
                        "{said} (sent to {}, signed for the region us-east-1 the table \
                         bucket's ARN states)",
                        fake.endpoint()
                    )
                );
                assert_eq!(path.as_str(), format!("{lake}/trial/events"));
            }
            other => panic!("expected the service's refusal, got {other:?}"),
        }
        assert!(!error.is_absent() && !error.is_conflict());
        assert_eq!(fake.request_count(), 1, "{error_type}");
    }
    assert!(fake.table("lake", "trial", "events").is_some());
}

#[test]
fn a_key_refused_in_an_opt_in_region_names_the_region_and_the_endpoint_before_the_key() {
    // AWS answers every key - sound or not - with the code an unknown key
    // earns in a region the account has not enabled, so the refusal says
    // where the request went and which region, and that the region is the
    // first thing to check.
    let fake = S3TablesFake::start();
    fake.set_region("eu-central-2");
    let lake = lake(&fake);
    assert_eq!(lake.region(), Some("eu-central-2"));
    let tables = client(&fake);
    for code in ["UnrecognizedClientException", "InvalidClientTokenId"] {
        fake.clear_requests();
        fake.refuse_next(
            403,
            code,
            "The security token included in the request is invalid.",
            1,
        );
        let error = tables.get_table_bucket(&lake).expect_err("a refused key");
        assert_eq!(refusal(&error), (403, code));
        let message = error.to_string();
        for named in [
            "The security token included in the request is invalid.",
            fake.endpoint().as_str(),
            "signed for the region eu-central-2 the table bucket's ARN states",
            "eu-central-2 is an opt-in region",
            "check the region before the key",
        ] {
            assert!(message.contains(named), "{named}: {message}");
        }
        // A key the caller stated is never traded for another: one request.
        assert_eq!(fake.request_count(), 1, "{code}");
    }

    // A code that refuses the request rather than its key says nothing of
    // the region's being enabled.
    fake.refuse_next(403, "AccessDeniedException", "not yours", 1);
    let message = tables
        .get_table_bucket(&lake)
        .expect_err("a refusal")
        .to_string();
    assert!(
        message.contains("signed for the region eu-central-2") && !message.contains("opt-in"),
        "{message}"
    );
}

#[test]
fn a_key_refused_in_a_region_enabled_by_default_names_the_region_and_no_opt_in() {
    let fake = S3TablesFake::start();
    fake.set_region("eu-west-3");
    let lake = lake(&fake);
    fake.refuse_next(
        403,
        "UnrecognizedClientException",
        "The security token included in the request is invalid.",
        1,
    );
    let message = client(&fake)
        .get_table_bucket(&lake)
        .expect_err("a refused key")
        .to_string();
    assert!(
        message.contains(&format!(
            "(sent to {}, signed for the region eu-west-3",
            fake.endpoint()
        )) && !message.contains("opt-in"),
        "{message}"
    );
}

#[test]
fn a_refusal_names_the_region_where_the_client_or_the_session_stated_it() {
    let fake = S3TablesFake::start();
    // A verb that names no table bucket signs for the session's region.
    fake.refuse_next(403, "ForbiddenException", "not yours", 1);
    let message = client(&fake)
        .create_table_bucket("staging")
        .expect_err("a refusal")
        .to_string();
    assert!(
        message.contains("signed for the region us-east-1 the session states"),
        "{message}"
    );
    // A region stated on the client wins over the bucket's own, and says so.
    let lake = lake(&fake);
    fake.refuse_next(403, "ForbiddenException", "not yours", 1);
    let message = client(&fake)
        .with_region("us-east-1")
        .get_table_bucket(&lake)
        .expect_err("a refusal")
        .to_string();
    assert!(
        message.contains("signed for the region us-east-1 S3Tables::with_region states"),
        "{message}"
    );
    // The service's message is bounded, so what follows it is never cut.
    let long = "x".repeat(4096);
    fake.refuse_next(400, "BadRequestException", &long, 1);
    let message = client(&fake)
        .get_table_bucket(&lake)
        .expect_err("a refusal")
        .to_string();
    assert!(
        message.ends_with("signed for the region us-east-1 the table bucket's ARN states)"),
        "{message}"
    );
}

#[test]
fn a_404_and_a_409_are_typed_only_where_the_verb_says_what_they_mean() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");

    // A read of what is not there: the crate's absence.
    let error = tables
        .get_namespace(&lake, "absent")
        .expect_err("no such namespace");
    assert!(
        matches!(&error, Error::Absent { expected, path }
            if *expected == "namespace" && path.as_str() == format!("{lake}/absent")),
        "{error:?}"
    );
    // A removal of what is not there: nothing left to do.
    tables
        .remove_namespace(&lake, "absent")
        .expect("already removed");
    // A create of what is there: the crate's conflict.
    let error = tables
        .create_namespace(&lake, "trial")
        .expect_err("a namespace of that name");
    assert!(
        matches!(&error, Error::Conflict { expected, actual, path }
            if *expected == "namespace" && *actual == "namespace"
                && path.as_str() == format!("{lake}/trial")),
        "{error:?}"
    );
    // A create under what is not there: the service's own 404, because the
    // missing thing is not what was addressed.
    let absent = arn(&fake.bucket_arn("absent"));
    let error = tables
        .create_namespace(&absent, "trial")
        .expect_err("no such bucket");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
    assert!(
        error.to_string().contains("table bucket does not exist"),
        "the service's message says which level is missing: {error}"
    );
}

#[test]
fn a_refusal_that_states_no_error_type_is_named_by_its_status_and_is_never_an_absence() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    // A gateway's 404: nothing says the service looked for the bucket.
    for verb in [
        &(|| tables.get_table_bucket(&lake).map(drop)) as &dyn Fn() -> yggdryl::Result<()>,
        &|| tables.remove_table_bucket(&lake),
    ] {
        fake.refuse_next(404, "", "no route", 1);
        let error = verb().expect_err("an answer the service did not give");
        assert_eq!(refusal(&error), (404, "NotFound"));
        assert!(!error.is_absent(), "{error:?}");
    }
    assert!(fake.has_bucket("lake"));
    // Neither is its 409 a conflict the service stated.
    fake.refuse_next(409, "", "busy", 1);
    let error = tables
        .create_namespace(&lake, "trial")
        .expect_err("an answer the service did not give");
    assert_eq!(refusal(&error), (409, "Conflict"));
    assert!(!error.is_conflict());
}

#[test]
fn the_error_type_is_read_wherever_the_protocol_lets_a_service_state_it() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");

    // In the body's `__type`, behind the namespace a service writes before
    // it: still the service's own NotFound, so a read is an absence...
    fake.answer_next(
        404,
        json!({"__type": "com.amazonaws.s3tables#NotFoundException", "message": "gone"}),
    );
    let error = tables.get_table_bucket(&lake).expect_err("not found");
    assert!(error.is_absent(), "{error:?}");
    // ...and, in the body's `code`, a removal is done.
    fake.answer_next(404, json!({"code": "NotFoundException", "message": "gone"}));
    tables
        .remove_namespace(&lake, "trial")
        .expect("already removed");
    assert!(
        fake.has_namespace("lake", "trial"),
        "the fake answered, and kept it"
    );

    // A conflict a create is told of the same way is the crate's.
    fake.answer_next(
        409,
        json!({"__type": "ConflictException:http://internal.amazon.com/", "message": "taken"}),
    );
    let error = tables
        .create_namespace(&lake, "desk")
        .expect_err("a conflict");
    assert!(error.is_conflict(), "{error:?}");

    // Any other type is the refusal's code, with the message beside it
    // under either spelling of its key.
    fake.answer_next(
        400,
        json!({"__type": "aws.protocols#BadRequestException", "Message": "no such member"}),
    );
    let error = tables
        .rename_table(&lake, "trial", "events", None, Some("fills"), None)
        .expect_err("a refusal");
    assert_eq!(refusal(&error), (400, "BadRequestException"));
    assert!(
        error.to_string().contains(": no such member (sent to "),
        "{error}"
    );

    // The header wins over the body when both state one.
    fake.refuse_next(403, "ForbiddenException", "not yours", 1);
    let error = tables.get_table_bucket(&lake).expect_err("a refusal");
    assert_eq!(refusal(&error), (403, "ForbiddenException"));
}

#[test]
fn an_answer_that_is_not_the_models_is_reported_rather_than_guessed() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    // A 200 whose document states none of the members the model requires.
    fake.refuse_next(200, "None", "not the model", 1);
    let error = tables.get_table_bucket(&lake).expect_err("no arn");
    match &error {
        Error::Remote {
            operation,
            status,
            code,
            message,
            path,
            ..
        } => {
            assert_eq!(*operation, "GetTableBucket");
            assert_eq!(*status, 200);
            assert_eq!(code.as_str(), "MalformedAnswer");
            assert!(message.contains("\"arn\""), "{message}");
            assert_eq!(path.as_str(), lake.to_string());
        }
        other => panic!("expected a malformed answer, got {other:?}"),
    }
}

#[test]
fn nothing_answering_is_an_io_failure_that_names_the_operation() {
    // A port nothing listens on: bound, read, and let go.
    let closed = std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .expect("a loopback port");
    let lake = arn("arn:aws:s3tables:us-east-1:123456789012:bucket/lake");
    let tables = S3Tables::new(
        offline(&[("AWS_MAX_ATTEMPTS", "1")])
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY)),
    )
    .try_with_endpoint_url(format!("http://{closed}"))
    .expect("an endpoint");

    let error = tables.get_table_bucket(&lake).expect_err("no listener");
    assert!(matches!(&error, Error::Io(_)), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("s3tables GetTableBucket")
            && message.contains("got no answer from")
            && message.contains(&closed.to_string()),
        "{message}"
    );
    // And a removal that reached nothing removed nothing: it is not success.
    tables
        .remove_table_bucket(&lake)
        .expect_err("no listener is not an absence");
}

#[cfg(feature = "internals")]
mod internal {
    //! What a caller cannot reach: the fake's own checks, each driven red by
    //! a request built by hand, since a check that has never refused
    //! anything proves nothing about the client that passes it - and the
    //! signer the client signs through, agreeing with the fake on the one
    //! spelling a signature for this service stands or falls on.

    use std::time::SystemTime;

    use yggdryl::http::{HttpOptions, Method, Session};
    use yggdryl::internals::aws_sigv4::{Signer, amz_date};

    use crate::fake::{ACCESS_KEY, S3TablesFake, SECRET_KEY, encode, sha256_hex};
    use crate::mod_::LAKE_LABEL;

    const LAKE: &str = "arn:aws:s3tables:us-east-1:123456789012:bucket/lake";

    /// What one hand-built request gets wrong, if anything.
    enum Flaw<'a> {
        None,
        /// Sign over this canonical URI rather than the path encoded again.
        Canonical(&'a str),
        /// State this payload hash rather than the body's own.
        PayloadHash(&'a str),
        /// Send no `x-amz-content-sha256` at all, as botocore sends none.
        NoPayloadHash,
        /// Send the body under no content type.
        NoContentType,
        /// Send no `authorization` header.
        NoAuthorization,
        /// Rewrite one part of the credential scope.
        Scope(&'a str, &'a str),
    }

    /// Send one request built and signed by hand with the fake's own reading
    /// of Signature Version 4, and answer the status and the error type it
    /// met.
    fn send(
        fake: &S3TablesFake,
        method: Method,
        wire: &str,
        body: &str,
        flaw: &Flaw<'_>,
    ) -> (u16, Option<String>) {
        let http = Session::with_options(HttpOptions::default().with_read_environment(false))
            .expect("an HTTP session");
        let (_, datetime) = amz_date(SystemTime::now());
        let body_hash = sha256_hex(body.as_bytes());
        let payload_hash = match flaw {
            Flaw::PayloadHash(hash) => Some((*hash).to_owned()),
            Flaw::NoPayloadHash => None,
            _ => Some(body_hash.clone()),
        };
        let content_type = !body.is_empty() && !matches!(flaw, Flaw::NoContentType);
        let mut signed = vec![
            ("host".to_owned(), fake.host()),
            ("x-amz-date".to_owned(), datetime.clone()),
        ];
        if let Some(hash) = &payload_hash {
            signed.push(("x-amz-content-sha256".to_owned(), hash.clone()));
        }
        if content_type {
            signed.push(("content-type".to_owned(), "application/json".to_owned()));
        }
        let canonical = match flaw {
            Flaw::Canonical(canonical) => (*canonical).to_owned(),
            _ => encode(wire, true),
        };
        let mut authorization = fake.authorization(
            method.as_str(),
            &canonical,
            "",
            &signed,
            payload_hash.as_deref().unwrap_or(&body_hash),
        );
        if let Flaw::Scope(from, to) = flaw {
            authorization = authorization.replace(from, to);
        }

        let mut request = http
            .request(method, &format!("{}{wire}", fake.endpoint()))
            .and_then(|request| request.with_header("x-amz-date", &datetime))
            .expect("a request")
            .with_body(body)
            .with_max_attempts(1)
            .with_idempotent(false);
        if let Some(hash) = &payload_hash {
            request = request
                .with_header("x-amz-content-sha256", hash)
                .expect("a header");
        }
        if content_type {
            request = request
                .with_header("content-type", "application/json")
                .expect("a header");
        }
        if !matches!(flaw, Flaw::NoAuthorization) {
            request = request
                .with_header("authorization", &authorization)
                .expect("a header");
        }
        let response = request.send().expect("an answer");
        (
            response.status().code(),
            response
                .headers()
                .get("x-amzn-errortype")
                .and_then(|value| value.split(':').next())
                .map(str::to_owned),
        )
    }

    /// The one violation the fake holds, taken.
    fn violation(fake: &S3TablesFake) -> String {
        let mut violations = fake.take_violations();
        assert_eq!(violations.len(), 1, "{violations:#?}");
        violations.remove(0)
    }

    #[test]
    fn the_signer_the_client_signs_through_spells_the_canonical_uri_as_the_fake_does() {
        // Every service but Amazon S3 signs the path as sent, encoded again:
        // the `%` of each escape becomes `%25`, and the separators stay.
        let signer = Signer::for_service("s3tables", ACCESS_KEY, SECRET_KEY, None, "us-east-1");
        for wire in [
            format!("/buckets/{LAKE_LABEL}"),
            format!("/namespaces/{LAKE_LABEL}/trial"),
            format!("/tables/{LAKE_LABEL}/trial/events/metadata-location"),
            "/buckets".to_owned(),
            "/get-table".to_owned(),
        ] {
            assert_eq!(signer.canonical_uri(&wire), encode(&wire, true), "{wire}");
        }
        assert_eq!(
            signer.canonical_uri(&format!("/buckets/{LAKE_LABEL}")),
            "/buckets/arn%253Aaws%253As3tables%253Aus-east-1%253A123456789012%253Abucket%252Flake"
        );
    }

    #[test]
    fn the_fake_refuses_a_path_signed_as_sent_and_accepts_it_signed_encoded_again() {
        let fake = S3TablesFake::start();
        fake.seed_bucket("lake");
        let wire = format!("/buckets/{LAKE_LABEL}");

        // Signed over the path as sent - what Amazon S3 alone accepts - the
        // signature does not match the one the service computes.
        assert_eq!(
            send(&fake, Method::Get, &wire, "", &Flaw::Canonical(&wire)),
            (403, Some("InvalidSignatureException".to_owned()))
        );
        assert!(violation(&fake).contains("the signature is"));

        // Signed over the path encoded again, it does - which is what every
        // request of the client passes to be answered at all.
        let signer = Signer::for_service("s3tables", ACCESS_KEY, SECRET_KEY, None, "us-east-1");
        assert_eq!(
            send(
                &fake,
                Method::Get,
                &wire,
                "",
                &Flaw::Canonical(&signer.canonical_uri(&wire))
            ),
            (200, None)
        );
        assert_eq!(
            send(&fake, Method::Get, &wire, "", &Flaw::None),
            (200, None)
        );
        assert!(fake.take_violations().is_empty());
    }

    #[test]
    fn the_fake_asks_no_more_of_a_request_than_the_service_does() {
        let fake = S3TablesFake::start();
        fake.seed_bucket("lake");
        let namespaces = format!("/namespaces/{LAKE_LABEL}");

        // botocore sends no `x-amz-content-sha256` to this service: the
        // service hashes the body it received and signs that.
        assert_eq!(
            send(
                &fake,
                Method::Put,
                &namespaces,
                r#"{"namespace":["trial"]}"#,
                &Flaw::NoPayloadHash
            ),
            (200, None)
        );
        assert_eq!(
            send(
                &fake,
                Method::Get,
                &format!("/buckets/{LAKE_LABEL}"),
                "",
                &Flaw::NoPayloadHash
            ),
            (200, None)
        );
        assert!(fake.has_namespace("lake", "trial"));
        assert!(fake.take_violations().is_empty());
    }

    #[test]
    fn the_fake_refuses_each_thing_a_request_must_not_get_wrong() {
        let fake = S3TablesFake::start();
        fake.seed_bucket("lake");
        let bucket = format!("/buckets/{LAKE_LABEL}");
        let namespaces = format!("/namespaces/{LAKE_LABEL}");
        let body = r#"{"namespace":["trial"]}"#;
        let invalid = (403, Some("InvalidSignatureException".to_owned()));
        let bad = (400, Some("BadRequestException".to_owned()));

        // No signature at all.
        assert_eq!(
            send(&fake, Method::Get, &bucket, "", &Flaw::NoAuthorization),
            (403, Some("MissingAuthenticationTokenException".to_owned()))
        );
        assert!(violation(&fake).contains("no authorization header"));

        // A scope that names another service, or another region.
        assert_eq!(
            send(
                &fake,
                Method::Get,
                &bucket,
                "",
                &Flaw::Scope("/s3tables/aws4_request", "/s3/aws4_request")
            ),
            invalid
        );
        assert!(violation(&fake).contains("does not end /s3tables/aws4_request"));
        assert_eq!(
            send(
                &fake,
                Method::Get,
                &bucket,
                "",
                &Flaw::Scope("/us-east-1/", "/eu-west-3/")
            ),
            invalid
        );
        assert!(violation(&fake).contains("names the region eu-west-3"));

        // A payload hash that is not the body's: the empty payload's, and
        // the marker Amazon S3 alone accepts.
        for hash in [sha256_hex(b"").as_str(), "UNSIGNED-PAYLOAD"] {
            assert_eq!(
                send(
                    &fake,
                    Method::Put,
                    &namespaces,
                    body,
                    &Flaw::PayloadHash(hash)
                ),
                invalid
            );
            assert!(violation(&fake).contains("the body received hashes to"));
        }

        // A body under no content type.
        assert_eq!(
            send(&fake, Method::Put, &namespaces, body, &Flaw::NoContentType),
            invalid
        );
        assert!(violation(&fake).contains("not application/json"));
        assert!(
            !fake.has_namespace("lake", "trial"),
            "nothing refused acted"
        );

        // A label sent as it is written, its `:` and `/` unescaped: the
        // signature holds, and the path is not one label.
        assert_eq!(
            send(
                &fake,
                Method::Get,
                &format!("/buckets/{LAKE}"),
                "",
                &Flaw::None
            ),
            bad
        );
        assert!(violation(&fake).contains("is not percent-encoded once"));

        // A label encoded twice: one label, and not an ARN.
        let twice = format!("/buckets/{}", LAKE_LABEL.replace('%', "%25"));
        assert_eq!(send(&fake, Method::Get, &twice, "", &Flaw::None), bad);
        assert!(violation(&fake).contains("is not a table bucket ARN"));

        // What the model bounds: an empty version token, a rename to nothing.
        let table = format!("/tables/{LAKE_LABEL}/trial/events");
        assert_eq!(
            send(
                &fake,
                Method::Put,
                &format!("{table}/rename"),
                "{}",
                &Flaw::None
            ),
            bad
        );
        assert!(violation(&fake).contains("names no new namespace and no new name"));
        assert_eq!(
            send(
                &fake,
                Method::Put,
                &format!("{table}/metadata-location"),
                r#"{"versionToken":"","metadataLocation":"s3://x--table-s3/m.json"}"#,
                &Flaw::None
            ),
            bad
        );
        assert!(violation(&fake).contains("an empty versionToken"));

        // And the same two requests, right, are answered.
        assert_eq!(
            send(&fake, Method::Put, &namespaces, body, &Flaw::None),
            (200, None)
        );
        assert_eq!(
            send(&fake, Method::Get, &bucket, "", &Flaw::None),
            (200, None)
        );
        assert!(fake.has_namespace("lake", "trial"));
    }
}
