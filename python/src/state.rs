//! What state one thing is in: the member table `yggdryl.State` is built from.

use pyo3::prelude::*;
use yggdryl::State;

/// Every member of the core enum, in code order: its stored name, the code a
/// column stores, what it means, and the band facts the core answers for it -
/// its rank, and whether it is pending, live, done, cancelled or failed, and
/// whether it reports an execution.
///
/// The Python enum is built from this once at import, so the binding lists no
/// member and decides no band of its own.
#[pyfunction]
#[allow(clippy::type_complexity)]
pub(crate) fn state_members() -> Vec<(&'static str, u16, &'static str, u8, [bool; 6])> {
    State::ALL
        .iter()
        .map(|state| {
            (
                state.as_str(),
                state.code(),
                state.description(),
                state.rank(),
                [
                    state.is_pending(),
                    state.is_live(),
                    state.is_done(),
                    state.is_cancelled(),
                    state.is_failed(),
                    state.is_execution(),
                ],
            )
        })
        .collect()
}

/// The code of the state one spelling names - a stored name, a FIX wire
/// code, the specification's name, a scheduler's word or a bridge's short
/// name - or `None` where none does.
#[pyfunction]
pub(crate) fn state_from_spelling(spelling: &str) -> Option<u16> {
    State::from_spelling(spelling).map(State::code)
}

/// The code of the state one FIX status field's code names, or `None` where
/// the tag is no status or the code says nothing about one.
#[pyfunction]
pub(crate) fn state_from_fix_status(tag: i32, code: &str) -> Option<u16> {
    State::from_fix_status(tag, code).map(State::code)
}

/// The code of the state a FIX message type asks for, or `None`.
#[pyfunction]
pub(crate) fn state_from_fix_msgtype(msgtype: &str) -> Option<u16> {
    State::from_fix_msgtype(msgtype).map(State::code)
}
