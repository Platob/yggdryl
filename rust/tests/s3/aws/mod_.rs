//! `rust/src/s3/aws/mod.rs`: what is Amazon's own - the region a bucket is
//! signed for, the digest a bulk delete carries, and the
//! [`Session`](yggdryl::aws::Session) a client signs as: a role it trades
//! for once and signs everything after with, a request it sends unsigned, a
//! pair a location carries, and a set the store calls expired.

mod protocol {

    use std::path::PathBuf;

    use yggdryl::aws::{AssumedRole, Session};
    use yggdryl::s3::S3Options;
    use yggdryl::{Error, IOBase};

    use crate::mod_::{BUCKET, file, file_with, folder, options, payload, store};
    use crate::server::Recorded;

    /// A directory of this test's own, empty when handed over, for the
    /// `~/.aws` whose CLI cache a traded session is filed in - so no run reads
    /// back a session an earlier one traded for, and nothing is written under
    /// the machine's own home.
    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("yggdryl-s3-aws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    /// Whether a recorded request is the STS exchange rather than a bucket one.
    fn is_exchange(request: &Recorded) -> bool {
        request
            .query
            .iter()
            .any(|(name, value)| name == "Action" && value == "AssumeRole")
    }

    /// The `Authorization` header a recorded request carried, or nothing.
    fn authorization(request: &Recorded) -> String {
        request
            .headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }

    /// Options that reach `store` with its fixture keys and sign as a session
    /// that consults nothing outside the test, assumes `role` at the store's
    /// own endpoint, and files what it traded for under `directory`.
    ///
    /// The options' explicit pair is what the session signs the exchange
    /// with: it is the base the role is traded from, never what the bucket
    /// requests sign as.
    fn assuming(store: &crate::server::FakeS3, role: AssumedRole, directory: PathBuf) -> S3Options {
        options(store).with_session(
            Session::new()
                .with_environment(false)
                .with_directory(directory)
                // STS is a host of its own on AWS; the fixture shares this one.
                .with_assumed_role(role.with_endpoint(store.endpoint())),
        )
    }

    #[test]
    fn a_bucket_in_another_region_is_signed_again_for_the_region_it_is_in() {
        let store = store();
        store.set_bucket_region(BUCKET, Some("eu-west-3"));
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("the object"), b"PAR1");
        assert_eq!(
            store.request_count(),
            2,
            "the redirect names the region, and the retry is signed for it"
        );
        let recorded = store.requests();
        let authorization = recorded[1]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.clone())
            .expect("an authorization header");
        assert!(authorization.contains("/eu-west-3/s3/"), "{authorization}");

