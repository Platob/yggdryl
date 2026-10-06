//! How long an order stands: the member table `TimeInForce` is built from,
//! and the FIX reading of its `TimeInForce(59)` wire values.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl::TimeInForce;

use crate::napi_error;

/// One member of the core's time-in-force enum: its stored name, the code a
/// `timeinforce` column stores, and what it means.
#[napi(object)]
pub struct TimeInForceMember {
    pub name: String,
    pub code: i32,
    pub description: String,
}

/// Every member of the core enum, in code order, so the JavaScript
/// `TimeInForce` is built from the core's own table and never lists a member
/// itself.
#[napi(js_name = "_timeInForceMembersNative", skip_typescript)]
pub fn time_in_force_members_native() -> Vec<TimeInForceMember> {
    TimeInForce::ALL
        .iter()
        .map(|member| TimeInForceMember {
            name: member.as_str().to_owned(),
            code: i32::from(member.code()),
            description: member.description().to_owned(),
        })
        .collect()
}

/// The member one `TimeInForce(59)` wire value stands for, as its stored
/// name; a value no member names - a venue's own - is `OTHER`.
#[napi(js_name = "timeInForceFromFix")]
pub fn time_in_force_from_fix(wire: String) -> String {
    TimeInForce::from_fix(&wire).as_str().to_owned()
}

/// The `TimeInForce(59)` wire value the member `name` stands for, or `null`
/// for `UKNW` and `OTHER`; throws on a name that is no member.
#[napi(js_name = "timeInForceFixCode")]
pub fn time_in_force_fix_code(name: String) -> Result<Option<String>> {
    let member = TimeInForce::from_spelling(&name)
        .ok_or_else(|| napi_error(format!("{name:?} is no timeinforce member")))?;
    Ok(member.fix_code().map(str::to_owned))
}
