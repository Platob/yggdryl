//! The type of its kind a market element is: the member table
//! `MarketDataType` is built from, and the FIX reading of its wire values.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl::MarketDataType;

use crate::napi_error;

/// One member of the core's market data type enum: its stored name, the code
/// a `marketdatatype` column stores, and what it means.
#[napi(object)]
pub struct MarketDataTypeMember {
    pub name: String,
    pub code: i32,
    pub description: String,
}

/// The FIX field and wire value one member stands for.
#[napi(object)]
pub struct MarketDataTypeFixCode {
    pub tag: i32,
    pub wire: String,
}

/// Every member of the core enum, in code order, so the JavaScript
/// `MarketDataType` is built from the core's own table and never lists a
/// member itself.
#[napi(js_name = "_marketDataTypeMembersNative", skip_typescript)]
pub fn market_data_type_members_native() -> Vec<MarketDataTypeMember> {
    MarketDataType::ALL
        .iter()
        .map(|member| MarketDataTypeMember {
            name: member.as_str().to_owned(),
            code: i32::from(member.code()),
            description: member.description().to_owned(),
        })
        .collect()
}

/// The member one FIX field's wire value types an element as, as its stored
/// name, or `null` for a field that types nothing.
#[napi(js_name = "marketDataTypeFromFix")]
pub fn market_data_type_from_fix(tag: i32, wire: String) -> Option<String> {
    MarketDataType::from_fix(tag, &wire).map(|member| member.as_str().to_owned())
}

/// The FIX field and wire value the member `name` stands for, or `null` for
/// `UNKN` and a catch-all; throws on a name that is no member.
#[napi(js_name = "marketDataTypeFixCode")]
pub fn market_data_type_fix_code(name: String) -> Result<Option<MarketDataTypeFixCode>> {
    let member = MarketDataType::from_spelling(&name)
        .ok_or_else(|| napi_error(format!("{name:?} is no marketdatatype member")))?;
    Ok(member.fix_code().map(|(tag, wire)| MarketDataTypeFixCode {
        tag,
        wire: wire.to_owned(),
    }))
}
