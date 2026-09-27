//! What kind of market data an element is: the member table
//! `MarketDataKind` is built from.

use napi_derive::napi;
use yggdryl::MarketDataKind;

/// One member of the core's market data kind enum - FIX's `MsgCat` code set:
/// its stored name, the code a `marketdatakind` column stores, and what it
/// means.
#[napi(object)]
pub struct MarketDataKindMember {
    pub name: String,
    pub code: i32,
    pub description: String,
}

/// Every member of the core enum, in code order, so the JavaScript
/// `MarketDataKind` is built from the core's own table and never lists a
/// member itself.
#[napi(js_name = "_marketDataKindMembersNative", skip_typescript)]
pub fn market_data_kind_members_native() -> Vec<MarketDataKindMember> {
    MarketDataKind::ALL
        .iter()
        .map(|kind| MarketDataKindMember {
            name: kind.as_str().to_owned(),
            code: kind.code(),
            description: kind.description().to_owned(),
        })
        .collect()
}
