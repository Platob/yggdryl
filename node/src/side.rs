//! Which side of the market a trade took: the member table `Side` is built
//! from.

use napi_derive::napi;
use yggdryl::Side;

/// One member of the core's side enum - FIX's `Side(54)`: its stored name,
/// the code a `side` column stores, what it means, its one-character FIX
/// code (`null` for `UNKNOWN`), and whether it is a bid or an ask.
#[napi(object)]
pub struct SideMember {
    pub name: String,
    pub code: i32,
    pub description: String,
    pub fix_code: Option<String>,
    pub is_bid: bool,
    pub is_ask: bool,
}

/// Every member of the core enum, in code order, so the JavaScript `Side`
/// is built from the core's own table and never lists a member itself.
#[napi(js_name = "_sideMembersNative", skip_typescript)]
pub fn side_members_native() -> Vec<SideMember> {
    Side::ALL
        .iter()
        .map(|side| SideMember {
            name: side.as_str().to_owned(),
            code: side.code(),
            description: side.description().to_owned(),
            fix_code: side.fix_code().map(String::from),
            is_bid: side.is_bid(),
            is_ask: side.is_ask(),
        })
        .collect()
}
