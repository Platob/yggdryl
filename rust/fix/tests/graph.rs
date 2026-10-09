//! One test file per file under `rust/market/src/graph/` that answers for
//! what `yggdryl-fix` owns, in `tests/graph/`. A test reaches this crate
//! through `yggdryl_fix::`, the market crate through `yggdryl_market::` and
//! the core through `yggdryl::`.

#[path = "support/install.rs"]
mod install;
#[path = "graph/iterator.rs"]
mod iterator;
#[path = "graph/market_data.rs"]
mod market_data;
