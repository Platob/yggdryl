//! FIX's side of a trade: the member table `yggdryl.Side` is built from.

use pyo3::prelude::*;
use yggdryl::Side;

/// Every member of the core enum, in code order: its four-letter code, the
/// code a column stores, what it means, its `Side(54)` wire character - `None` for
/// `UKNW` and `BOTH` - and whether it takes the bid and whether the ask.
///
/// The Python enum is built from this once at import, so the binding lists
/// no member and decides no side of its own.
#[pyfunction]
#[allow(clippy::type_complexity)]
pub(crate) fn side_members() -> Vec<(&'static str, u8, &'static str, Option<char>, [bool; 2])> {
    Side::ALL
        .iter()
        .map(|side| {
            (
                side.as_str(),
                side.code(),
                side.description(),
                side.fix_code(),
                [side.is_bid(), side.is_ask()],
            )
        })
        .collect()
}

/// The code of the side one spelling names - a four-letter code, a FIX wire
/// code or the specification's name - or `None` where none does.
#[pyfunction]
pub(crate) fn side_from_spelling(spelling: &str) -> Option<u8> {
    Side::from_spelling(spelling).map(Side::code)
}
