//! The claim every FIX harness makes before a test reads a name: the core's
//! register answers a market kind and a FIX Latest name only once
//! `yggdryl-fix` - and the market crate under it - has claimed them, so
//! each test opens with [`installed`].

/// Claims the FIX crate's names and the market crate's kinds, once for the
/// process.
pub fn installed() {
    yggdryl_fix::install().expect("yggdryl-fix claims its names");
}
