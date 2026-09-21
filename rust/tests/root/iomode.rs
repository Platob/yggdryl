//! `rust/src/iomode.rs`.

mod enums {

    use yggdryl::{Error, IOMode};

    #[test]
    fn every_io_mode_has_one_required_canonical_spelling() {
        for (mode, canonical) in [
            (IOMode::Overwrite, "overwrite"),
            (IOMode::Append, "append"),
            (IOMode::Merge, "merge"),
            (IOMode::ReadOnly, "readonly"),
            (IOMode::Random, "random"),
        ] {
            assert_eq!(mode.as_str(), canonical);
            assert_eq!(mode.as_ref(), canonical);
            assert_eq!(mode.to_string(), canonical);
            assert_eq!(IOMode::from_str(canonical).unwrap(), mode);
            assert_eq!(IOMode::from_str(&canonical.to_uppercase()).unwrap(), mode);
            assert_eq!(IOMode::from_str(&format!("  {canonical}\t")).unwrap(), mode);
            assert_eq!(
                serde_json::to_string(&mode).unwrap(),
                format!("\"{canonical}\"")
            );
            assert_eq!(
                serde_json::from_str::<IOMode>(&format!("\"{canonical}\"")).unwrap(),
                mode
            );
        }
        assert_eq!(IOMode::ALL.len(), 5);
        assert_eq!(IOMode::WRITE.len(), 3);

        let error = IOMode::from_str("upsert").unwrap_err();
        assert!(matches!(error, Error::Parse { target: "mode", .. }));
        assert!(
            error
                .to_string()
                .contains("overwrite, append, merge, readonly, random")
        );
        assert!(serde_json::from_str::<IOMode>("\"write\"").is_err());
    }
}
