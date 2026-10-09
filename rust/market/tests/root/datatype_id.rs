//! `rust/src/datatype_id.rs` over the kinds `yggdryl-market` claims: each
//! takes its byte in the enum family's range, after `State`. Where the
//! listing names it among the core's codes is `cli/tests/market_register.rs`'s;
//! the core's own identifiers are `rust/tests/root/datatype_id.rs`'s.

use yggdryl::{DataTypeId, DataTypeKind};
use yggdryl_market::{MarketDataKind, MarketDataType, Side, TimeInForce};

#[test]
fn the_market_kinds_sit_in_the_enum_familys_range_after_state() {
    crate::install::installed();
    let kinds = [
        (MarketDataKind::ID, 0xc2),
        (Side::ID, 0xc3),
        (MarketDataType::ID, 0xc4),
        (TimeInForce::ID, 0xc5),
    ];
    for (id, byte) in kinds {
        assert_eq!(id.as_u8(), byte, "{id}");
        assert_eq!(id.kind(), DataTypeKind::Enum, "{id}");
        for other in DataTypeKind::ALL {
            assert_eq!(
                other.contains(id),
                other == DataTypeKind::Enum,
                "{id} in {other}"
            );
        }
    }
    // Claimed, they follow the core's enum leaf in the listing.
    let all = DataTypeId::all();
    let state = all.iter().position(|id| *id == DataTypeId::State).unwrap();
    assert_eq!(
        &all[state + 1..state + 5],
        &kinds.map(|(id, _)| id),
        "the listing after state"
    );
}
