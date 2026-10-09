//! `rust/src/datatype.rs` over the kinds `yggdryl-market` claims: each
//! keeps its extension identity across the C Data Interface and hashes as
//! the shape its reserved numbers state. Where they sort among the core's
//! datatypes is `cli/tests/market_register.rs`'s; the core's own datatypes
//! are `rust/tests/root/datatype.rs`'s.

mod arrow {
    use arrow_schema::Field as ArrowField;
    use yggdryl::{DataType, Field};
    use yggdryl_market::{Side, TimeInForce};

    #[test]
    fn a_market_kind_keeps_its_identity_across_the_c_interface() {
        crate::install::installed();
        for dtype in [
            Side::dtype(),
            TimeInForce::dtype(),
            DataType::from_str("marketdatakind").unwrap(),
            DataType::from_str("marketdatatype").unwrap(),
        ] {
            let ffi = dtype.clone().into_arrow_datatype_ffi().unwrap();
            let arrow = ArrowField::try_from(&ffi)
                .unwrap_or_else(|error| panic!("{dtype} did not project a C schema: {error}"));
            let name = arrow
                .metadata()
                .get("ARROW:extension:name")
                .unwrap_or_else(|| {
                    panic!("{dtype} crossed the C Data Interface without its identity")
                });
            assert!(name.starts_with("yggdryl."), "{dtype}: {name:?}");
            assert_eq!(
                Field::from_arrow_field(&arrow).unwrap().dtype(),
                &dtype,
                "{dtype} did not read back as itself"
            );
        }
    }
}

/// A market kind hashes as the shape its reserved numbers state, so no
/// stored digest over one moves.
#[cfg(feature = "internals")]
mod structural_hash {
    use yggdryl::DataType;
    use yggdryl::internals::hashing_stable::stable_hash_of;

    const PINNED: [(&str, u64); 2] = [
        ("side", 0x52a98a94618c687b),
        ("timeinforce", 0x5e9bed84925ef4fe),
    ];

    #[test]
    fn every_market_kind_hashes_as_it_did_before_the_leaves_split() {
        crate::install::installed();
        for (name, pinned) in PINNED {
            let dtype = DataType::from_str(name).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(dtype.to_string(), name, "{name} spells itself");
            assert_eq!(stable_hash_of(&dtype), pinned, "{name}");
        }
    }
}
