//! `rust/src/s3/options.rs`: the knobs an object-store client has, the
//! order their values are found in, the AWS session they carry, and whether a
//! request hashes its payload.

mod protocol {
    use yggdryl::IOBase;
    use yggdryl::aws::{AssumedRole, Session};
    use yggdryl::s3::{Credentials, S3Options};

    use crate::mod_::{file_with, options, store};

    #[test]
    fn options_record_what_was_asked_for_and_the_store_clamps_it() {
        // The options keep the caller's number, because which store will answer is
        // not known until a location is handed over.
        let bounded = S3Options::default()
            .with_part_size(1)
            .with_list_page_size(50_000)
            .with_max_attempts(0);
        assert_eq!(bounded.part_size(), 1);
        assert_eq!(bounded.list_page_size(), 50_000);
        assert_eq!(bounded.max_attempts(), 1, "one attempt is still an attempt");

        // The store clamps it, and the three stores clamp it differently.
        use yggdryl::s3::Provider;
        assert_eq!(Provider::Aws.min_part_size(), 5 * 1024 * 1024);
        assert_eq!(Provider::Google.min_part_size(), 256 * 1024);
        assert_eq!(Provider::Aws.max_list_page(), 5000);
        assert_eq!(Provider::Google.max_list_page(), 1000);

        // A bare host becomes an https endpoint, and a trailing slash is dropped.
        assert_eq!(
            S3Options::default()
                .with_endpoint("s3.example.io/")
                .endpoint(),
            Some("https://s3.example.io")
        );
        // Asking for anonymous access drops any credentials that were set.
        let anonymous = S3Options::default()
            .with_credentials(Credentials::new("a", "b"))
            .with_anonymous(true);
        assert!(anonymous.credentials().is_none());
        assert!(anonymous.anonymous());
    }

