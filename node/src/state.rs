//! What state one thing is in: the member table `State` is built from.

use napi_derive::napi;
use yggdryl::State;

/// One member of the core's state enum: its stored name, the code a `state`
/// column stores, what it means, and the rank the code's hundreds state.
#[napi(object)]
pub struct StateMember {
    pub name: String,
    pub code: i32,
    pub description: String,
    pub rank: u32,
}

/// Every member of the core enum, in code order, so the JavaScript `State`
/// is built from the core's own table and never lists a member itself.
#[napi(js_name = "_stateMembersNative", skip_typescript)]
pub fn state_members_native() -> Vec<StateMember> {
    State::ALL
        .iter()
        .map(|state| StateMember {
            name: state.as_str().to_owned(),
            code: state.code(),
            description: state.description().to_owned(),
            rank: u32::from(state.rank()),
        })
        .collect()
}
