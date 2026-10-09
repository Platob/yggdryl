//! One test file per file at the crate root, under `tests/root/`.
//!
//! `rust/market/tests/` mirrors `rust/market/src/`: a source file has exactly
//! one test file at the matching path, and this target is the harness for
//! the files the crate root itself holds - and, at a core root file's name,
//! what that file answers for the kinds this crate claims. A test reaches
//! this crate through `yggdryl_market::` and the core through `yggdryl::`;
//! where what it pins is not reachable that way, it reaches
//! `yggdryl_market::internals`, which exists only under the `internals`
//! feature, and the file that reaches it is declared behind that feature
//! here.

#[path = "root/code.rs"]
mod code;
#[path = "root/compatibility.rs"]
mod compatibility;
#[path = "root/datatype.rs"]
mod datatype;
#[path = "root/datatype_id.rs"]
mod datatype_id;
#[path = "root/enums.rs"]
mod enums;
#[path = "root/eusipa.rs"]
mod eusipa;
#[path = "root/identifier.rs"]
mod identifier;
#[path = "root/idkey.rs"]
mod idkey;
#[path = "root/idsource.rs"]
mod idsource;
#[path = "root/idtype.rs"]
mod idtype;
#[path = "root/implementer.rs"]
mod implementer;
#[path = "support/install.rs"]
mod install;
#[path = "root/isin_registry.rs"]
mod isin_registry;
#[path = "root/limit.rs"]
mod limit;
#[path = "root/market.rs"]
mod market;
#[path = "root/marketdatakind.rs"]
mod marketdatakind;
#[path = "root/marketdatatype.rs"]
mod marketdatatype;
#[path = "root/scalar.rs"]
mod scalar;
#[path = "root/securityid.rs"]
mod securityid;
#[path = "root/side.rs"]
mod side;
#[path = "root/timeinforce.rs"]
mod timeinforce;
#[path = "root/valuestream.rs"]
mod valuestream;
