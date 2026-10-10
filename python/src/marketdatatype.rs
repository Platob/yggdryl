//! What type of its kind a market element is: the member table
//! `yggdryl.MarketDataType` is built from.

use pyo3::prelude::*;
use yggdryl_market::{MarketDataKind, MarketDataType};

/// Every member of the core enum, in code order: its stored name, the code a
/// column stores, and what it means.
///
/// The Python enum is built from this once at import, so the binding lists
/// no member of its own.
#[pyfunction]
pub(crate) fn marketdatatype_members() -> Vec<(&'static str, u16, &'static str)> {
    MarketDataType::ALL
        .iter()
        .map(|member| (member.as_str(), member.code(), member.description()))
        .collect()
}

/// The code of the member one spelling names - the stored name in any case,
/// or the FIX specification's own name folded - or `None` where none does.
#[pyfunction]
pub(crate) fn marketdatatype_from_spelling(spelling: &str) -> Option<u16> {
    MarketDataType::from_spelling(spelling).map(MarketDataType::code)
}

/// The code of the member one FIX field's wire value types an element as, or
/// `None` for a field that types nothing.
#[pyfunction]
pub(crate) fn marketdatatype_from_fix(tag: i32, wire: &str) -> Option<u16> {
    MarketDataType::from_fix(tag, wire).map(MarketDataType::code)
}

/// The FIX field and wire value the member with this code stands for, or
/// `None` for `UKNW`, a catch-all or a code no member has.
#[pyfunction]
pub(crate) fn marketdatatype_fix_code(code: u16) -> Option<(i32, &'static str)> {
    MarketDataType::from_code(code).and_then(MarketDataType::fix_code)
}

/// The FIX fields that type an element of the `MarketDataKind` with this
/// code, the one its kind names first; empty for a code no kind has.
#[pyfunction]
pub(crate) fn marketdatatype_fix_tags(kind: u8) -> Vec<i32> {
    MarketDataKind::from_code(kind)
        .map(|kind| MarketDataType::fix_tags(kind).to_vec())
        .unwrap_or_default()
}

/// The FIX fields that type a message of type `msgtype` filed under the
/// `MarketDataKind` with this code: the message type's own rule where the
/// core states one, else its kind's; empty for a code no kind has.
#[pyfunction]
pub(crate) fn marketdatatype_fix_tags_of(msgtype: &str, kind: u8) -> Vec<i32> {
    MarketDataKind::from_code(kind)
        .map(|kind| MarketDataType::fix_tags_of(msgtype, kind).to_vec())
        .unwrap_or_default()
}
