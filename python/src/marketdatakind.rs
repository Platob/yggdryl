//! What kind of market data an element is: the member table
//! `yggdryl.MarketDataKind` is built from.

use pyo3::prelude::*;
use yggdryl::MarketDataKind;

/// Every member of the core enum, in code order: its stored name - the
/// four-letter `MsgCat` code - the code a column stores, and what it means.
///
/// The Python enum is built from this once at import, so the binding lists
/// no member of its own.
#[pyfunction]
pub(crate) fn marketdatakind_members() -> Vec<(&'static str, u8, &'static str)> {
    MarketDataKind::ALL
        .iter()
        .map(|kind| (kind.as_str(), kind.code(), kind.description()))
        .collect()
}

/// The code of the kind one spelling names - the four-letter code in any
/// case, or the member's own word folded - or `None` where none does.
#[pyfunction]
pub(crate) fn marketdatakind_from_spelling(spelling: &str) -> Option<u8> {
    MarketDataKind::from_spelling(spelling).map(MarketDataKind::code)
}
