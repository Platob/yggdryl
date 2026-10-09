//! How long an order stands: the member table `yggdryl.TimeInForce` is built
//! from.

use pyo3::prelude::*;
use yggdryl_market::TimeInForce;

/// Every member of the core enum, in code order: its stored name, the code a
/// column stores, what it means, and its `TimeInForce(59)` wire value -
/// `None` for `UKNW` and `OTHER`.
///
/// The Python enum is built from this once at import, so the binding lists
/// no member of its own.
#[pyfunction]
#[allow(clippy::type_complexity)]
pub(crate) fn timeinforce_members() -> Vec<(&'static str, u8, &'static str, Option<&'static str>)> {
    TimeInForce::ALL
        .iter()
        .map(|member| {
            (
                member.as_str(),
                member.code(),
                member.description(),
                member.fix_code(),
            )
        })
        .collect()
}

/// The code of the member one spelling names - the stored name in any case,
/// the FIX specification's own name folded, or the `TimeInForce(59)` wire
/// value - or `None` where none does.
#[pyfunction]
pub(crate) fn timeinforce_from_spelling(spelling: &str) -> Option<u8> {
    TimeInForce::from_spelling(spelling).map(TimeInForce::code)
}

/// The code of the member one `TimeInForce(59)` wire value stands for,
/// `OTHER` for a value no member names.
#[pyfunction]
pub(crate) fn timeinforce_from_fix(wire: &str) -> u8 {
    TimeInForce::from_fix(wire).code()
}
