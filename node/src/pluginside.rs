//! The role of a FIX plugin: the member table `PluginSide` is built from,
//! the reading of a `CBlock`'s plugin class, and the one coercion every door
//! taking a role reads through.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl::PluginSide;

use crate::{exact_i64, napi_error};

/// One member of the core's plugin-side enum: its stored name, the code a
/// `pluginside` column stores, and what it means.
#[napi(object)]
pub struct PluginSideMember {
    pub name: String,
    pub code: i32,
    pub description: String,
}

/// Every member of the core enum, in code order, so the JavaScript
/// `PluginSide` is built from the core's own table and never lists a member
/// itself.
#[napi(js_name = "_pluginSideMembersNative", skip_typescript)]
pub fn plugin_side_members_native() -> Vec<PluginSideMember> {
    PluginSide::ALL
        .iter()
        .map(|member| PluginSideMember {
            name: member.as_str().to_owned(),
            code: i32::from(member.code()),
            description: member.description().to_owned(),
        })
        .collect()
}

/// The role one plugin class names, as its stored name: a `CBlock` root's
/// `type`, whose last `.`-separated segment, folded, holding `buyside` is
/// `BUYS`, holding `sellside` is `SELL`, and anything else `UKNW`. Never
/// throws.
#[napi(js_name = "pluginSideFromPluginType")]
pub fn plugin_side_from_plugin_type(plugin_type: String) -> String {
    PluginSide::from_plugin_type(&plugin_type)
        .as_str()
        .to_owned()
}

/// The role `side` names: a spelling read through the core `PluginSide`
/// vocabulary - the stored name in any case, `BuySide`, `sell-side` - or a
/// `PluginSide` code, what `PluginSide.BUYS` holds.
pub(crate) fn plugin_side_of(side: Either<String, f64>) -> Result<PluginSide> {
    match side {
        Either::A(text) => PluginSide::read(&text),
        Either::B(code) => PluginSide::read_code(exact_i64(code, "pluginside")?),
    }
    .map_err(napi_error)
}
