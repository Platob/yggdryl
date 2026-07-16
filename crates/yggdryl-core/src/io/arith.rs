//! [`ArithOp`] — the closed set of the **element-wise arithmetic** operators (`+ - * / %`), the one
//! place the op space is enumerated, plus its wrapping `i128` kernel (the shared integer semantics
//! the temporal backing path runs through).
//!
//! It is a *crate-internal* dispatch detail: the public surface is the named `add` / `sub` / `mul` /
//! `div` / `rem` methods (and their `_unchecked` twins) on [`Serie`](crate::io::fixed::Serie) and the
//! erased [`AnySerie`](crate::io::AnySerie); each selects one `ArithOp` and threads it through the
//! shared single-pass loop, so adding an operator touches one enum, not five parallel loops.

/// One element-wise arithmetic operator. The single enumerated op space the typed fast path
/// ([`Serie::add_unchecked`](crate::io::fixed::Serie) …) and the erased base ops
/// ([`dyn AnySerie::add`](crate::io::AnySerie) …) both dispatch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArithOp {
    /// `a + b`.
    Add,
    /// `a - b`.
    Sub,
    /// `a * b`.
    Mul,
    /// `a / b` — integer division by zero yields *no value* (a null), never a panic.
    Div,
    /// `a % b` — integer remainder by zero yields *no value* (a null), never a panic.
    Rem,
}

impl ArithOp {
    /// Applies this operator to two `i128`s with **wrapping** (modular) integer semantics — the
    /// shared kernel the temporal backing-integer path runs through. `Add` / `Sub` / `Mul` always
    /// produce a value (wrapping at the `i128` boundary); `Div` / `Rem` produce `None` when the
    /// divisor is zero (the caller turns that into a null, never a panic). A non-zero `Div` / `Rem`
    /// uses the wrapping form so the sole overflow case (`MIN / -1`) wraps rather than panics.
    pub(crate) fn apply_i128_wrapping(self, a: i128, b: i128) -> Option<i128> {
        match self {
            Self::Add => Some(a.wrapping_add(b)),
            Self::Sub => Some(a.wrapping_sub(b)),
            Self::Mul => Some(a.wrapping_mul(b)),
            Self::Div => (b != 0).then(|| a.wrapping_div(b)),
            Self::Rem => (b != 0).then(|| a.wrapping_rem(b)),
        }
    }
}
