//! `rust/src/uri/handle.rs` over an Amazon S3 Tables table: what an
//! identifier used as a handle costs in requests when the location it names
//! is a table a table bucket keeps.
//!
//! A [`Uri`](yggdryl::Uri) resolves under no properties, so who signs and
//! where the control plane is are the process environment's alone. A process
//! has one environment, and writing it while another thread reads it is a
//! data race, so this target owns its process and holds exactly one test:
//! it points the environment at the fake control plane - the keys the fake
//! accepts, its endpoint, its region, shared files and a home of this test's
//! own - before anything reads it, and proves the session resolves the
//! fake's endpoint before a request is sent. Every other suite states its
//! identity on a sealed session or as properties and never touches the
//! environment; the mirror of `handle.rs` is `rust/tests/uri/handle.rs`.

#[cfg(feature = "s3tables")]
#[path = "support/s3tables.rs"]
mod fake;

/// A resolved table costs its one describing request for the value's life;
/// an absent one costs it at every operation and every accessor, because a
/// resolution that fails is kept nowhere.
#[cfg(feature = "s3tables")]
#[test]
fn a_table_resolves_once_when_it_is_there_and_at_every_ask_when_it_is_not() {
    use fake::{ACCESS_KEY, REGION, S3TablesFake, SECRET_KEY};
    use yggdryl::aws::Session;
    use yggdryl::holder::Holder;
    use yggdryl::{Arn, IOBase, IOKind, Uri};

    let fake = S3TablesFake::start();
    let bucket = fake.seed_bucket("lake");
    let table = fake.seed_table("lake", "desk", "quotes");
    let home = std::env::temp_dir().join(format!("yggdryl-s3tables-handle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("a home of this test's own");

    // SAFETY: `set_var` and `remove_var` are `unsafe` because another thread
    // reading the environment concurrently is a data race. This binary holds
    // only this test, nothing has read the environment yet, and the fake's
    // threads never do.
    unsafe {
        for name in [
            "AWS_PROFILE",
            "AWS_DEFAULT_PROFILE",
            "AWS_SESSION_TOKEN",
            "AWS_SECURITY_TOKEN",
            "AWS_CREDENTIAL_EXPIRATION",
            "AWS_DEFAULT_REGION",
            "AWS_ENDPOINT_URL_S3TABLES",
            "AWS_ENDPOINT_URL_S3",
            "AWS_IGNORE_CONFIGURED_ENDPOINT_URLS",
            "AWS_ROLE_ARN",
            "AWS_WEB_IDENTITY_TOKEN_FILE",
            "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            "AWS_USE_FIPS_ENDPOINT",
            "AWS_USE_DUALSTACK_ENDPOINT",
            "AWS_CA_BUNDLE",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("AWS_ACCESS_KEY_ID", ACCESS_KEY);
        std::env::set_var("AWS_SECRET_ACCESS_KEY", SECRET_KEY);
        std::env::set_var("AWS_REGION", REGION);
        std::env::set_var("AWS_ENDPOINT_URL", fake.endpoint());
        std::env::set_var("AWS_CONFIG_FILE", home.join("config"));
        std::env::set_var("AWS_SHARED_CREDENTIALS_FILE", home.join("credentials"));
        std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }
    // The environment is what an identifier resolves under: every request
    // below goes to the fake, or the test stops here.
    let ambient = Session::new();
    assert_eq!(
        ambient
            .service_endpoint("s3tables", REGION)
            .expect("an endpoint"),
        fake.endpoint()
    );
    assert_eq!(ambient.region().as_deref(), Some(REGION));

    let label = fake::encode(&bucket, false);
    let listed = "GET /buckets?maxBuckets=250".to_owned();
    let located = |name: &str| format!("GET /tables/{label}/desk/{name}/metadata-location");
    let described = |arn: &str| format!("GET /get-table?tableArn={}", fake::encode(arn, false));

    // A table that is there, by its location: the listing that finds the
    // bucket and the request that describes the table, on the first ask -
    // and nothing after it, the resolution being kept.
    let by_url = Uri::from_str("s3tables://lake/desk/quotes").expect("a location");
    fake.clear_requests();
    assert_eq!(fake.request_count(), 0, "building the identifier sent");
    assert_eq!(IOBase::kind(&by_url), IOKind::Table);
    assert_eq!(fake.lines(), [listed.clone(), located("quotes")]);
    assert_eq!(IOBase::kind(&by_url), IOKind::Table);
    assert!(by_url.is_tabular());
    assert!(by_url.is_container());
    assert_eq!(fake.request_count(), 2, "{:?}", fake.lines());

    // By its ARN: the one `GetTable` it addresses, once - as the identifier
    // it is, and as the `Holder` holding it, which resolves on its own.
    let arn = Arn::from_str(&table.arn).expect("a table's ARN");
    let by_arn = arn.clone().into_uri();
    fake.clear_requests();
    for _ in 0..3 {
        assert_eq!(IOBase::kind(&by_arn), IOKind::Table);
    }
    assert!(by_arn.is_tabular());
    assert_eq!(fake.lines(), [described(&table.arn)]);
    let held = Holder::from(arn);
    assert!(matches!(held, Holder::Uri(_)));
    fake.clear_requests();
    assert_eq!(held.kind(), IOKind::Table);
    assert_eq!(held.kind(), IOKind::Table);
    assert_eq!(fake.lines(), [described(&table.arn)]);

    // A table that is not there: the resolution fails, is kept nowhere, and
    // is sent again by every accessor - the empty answer each time - and by
    // every operation, whose error is the absence.
    let absent = Uri::from_str("s3tables://lake/desk/missing").expect("a location");
    fake.clear_requests();
    assert_eq!(IOBase::kind(&absent), IOKind::Unknown);
    assert!(!absent.is_tabular());
    assert_eq!(absent.size(), 0);
    let error = absent.read_all_bytes().expect_err("no such table");
    assert!(error.is_absent(), "{error}");
    let asked = [listed, located("missing")];
    assert_eq!(fake.lines(), [&asked[..], &asked, &asked, &asked].concat());

    let unknown = format!("{bucket}/table/00000000-0000-4000-8000-00000000beef");
    let absent = Arn::from_str(&unknown).expect("a table's ARN").into_uri();
    fake.clear_requests();
    assert_eq!(IOBase::kind(&absent), IOKind::Unknown);
    assert!(!absent.is_container());
    let error = absent.read_all_bytes().expect_err("no such table");
    assert!(error.is_absent(), "{error}");
    assert_eq!(
        fake.lines(),
        [
            described(&unknown),
            described(&unknown),
            described(&unknown)
        ]
    );
    let _ = std::fs::remove_dir_all(&home);
}
