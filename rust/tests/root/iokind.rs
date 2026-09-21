//! `rust/src/iokind.rs`.

mod enums {
    use std::collections::BTreeSet;

    use yggdryl::{Error, IOKind};

    #[test]
    fn every_io_kind_names_itself_canonically_and_parses_back() {
        for (kind, canonical) in [
            (IOKind::Memory, "memory"),
            (IOKind::File, "file"),
            (IOKind::Directory, "directory"),
            (IOKind::Table, "table"),
            (IOKind::Namespace, "namespace"),
            (IOKind::Catalog, "catalog"),
            (IOKind::Unknown, "unknown"),
        ] {
            assert_eq!(kind.as_str(), canonical);
            assert_eq!(kind.to_string(), canonical);
            assert_eq!(IOKind::from_str(canonical).unwrap(), kind);
            // The parser is ASCII case-insensitive and trims, like every other.
            assert_eq!(IOKind::from_str(&canonical.to_uppercase()).unwrap(), kind);
            assert_eq!(IOKind::from_str(&format!("  {canonical}\t")).unwrap(), kind);

            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{canonical}\"")
            );
            assert_eq!(
                serde_json::from_str::<IOKind>(&format!("\"{canonical}\"")).unwrap(),
                kind
            );
        }

        // `ALL` is the vocabulary, in canonical order and without repeats.
        assert_eq!(
            IOKind::ALL,
            [
                IOKind::Memory,
                IOKind::File,
                IOKind::Directory,
                IOKind::Table,
                IOKind::Namespace,
                IOKind::Catalog,
                IOKind::Unknown,
            ]
        );
        assert_eq!(BTreeSet::from(IOKind::ALL).len(), IOKind::ALL.len());
        assert_eq!(IOKind::default(), IOKind::File);
    }

    #[test]
    fn io_kind_categories_stay_disjoint_across_the_whole_vocabulary() {
        for kind in IOKind::ALL {
            // A resource holds bytes or holds others, never both.
            assert!(!(kind.is_leaf() && kind.is_container()), "{kind}");
            assert_eq!(kind.is_known(), kind != IOKind::Unknown, "{kind}");
        }

        for container in [
            IOKind::Directory,
            IOKind::Table,
            IOKind::Namespace,
            IOKind::Catalog,
        ] {
            assert!(container.is_container(), "{container}");
            assert!(!container.is_leaf(), "{container}");
        }

        for leaf in [IOKind::Memory, IOKind::File] {
            assert!(leaf.is_leaf(), "{leaf}");
            assert!(!leaf.is_container(), "{leaf}");
        }

        // The undecided answer is neither, which is what makes it honest.
        assert!(!IOKind::Unknown.is_leaf());
        assert!(!IOKind::Unknown.is_container());
        assert!(!IOKind::Unknown.is_known());
    }

    #[test]
    fn io_kind_parsing_names_the_whole_accepted_vocabulary() {
        let error = IOKind::from_str("warehouse").unwrap_err();
        let message = error.to_string();
        for name in [
            "memory",
            "file",
            "directory",
            "table",
            "namespace",
            "catalog",
            "unknown",
        ] {
            assert!(message.contains(name), "{message}");
        }
        assert!(message.contains("\"warehouse\""), "{message}");
        assert!(matches!(
            error,
            Error::Parse {
                target: "io kind",
                position: 0,
                ..
            }
        ));
    }
}
