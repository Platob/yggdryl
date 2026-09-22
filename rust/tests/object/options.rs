//! `rust/src/object/options.rs`: the knobs an object-store client has, and the
//! order their values are found in.

mod protocol {
    use yggdryl::IOBase;
    use yggdryl::object::{Credentials, S3Options};

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
        use yggdryl::object::Provider;
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
}
