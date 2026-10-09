//! `rust/src/compatibility.rs` over the enum leaves `yggdryl-market`
//! claims: every foreign engine reads a claimed kind as the `int32` code of
//! its members, as it reads the core's `state`, and Arrow keeps the kind.
//! The rule over the core's own leaves is `rust/tests/root/compatibility.rs`'s.

use yggdryl::{DataType, DataTypeId, Scheme};
use yggdryl_market::{MarketDataKind, MarketDataType, Side, TimeInForce};

#[test]
fn every_foreign_engine_reads_a_claimed_kind_as_its_int32_code() {
    crate::install::installed();
    for dtype in [
        MarketDataKind::dtype(),
        Side::dtype(),
        MarketDataType::dtype(),
        TimeInForce::dtype(),
    ] {
        assert!(dtype.is_enum(), "{dtype}");
        for scheme in [
            &Scheme::SPARK,
            &Scheme::POLARS,
            &Scheme::PANDAS,
            &Scheme::ICEBERG,
        ] {
            assert_eq!(
                dtype.clone().into_scheme_compat(scheme).unwrap(),
                DataType::Int32,
                "{dtype} under {scheme}"
            );
        }
        assert_eq!(
            dtype.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
            dtype
        );
    }
    // Claimed, the four join the listing every engine is asked over: the
    // core's `state` and the four kinds are the enum leaves it holds.
    let enums = DataTypeId::all()
        .into_iter()
        .filter_map(|id| DataType::from_str(id.as_str()).ok())
        .filter(DataType::is_enum)
        .count();
    assert_eq!(enums, 5);
}
