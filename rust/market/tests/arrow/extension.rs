//! `rust/market/src/side.rs`'s Arrow marker: a kind this crate claims rides
//! its extension name, and an Arrow field typed by its marker imports as the
//! kind's datatype - the core's `arrow::extension` rule, which
//! `rust/tests/arrow/extension.rs` pins over the core's own markers.

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::{CcyType, DataTypeId, Field};
use yggdryl_market::{Side, SideType};

#[test]
fn an_arrow_field_typed_by_a_market_marker_imports_as_its_kind() {
    crate::install::installed();
    let field = ArrowField::new("x", ArrowDataType::UInt8, true).with_extension_type(SideType);
    let imported = Field::from_arrow_field(&field).expect("the field imports");
    assert_eq!(imported.dtype(), &Side::dtype(), "{field:?}");
    // Another name over the kind's field is refused.
    let side = Field::new("x", Side::dtype(), true)
        .into_arrow_field()
        .expect("an Arrow field");
    assert!(side.try_extension_type::<CcyType>().is_err());
}

#[test]
fn the_kinds_names_join_the_listing_once_claimed() {
    crate::install::installed();
    let names: Vec<&str> = DataTypeId::arrow_extension_names().collect();
    let mut distinct = names.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), names.len(), "{names:?}");
    // The core's thirty-one and the four the market claims.
    assert_eq!(names.len(), 35, "{names:?}");
    for name in [
        "yggdryl.marketdatakind",
        "yggdryl.side",
        "yggdryl.marketdatatype",
        "yggdryl.timeinforce",
    ] {
        assert!(names.contains(&name), "{name}: {names:?}");
    }
}
