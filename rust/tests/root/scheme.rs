//! `rust/src/scheme.rs`.

mod vocabulary {
    use std::collections::{BTreeSet, HashSet};

    use yggdryl::Scheme;

    #[test]
    fn every_known_scheme_parses_to_its_static_value() {
        for (source, expected) in [
            ("HTTP", Scheme::HTTP),
            ("HTTPS", Scheme::HTTPS),
            ("FILE", Scheme::FILE),
            ("URN", Scheme::URN),
            ("POSTGRES", Scheme::POSTGRES),
            ("POSTGRESQL", Scheme::POSTGRESQL),
            ("MYSQL", Scheme::MYSQL),
            ("ARROW", Scheme::ARROW),
            ("SQL", Scheme::SQL),
            ("GLUE", Scheme::GLUE),
            ("ICEBERG", Scheme::ICEBERG),
            ("FIX", Scheme::FIX),
            ("FIELD", Scheme::FIELD),
            ("DIGEST", Scheme::DIGEST),
            ("IDENTITY", Scheme::IDENTITY),
            ("PARTITION", Scheme::PARTITION),
            ("S3", Scheme::S3),
            ("S3A", Scheme::S3A),
            ("S3N", Scheme::S3N),
            ("GS", Scheme::GS),
            ("AZ", Scheme::AZ),
            ("SPARK", Scheme::SPARK),
            ("POLARS", Scheme::POLARS),
            ("PANDAS", Scheme::PANDAS),
            ("PYTHON", Scheme::PYTHON),
        ] {
            let parsed = Scheme::from_str(source).unwrap();
            assert_eq!(parsed, expected);
            assert!(parsed.is_known());
            assert_eq!(parsed.as_str(), source.to_ascii_lowercase());
        }
    }

    #[test]
    fn the_three_s3_spellings_are_one_protocol_and_stay_distinct_values() {
        // A backend asks `is_s3`, never which spelling it was handed: the Hadoop
        // names differ only in the connector that once read them.
        for scheme in [Scheme::S3, Scheme::S3A, Scheme::S3N] {
            assert!(scheme.is_s3(), "{scheme}");
            assert!(scheme.is_storage(), "{scheme}");
        }
        for scheme in [Scheme::FILE, Scheme::HTTPS, Scheme::GS, Scheme::AZ] {
            assert!(!scheme.is_s3(), "{scheme}");
        }
        assert!(!Scheme::from_str("s3x").unwrap().is_s3());

        // One protocol is not one value: a location reports the spelling it was
        // written with, so equality and ordering keep the three apart.
        assert_ne!(Scheme::S3, Scheme::S3A);
        assert_ne!(Scheme::S3A, Scheme::S3N);
        assert_eq!(Scheme::S3A.as_str(), "s3a");
        assert_eq!(Scheme::S3N.as_str(), "s3n");
    }

    #[test]
    fn known_and_custom_schemes_share_canonical_value_semantics() {
        let known = Scheme::from_str("POSTGRES").unwrap();
        let custom = Scheme::from_str("Acme+Wire").unwrap();

        assert_eq!(known, Scheme::POSTGRES);
        assert!(known.is_known());
        assert_eq!(custom.as_str(), "acme+wire");
        assert!(!custom.is_known());
        let displayed = custom.to_string();
        assert_eq!(displayed, "acme+wire");
        assert_eq!(Scheme::from_str(&displayed).unwrap(), custom);
        assert_eq!(
            serde_json::from_str::<Scheme>(r#""S3""#).unwrap(),
            Scheme::S3
        );
        assert_eq!(
            serde_json::from_str::<Scheme>(&serde_json::to_string(&custom).unwrap()).unwrap(),
            custom
        );
        assert_eq!(
            custom.stable_hash(),
            Scheme::from_str("ACME+WIRE").unwrap().stable_hash()
        );

        assert!(BTreeSet::from([known.clone()]).contains(&known));
        assert!(HashSet::from([custom.clone()]).contains(&custom));
    }

    #[test]
    fn scheme_validation_reports_the_original_byte() {
        for invalid in ["", "1http", "http_compat", "http://"] {
            assert!(Scheme::from_str(invalid).is_err(), "{invalid:?}");
        }

        let error = Scheme::from_str("http_").unwrap_err();
        assert!(matches!(
            error,
            yggdryl::Error::Parse {
                target: "scheme",
                position: 4,
                ..
            }
        ));
    }
}
