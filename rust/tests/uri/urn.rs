//! `rust/src/uri/urn.rs`: uniform resource names and the escapes their
//! namespace-specific strings keep.

mod encoding {

    use yggdryl::Urn;

    /// RFC 8141 s2 and s3.1: the NSS carries escapes, and they are never decoded
    /// for comparison, so `%2C` and `,` stay two different URNs.
    #[test]
    fn urn_namespace_specific_strings_keep_their_escapes() {
        let urn = Urn::from_str("URN:EXAMPLE:a123%2cz456").unwrap();

        // Scheme and namespace lowercase; the NSS keeps its case and its escapes,
        // whose hex digits uppercase.
        assert_eq!(urn.to_string(), "urn:example:a123%2Cz456");
        assert_eq!(urn.namespace(), "example");
        assert_eq!(urn.namespace_specific(), "a123%2Cz456");
        assert_ne!(urn, Urn::from_str("urn:example:a123,z456").unwrap());
        assert_ne!(urn, Urn::from_str("urn:example:A123%2Cz456").unwrap());

        // The NSS may hold `/`, and `?`, `#`, `[`, `]` only as escapes.
        assert_eq!(
            Urn::from_str("urn:example:1/406/47452/2")
                .unwrap()
                .file_name(),
            Some("2")
        );
        let escaped = Urn::from_str("urn:example:a%5Bb%5D%3Fc").unwrap();
        assert_eq!(escaped.namespace_specific(), "a%5Bb%5D%3Fc");
        assert_eq!(escaped.path_text(true).unwrap(), "example:a[b]?c");

        // A `?` that opens neither an r- nor a q-component is a syntax error.
        assert!(Urn::from_str("urn:example:foo?bar").is_err());
        let resolved = Urn::from_str("urn:example:foo?+CCResolve:cc=uk?=q1#f1").unwrap();
        assert_eq!(
            resolved.query(false).unwrap().as_deref(),
            Some("+CCResolve:cc=uk?=q1")
        );
        assert_eq!(resolved.fragment(false).unwrap().as_deref(), Some("f1"));
    }
}
