//! `rust/src/uri/path.rs`: URI paths and what normalizing one keeps.

mod encoding {

    use yggdryl::UriPath;

    /// Resolving `.` and `..` never turns a relative path into an absolute one.
    ///
    /// A relative path that resolves to no name at all is still relative: the
    /// trailing separator says the location is a container, not that it is the
    /// root.
    #[test]
    fn normalizing_a_relative_path_keeps_it_relative() {
        for (input, expected) in [
            ("./", ""),
            (".///", ""),
            ("a/../", ""),
            ("./a/", "a/"),
            ("a/./b", "a/b"),
            ("/./", "/"),
            ("/a/../", "/"),
            ("/", "/"),
            ("a/b/", "a/b/"),
        ] {
            let normalized = UriPath::from_str(input).unwrap().normalize().unwrap();
            assert_eq!(normalized.as_str(), expected, "{input:?}");
            assert_eq!(
                normalized.is_absolute(),
                UriPath::from_str(input).unwrap().is_absolute(),
                "{input:?} changed rootedness"
            );
        }
    }
}
