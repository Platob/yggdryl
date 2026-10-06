//! The role of a FIX plugin: the member table `yggdryl.PluginSide` is built
//! from.

use pyo3::prelude::*;
use yggdryl::PluginSide;

/// Every member of the core enum, in code order: its stored name, the code a
/// column stores and what it means.
///
/// The Python enum is built from this once at import, so the binding lists
/// no member of its own.
#[pyfunction]
pub(crate) fn pluginside_members() -> Vec<(&'static str, u8, &'static str)> {
    PluginSide::ALL
        .iter()
        .map(|member| (member.as_str(), member.code(), member.description()))
        .collect()
}

/// The code of the member one spelling names - the stored name in any case,
/// or the role's own name folded (`BuySide`, `sell-side`) - or `None` where
/// none does.
#[pyfunction]
pub(crate) fn pluginside_from_spelling(spelling: &str) -> Option<u8> {
    PluginSide::from_spelling(spelling).map(PluginSide::code)
}

/// The code of the role one plugin class name states - a `CBlock`'s root
/// `type` attribute - `UKNW` for a class naming no role.
#[pyfunction]
pub(crate) fn pluginside_from_plugin_type(class: &str) -> u8 {
    PluginSide::from_plugin_type(class).code()
}
