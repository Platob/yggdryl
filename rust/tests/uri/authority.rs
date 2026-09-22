//! `rust/src/uri/authority.rs`: the authority a URI carries - its host forms,
//! its port, and the delimiters an escape must not move.

mod encoding {

    use yggdryl::Uri;

    /// RFC 6874 s2: `IPv6addrz = IPv6address "%25" ZoneID`, and the zone is
    /// `1*( unreserved / pct-encoded )`.
    ///
    /// The `%25` is the zone marker, not an escape to decode, so it survives
    /// normalization untouched. WHATWG refuses zone identifiers outright; this
    /// crate accepts the RFC form and refuses everything around it.
    #[test]
    fn ipv6_zone_identifiers_follow_the_percent_25_rule() {
        let uri = Uri::from_str("https://[fe80::1%25eth0]:8443/x").unwrap();
        assert_eq!(uri.authority().as_str(), "[fe80::1%25eth0]:8443");
        assert_eq!(uri.authority().host(), "fe80::1%25eth0");
        assert_eq!(uri.authority().port(), Some(8443));
        assert_eq!(uri.to_string(), "https://[fe80::1%25eth0]:8443/x");

        // A zone byte outside the unreserved set is spelled with an escape.
        assert!(Uri::from_str("https://[fe80::1%25en%201]/").is_ok());
        // A bare `%` is not the marker, and an empty zone is not a zone.
        assert!(Uri::from_str("https://[fe80::1%eth0]/").is_err());
        assert!(Uri::from_str("https://[fe80::1%25]/").is_err());
        assert!(Uri::from_str("https://[fe80::1%25 ]/").is_err());
        // A zone belongs to an IPv6 literal, not to IPvFuture or a registered name.
        assert!(Uri::from_str("https://[v1.fe80::a%25en1]/").is_err());
        assert_eq!(
            Uri::from_str("https://192.0.2.1%25eth0/")
                .unwrap()
                .authority()
                .host(),
            "192.0.2.1%25eth0"
        );
    }

    /// The user-information delimiter is the last `@`, so an escaped one is data.
    ///
    /// `http://a%40b@example.com/` names the user `a%40b` on host `example.com`;
    /// reading the first `@` instead hands the URI to the wrong host.
    #[test]
    fn escaped_authority_delimiters_do_not_move_the_real_ones() {
        let uri = Uri::from_str("https://a%40b:p%40ss@example.test:8443/x").unwrap();

        assert_eq!(uri.user(), Some("a%40b"));
        assert_eq!(uri.password(), Some("p%40ss"));
        assert_eq!(uri.hostname(), Some("example.test"));
        assert_eq!(uri.authority().port(), Some(8443));

        // An escaped colon stays inside the user name.
        let escaped_colon = Uri::from_str("https://user%3Aname:pass@example.test/").unwrap();
        assert_eq!(escaped_colon.user(), Some("user%3Aname"));
        assert_eq!(escaped_colon.password(), Some("pass"));

        // An escaped `%` in the user name is one escape, not a bare percent.
        let percent = Uri::from_str("https://%25DOMAIN:secret@example.test/").unwrap();
        assert_eq!(percent.user(), Some("%25DOMAIN"));
        assert_eq!(
            percent.to_string(),
            "https://%25DOMAIN:secret@example.test/"
        );

        // A second unescaped `@` is ambiguous and refused.
        assert!(Uri::from_str("https://a@b@example.test/").is_err());
    }
}

mod value {
    use yggdryl::Authority;

    #[test]
    fn authority_ports_are_explicit_valid_u16_values() {
        assert_eq!(
            Authority::from_str("minio:9000").unwrap().port(),
            Some(9000)
        );
        assert_eq!(
            Authority::from_str("[::1]:9000").unwrap().port(),
            Some(9000)
        );
        for invalid in ["minio:", "minio:65536", "[::1]:", "[::1]:99999"] {
            assert!(Authority::from_str(invalid).is_err(), "{invalid}");
        }
    }
}

mod store {

    use yggdryl::{Arn, Uri, Url};

    /// A table bucket is one position in a location, so the store accessors
    /// read an `s3tables:` URL the way they read an `s3:` one.
    #[test]
    fn an_s3_tables_url_reads_as_the_container_and_the_name_below_it() {
        let table = Uri::from_str("s3tables://lake/t-a1").unwrap();
        assert_eq!(table.bucket(), Some("lake"));
        assert_eq!(table.key(), Some("t-a1"));
        assert_eq!(table.hostname(), None);
        assert_eq!(table.store_endpoint(), None);
        assert!(!table.is_virtual_hosted());
        assert_eq!(table.region(), None);
        assert_eq!(table.account(), None);

        // The container alone is the whole location, with nothing below it.
        let bucket = Uri::from_str("s3tables://lake").unwrap();
        assert_eq!(bucket.bucket(), Some("lake"));
        assert_eq!(bucket.key(), Some(""));

        // It is a URL, so a reader can hold it; it is not an object store, so
        // no byte backend is selected by it.
        assert!(Url::from_str("s3tables://lake/t-a1").is_ok());
        assert!(!table.scheme().is_object_store());
        assert!(!table.scheme().is_storage());

        // A URL and the ARN it came from name the same container.
        let arn = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")
            .unwrap();
        assert_eq!(Uri::from(arn.locator().unwrap()), table);
    }
}