        // The region sticks, so the next read costs one request again.
        store.clear_requests();
        handle.read_all_bytes().expect("the object");
        assert_eq!(store.request_count(), 1, "the correction is learned once");
    }

    #[test]
    fn a_bulk_delete_carries_the_digest_s3_requires() {
        let store = store();
        for part in 0..3 {
            store.put(BUCKET, &format!("lake/part-{part}.parquet"), b"PAR1");
        }
        let mut lake = folder(&store, "lake/");

        store.clear_requests();
        lake.clear().expect("an emptied prefix");
        let recorded = store.requests();
        let delete = recorded
            .iter()
            .find(|request| request.method == "POST")
            .expect("the bulk delete");
        assert!(
            delete.query.iter().any(|(name, _)| name == "delete"),
            "the delete sub-resource names the operation"
        );
        assert!(
            delete.headers.iter().any(|(name, _)| name == "content-md5"),
            "S3 refuses a bulk delete without Content-MD5"
        );
    }

    #[test]
    fn a_named_role_is_traded_for_a_session_once_and_signs_everything_after() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(64));
        let directory = scratch("named-role");
        let role = AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
            .with_session_name("power-desk")
            .with_external_id("desk-42");
        let handle = file_with("lake/part.bin", assuming(&store, role, directory.clone()));

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        assert_eq!(handle.read_range_bytes(0, 8).expect("a read"), payload(8));

        let recorded = store.requests();
        let exchanges: Vec<&Recorded> = recorded
            .iter()
            .filter(|request| is_exchange(request))
            .collect();
        assert_eq!(
            exchanges.len(),
            1,
            "one exchange, however many requests follow it: {:?}",
            recorded
                .iter()
                .map(|request| &request.path)
                .collect::<Vec<_>>()
        );

        // The exchange says what was asked for, and is signed for STS rather
        // than for S3 - a different service in the scope is a different
        // signing key - by the options' own pair, which is the base the role
        // is traded from.
        let exchange = exchanges[0];
        let asked = |name: &str| {
            exchange
                .query
                .iter()
                .find(|(held, _)| held == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(asked("Version"), Some("2011-06-15"));
        assert_eq!(
            asked("RoleArn"),
            Some("arn:aws:iam::123456789012:role/lake-reader")
        );
        assert_eq!(asked("RoleSessionName"), Some("power-desk"));
        assert_eq!(asked("ExternalId"), Some("desk-42"));
        assert_eq!(asked("DurationSeconds"), Some("3600"));
        let signed = authorization(exchange);
        assert!(signed.contains("/sts/aws4_request"), "{signed}");
        assert!(
            signed.contains("Credential=AKIAIOSFODNN7EXAMPLE/"),
            "the options' pair signs the exchange: {signed}"
        );

        // And every bucket request after it is signed as the role's session,
        // never as the pair that traded for it.
        let bucket_requests: Vec<&Recorded> = recorded
            .iter()
            .filter(|request| !is_exchange(request))
            .collect();
        assert_eq!(
            bucket_requests.len(),
            2,
            "the whole read and the ranged one"
        );
        for request in bucket_requests {
            let signed = authorization(request);
            assert!(signed.contains("Credential=ASIAlake-reader/"), "{signed}");
            assert!(signed.contains("/s3/aws4_request"), "{signed}");
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_lapsed_session_is_traded_again_rather_than_signed_with() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(64));
        // Every session comes back already expired, which is the refresh path
        // without waiting an hour for it.
        store.expire_roles(true);
        let directory = scratch("lapsed-role");
        let role = AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader");
        let handle = file_with("lake/part.bin", assuming(&store, role, directory.clone()));

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        let exchanges = store
            .requests()
            .iter()
            .filter(|request| is_exchange(request))
            .count();
        assert_eq!(
            exchanges, 2,
            "a session past its expiry is not signed with, it is replaced"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_anonymous_client_signs_nothing() {
        let store = store();
        store.allow_anonymous(true);
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let handle = file_with(
            "lake/part.parquet",
            S3Options::default()
                .with_environment(false)
                .with_endpoint(store.endpoint())
                .with_path_style(true)
                .with_anonymous(true),
        );

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("a public object"), b"PAR1");
        let recorded = store.requests();
        assert_eq!(recorded.len(), 1, "one unsigned read");
        assert!(
            !recorded[0]
                .headers
                .iter()
                .any(|(name, _)| name == "authorization"),
            "an anonymous request carries no authorization"
        );
    }

    #[test]
    fn credentials_written_into_a_location_are_used_and_then_never_rendered() {
        let store = store();
        store.require_access_key(Some("AKIAINURL"));
        let mut handle = yggdryl::s3::file_with(
            &format!("s3://AKIAINURL:s3cr3t@{BUCKET}/lake/part.parquet"),
            S3Options::default()
                .with_environment(false)
                .with_endpoint(store.endpoint())
                .with_region("us-east-1")
                .with_path_style(true),
        )
        .expect("a handle");
        handle.write_all_bytes(b"PAR1").expect("a signed write");
        assert_eq!(
            store.get(BUCKET, "lake/part.parquet").as_deref(),
            Some(&b"PAR1"[..]),
            "the store accepted the key the location carried"
        );

        // The keys signed the request, and the handle's own location has none.
        let rendered = handle.url().to_string();
        assert_eq!(rendered, "s3://trades/lake/part.parquet");
        assert!(!rendered.contains("s3cr3t"));
        assert!(!format!("{handle:?}").contains("s3cr3t"));
    }

    #[test]
    fn a_set_the_store_calls_expired_is_obtained_again_and_the_request_signed_once_more() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(64));
        let handle = file(&store, "lake/part.bin");

        // The store knows a set lapsed before the session thought it would:
        // the client forgets it, walks the chain again, and signs the same
        // request once more.
        store.fail_next(400, "ExpiredToken", 1);
        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        let recorded = store.requests();
        assert_eq!(
            recorded.len(),
            2,
            "the refusal and the request signed again: {:?}",
            recorded
                .iter()
                .map(|request| (&request.method, request.status))
                .collect::<Vec<_>>()
        );
        assert_eq!(recorded[0].status, 400, "the first answer is the refusal");
        assert_eq!(recorded[1].status, 200, "the second is the object");
        for request in &recorded {
            let signed = authorization(request);
            assert!(
                signed.contains("Credential=AKIAIOSFODNN7EXAMPLE/"),
                "both attempts are signed: {signed}"
            );
        }

        // Once, not in a loop: a store that still calls the set expired is
        // answered with its own refusal after the second attempt.
        store.fail_next(400, "ExpiredToken", 2);
        store.clear_requests();
        let refused = handle.read_all_bytes().expect_err("a refusal");
        match &refused {
            Error::Remote { status, code, .. } => {
                assert_eq!(*status, 400);
                assert_eq!(code.as_str(), "ExpiredToken");
            }
            other => panic!("expected the store's refusal, got {other:?}"),
        }
        assert_eq!(
            store.request_count(),
            2,
            "one refresh per request, whatever the store keeps saying"
        );
    }
}

#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "xml.rs"]
mod xml;
