//! The claim every market harness makes before a test reads a name: the
//! core's register answers a market kind only once `yggdryl-market` has
//! claimed it, so each test opens with [`installed`].

/// Claims the market crate's kinds, once for the process.
pub fn installed() {
    yggdryl_market::install().expect("yggdryl-market claims its kinds");
}
