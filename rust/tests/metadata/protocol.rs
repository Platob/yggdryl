//! `rust/src/metadata/protocol.rs`: metadata value and protocol-view tests.

mod metadata {
    use std::hash::{Hash, Hasher};
    use yggdryl::Metadata;
    use yggdryl::Scheme;

    #[test]
    fn protocol_iteration_is_exact_sorted_double_ended_and_cursor_compatible() {
        let metadata = Metadata::from_entries([
            ("postgre", "before"),
            ("postgres", "plain"),
            ("postgres-prefix", "before-colon"),
            ("POSTGRES:alpha", "a"),
            ("POSTGRES:middle", "m"),
            ("POSTGRES:omega", "z"),
            ("postgres0", "before-colon"),
            ("POSTGRESQL:alpha", "different-scheme"),
            ("z:last", "after"),
        ])
        .unwrap();

        let mut properties = metadata.property_iter(&Scheme::POSTGRES);
        assert_eq!(properties.next(), Some(("alpha", "a")));
        assert_eq!(properties.next_back(), Some(("omega", "z")));
        assert_eq!(properties.next(), Some(("middle", "m")));
        assert_eq!(properties.next(), None);
        assert_eq!(properties.next_back(), None);

        assert_eq!(
            metadata.next_property_entry(&Scheme::POSTGRES, Some("alpha")),
            Some(("middle", "m"))
        );
        assert_eq!(
            metadata.next_property_entry(&Scheme::POSTGRES, Some("omega")),
            None
        );
        assert_eq!(metadata.get_property(&Scheme::POSTGRES, "alpha"), Some("a"));
        assert_eq!(metadata.get_property(&Scheme::POSTGRES, "missing"), None);
    }

    #[test]
    fn protocol_cursor_visits_every_wide_property_once() {
        let metadata = Metadata::from_entries(
            (0..1_024).map(|index| (format!("POSTGRES:key-{index:04}"), index.to_string())),
        )
        .unwrap();
        let mut after = None;
        let mut count = 0;
        while let Some((name, value)) = metadata.next_property_entry(&Scheme::POSTGRES, after) {
            assert_eq!(name, format!("key-{count:04}"));
            assert_eq!(value, count.to_string());
            after = Some(name);
            count += 1;
        }
        assert_eq!(count, 1_024);
    }

    #[test]
    fn protocol_views_order_and_hash_the_properties_they_expose() {
        fn hash(value: &impl Hash) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        }

        let first = Metadata::from_entries([("POSTGRES:a", "1")]).unwrap();
        let equal =
            Metadata::from_entries([("POSTGRES:a", "1"), ("s3:bucket", "ignored")]).unwrap();
        let later = Metadata::from_entries([("POSTGRES:b", "1")]).unwrap();
        assert_eq!(first.as_postgres(), equal.as_postgres());
        assert_eq!(hash(&first.as_postgres()), hash(&equal.as_postgres()));
        assert!(first.as_postgres() < later.as_postgres());
    }
}