    #[test]
    fn default_metadata_rides_every_write_without_displacing_a_content_type() {
        let store = store();
        let mut handle = file_with(
            "lake/part.parquet",
            options(&store).with_default_metadata([
                ("desk", "power"),
                ("Cache-Control", "max-age=31536000"),
                ("content-type", "application/x-nonsense"),
            ]),
        );

        store.clear_requests();
        handle.write_all_bytes(b"PAR1").expect("a write");
        let headers = &store.requests()[0].headers;
        let header = |name: &str| {
            headers
                .iter()
                .find(|(held, _)| held == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(header("x-amz-meta-desk"), Some("power"));
        assert_eq!(header("cache-control"), Some("max-age=31536000"));
        assert_eq!(
            header("content-type"),
            Some("application/vnd.apache.parquet"),
            "a name the write sets for itself is not displaced by a default"
        );
    }

    #[test]
    fn a_session_handed_to_the_options_is_the_one_they_carry() {
        // Nothing handed over is a session that states nothing and consults the
        // process the way the AWS tools do.
        let default = S3Options::default();
        assert!(default.session().reads_environment());
        assert!(default.session().assumed_role().is_none());
        assert!(default.session().sso().is_none());

        let session = Session::new()
            .with_environment(false)
            .with_profile("trading")
            .with_region("eu-west-3")
            .with_assumed_role(AssumedRole::new(
                "arn:aws:iam::123456789012:role/lake-reader",
            ));
        let options = S3Options::default().with_session(session);
        assert_eq!(options.session().profile_name(), "trading");
        assert_eq!(options.session().region().as_deref(), Some("eu-west-3"));
        assert!(!options.session().reads_environment());
        assert_eq!(
            options.session().assumed_role().map(AssumedRole::role_arn),
            Some("arn:aws:iam::123456789012:role/lake-reader")
        );

        // The options' own knobs are theirs: a session with a region leaves
        // the options' explicit region unset, and the reverse.
        assert_eq!(options.region(), None);
        let regional = options.with_region("us-west-2");
        assert_eq!(regional.region(), Some("us-west-2"));
        assert_eq!(regional.session().region().as_deref(), Some("eu-west-3"));
        assert_eq!(
            regional.session().profile_name(),
            "trading",
            "a later knob keeps the session it was handed"
        );
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The payload-signing policy, which only the request that goes out says
    //! was picked, and the digests it picks between.

    use yggdryl::IOBase;
    use yggdryl::internals::aws_sigv4::{EMPTY_PAYLOAD_SHA256, UNSIGNED_PAYLOAD, sha256_hex};
    use yggdryl::internals::s3_options::signs_payload;
    use yggdryl::s3::{AwsOptions, S3Options};

    use crate::mod_::{BUCKET, file, file_with, options, store};
    use crate::server::Recorded;

    /// One header a recorded request carried.
    fn header<'request>(request: &'request Recorded, name: &str) -> Option<&'request str> {
        request
            .headers
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn every_request_carries_a_signature_over_the_headers_it_names() {
        let store = store();
        store.require_access_key(Some("AKIAIOSFODNN7EXAMPLE"));
        let mut handle = file(&store, "lake/part.parquet");
        store.clear_requests();
        handle.write_all_bytes(b"PAR1").expect("a signed write");

        let recorded = store.requests();
        let put = recorded.last().expect("the write");
        let authorization = header(put, "authorization").expect("an authorization header");
        assert!(
            authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"),
            "{authorization}"
        );
        assert!(
            authorization.contains("/us-east-1/s3/aws4_request"),
            "{authorization}"
        );
        assert!(authorization.contains("SignedHeaders="), "{authorization}");
        // The payload is signed by its real hash, so the store can verify it.
        assert_eq!(
            header(put, "x-amz-content-sha256"),
            Some(sha256_hex(b"PAR1").as_str())
        );

        // A read has no body, and says so by the empty payload's own hash.
        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("the object"), b"PAR1");
        let recorded = store.requests();
        let get = recorded.last().expect("the read");
        assert_eq!(
            header(get, "x-amz-content-sha256"),
            Some(EMPTY_PAYLOAD_SHA256)
        );
        assert_eq!(
            EMPTY_PAYLOAD_SHA256,
            sha256_hex(b""),
            "the constant is the digest of nothing"
        );
    }

    #[test]
    fn a_write_signs_its_payload_over_http_and_leaves_it_unsigned_over_tls() {
        let store = store();
        // The fixture endpoint is plain HTTP, where nothing but the hash would
        // establish that the body arrived as it was sent.
        let mut handle = file(&store, "lake/part.bin");
        store.clear_requests();
        handle.write_all_bytes(b"AAPL,187.23").expect("a write");
        let recorded = store.requests();
        let put = recorded.last().expect("the write");
        assert_eq!(
            header(put, "x-amz-content-sha256"),
            Some(sha256_hex(b"AAPL,187.23").as_str()),
        );

        // Asking for the other policy sends the literal S3 accepts instead,
        // which is what an HTTPS endpoint selects on its own: hashing a large
        // value costs more than the rest of the request, and TLS already
        // covers it.
        store.clear_requests();
        let mut unsigned = file_with(
            "lake/unsigned.bin",
            options(&store).with_aws(AwsOptions::default().with_payload_signing(false)),
        );
        unsigned.write_all_bytes(b"AAPL,187.23").expect("a write");
        let recorded = store.requests();
        let put = recorded.last().expect("the write");
        assert_eq!(header(put, "x-amz-content-sha256"), Some(UNSIGNED_PAYLOAD));
        assert_eq!(UNSIGNED_PAYLOAD, "UNSIGNED-PAYLOAD");
        // Either way the store received the bytes it was sent.
        assert_eq!(
            store.get(BUCKET, "lake/unsigned.bin").expect("the object"),
            b"AAPL,187.23"
        );

        // The policy an unset value picks follows the endpoint's scheme, in
        // any case, and an explicit choice wins over it either way.
        let over_tls = S3Options::default().with_endpoint("https://s3.example.io");
        assert!(!signs_payload(&over_tls, "https"));
        assert!(!signs_payload(&over_tls, "HTTPS"));
        assert!(signs_payload(&S3Options::default(), "http"));
        assert!(signs_payload(
            &over_tls.with_aws(AwsOptions::default().with_payload_signing(true)),
            "https"
        ));
        assert!(!signs_payload(
            &S3Options::default().with_aws(AwsOptions::default().with_payload_signing(false)),
            "http"
        ));
    }
}
