//! [`DecimalError`] — the guided failures of decimal construction and arithmetic. Every message
//! names the type, the offending value, and how to fix it; the same text surfaces unchanged as a
//! Python `ValueError` and a Node thrown `Error`.

/// A decimal construction, arithmetic, or conversion failure. Each variant's [`Display`] is a
/// **guided** message: it names the decimal type, the offending value, and the remedy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecimalError {
    /// A coefficient did not fit the width's integer range.
    CoefficientOutOfRange {
        /// The decimal type name (`"d32"` …).
        ty: &'static str,
        /// The maximum precision (significant digits) the width holds.
        max_precision: u8,
    },
    /// A value's precision (significant digits) exceeded a column's declared precision.
    PrecisionExceeded {
        /// The decimal type name (`"d32"` …).
        ty: &'static str,
        /// The value's precision (significant digits).
        precision: u32,
        /// The column's declared maximum precision.
        max: u8,
    },
    /// An arithmetic result overflowed the coefficient integer.
    Overflow {
        /// The decimal type name (`"d32"` …).
        ty: &'static str,
        /// The operation that overflowed (`"add"`, `"mul"`, `"rescale"`, …).
        op: &'static str,
    },
    /// A division (or remainder) by zero.
    DivideByZero {
        /// The decimal type name.
        ty: &'static str,
    },
    /// A rescale to a smaller scale would drop non-zero fractional digits (a lossy narrowing the
    /// caller must opt into via a rounding/truncating method instead).
    InexactRescale {
        /// The decimal type name.
        ty: &'static str,
        /// The current scale.
        from: i8,
        /// The requested (smaller) scale.
        to: i8,
    },
    /// A value with a non-zero fractional part cannot be converted to an integer exactly.
    NotInteger {
        /// The decimal type name.
        ty: &'static str,
        /// The value's scale.
        scale: i8,
    },
    /// A value's magnitude did not fit the requested target width during a cast.
    OutOfWidth {
        /// The source decimal type name.
        from: &'static str,
        /// The target decimal type name.
        to: &'static str,
    },
    /// A string was not a valid decimal literal.
    ParseError {
        /// The decimal type name.
        ty: &'static str,
    },
    /// A non-finite `f64` (`NaN` / `±inf`) has no decimal value.
    NonFinite {
        /// The decimal type name.
        ty: &'static str,
    },
    /// An in-place `set` / `set_range` addressed an element (or range) outside the column.
    IndexOutOfBounds {
        /// The offending element index.
        index: usize,
        /// The column length the index had to fall inside.
        len: usize,
    },
}

impl core::fmt::Display for DecimalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PrecisionExceeded { ty, precision, max } => write!(
                f,
                "{ty} value needs {precision} significant digits but the column allows only {max} \
                 — declare a larger precision (up to the type maximum) or round to fewer digits"
            ),
            Self::CoefficientOutOfRange { ty, max_precision } => write!(
                f,
                "{ty} coefficient out of range: it exceeds the {max_precision}-digit maximum \
                 precision — use a wider decimal (d64/d128/d256) or reduce the scale"
            ),
            Self::Overflow { ty, op } => write!(
                f,
                "{ty} {op} overflow: the result exceeds the coefficient integer range — use a \
                 wider decimal (d64/d128/d256), or the checked_* method to handle the overflow"
            ),
            Self::DivideByZero { ty } => {
                write!(f, "{ty} division by zero: the divisor must be non-zero")
            }
            Self::InexactRescale { ty, from, to } => write!(
                f,
                "{ty} rescale from scale {from} to {to} would drop non-zero fractional digits — \
                 use round_to_scale/trunc_to_scale to opt into the loss, or keep scale >= {from}"
            ),
            Self::NotInteger { ty, scale } => write!(
                f,
                "{ty} value has scale {scale} (a fractional part) and is not an exact integer — \
                 use trunc()/round() first, or to_f64() for an approximation"
            ),
            Self::OutOfWidth { from, to } => write!(
                f,
                "{from} value does not fit {to}: its magnitude exceeds the {to} coefficient range \
                 — cast to a wider decimal, or round to fewer significant digits first"
            ),
            Self::ParseError { ty } => write!(
                f,
                "invalid {ty} literal: expected an optional sign, digits, and an optional \
                 '.'-separated fraction (e.g. \"-123.45\")"
            ),
            Self::NonFinite { ty } => write!(
                f,
                "cannot build a {ty} from a non-finite f64 (NaN or +/-inf): pass a finite value"
            ),
            Self::IndexOutOfBounds { index, len } => write!(
                f,
                "index {index} is out of bounds for a decimal column of length {len}: `set` \
                 overwrites an existing element — `push` to grow the column, or index within [0, {len})"
            ),
        }
    }
}

impl std::error::Error for DecimalError {}
