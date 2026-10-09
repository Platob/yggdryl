//! ISO 18774 financial instrument short names.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::code::{code_value, folded_code};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One ISO 18774 Financial Instrument Short Name, held by its shape.
///
/// At most thirty-five printable ASCII characters, delimiter included: the
/// issuer's short name, a `/`, then the instrument's description -
/// `ACME CORP/SH`. The separator is the first `/`: a description may hold
/// another (`AMORT PN W/P/C`), and dots, ampersands, parentheses, hyphens
/// and colons besides. The issuer takes at most fifteen characters by the
/// ANNA guidelines, save a collective investment vehicle's or an OTC
/// derivative's, whose issuer may run longer, so that bound is not a shape
/// rule. The standard writes upper case alone, so lower case folds once at
/// construction. ISO 18774 draws on ISO/IEC 8859-1, while a code here stores
/// US-ASCII: a Latin-1 letter is refused. No check character: every
/// well-shaped value ranks the same. A short name past twenty-three bytes
/// spills the compact string into one shared allocation, as a Bloomberg
/// identifier or a RIC does.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Fisn(SmolStr);

impl<'de> Deserialize<'de> for Fisn {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Fisn {
    /// Validate and construct a Financial Instrument Short Name: at most
    /// thirty-five printable ASCII bytes, an issuer and a description either
    /// side of the first `/`, upper-cased.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Fisn};
    ///
    /// let name = Fisn::new("acme corp/sh")?;
    /// assert_eq!(name.as_str(), "ACME CORP/SH");
    /// assert_eq!(name.issuer(), "ACME CORP");
    /// assert_eq!(name.description(), "SH");
    /// assert!(name.is_real());
    /// assert_eq!(Fisn::new("ACME CORP/AMORT PN W/P/C")?.description(), "AMORT PN W/P/C");
    /// assert!(Fisn::new("ACME CORP SH").is_err(), "no '/'");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is wider than thirty-five bytes, holds
    /// a byte that is not printable ASCII, or does not state an issuer and a
    /// description either side of a `/`.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        folded_code::<FISN_WIDTH>("fisn", value.as_ref(), Self::refusal).map(Self)
    }

    /// Borrow the short name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the compact storage without copying the short name.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The issuer's short name: everything before the first `/`.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.split().0
    }

    /// The instrument's description: everything after the first `/`.
    #[must_use]
    pub fn description(&self) -> &str {
        self.split().1
    }

    /// Whether `text` is exactly the spelling this type stores: upper case
    /// and of the shape [`Self::new`] admits.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() <= FISN_WIDTH
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// How alike two short names are, from `0` to `1`: one less the
    /// Levenshtein distance between their bytes over the longer's length,
    /// both already upper case. Symmetric, `1` for two equal names; what
    /// an ISIN registry scores an economic match by. The distance is never less than the two lengths'
    /// difference, so `1 - |a - b| / max(a, b)` bounds it from above: a pair
    /// whose lengths differ by more than `(1 - threshold) * max(a, b)` is
    /// below `threshold` without the distance being computed. Allocates
    /// nothing.
    ///
    /// ```
    /// use yggdryl::Fisn;
    ///
    /// let apple = Fisn::new("APPLE INC/SH")?;
    /// assert_eq!(apple.similarity(&apple), 1.0);
    /// let dotted = Fisn::new("APPLE INC./SH")?;
    /// assert!((apple.similarity(&dotted) - 12.0 / 13.0).abs() < 1e-12, "one insertion in thirteen");
    /// assert_eq!(apple.similarity(&dotted), dotted.similarity(&apple));
    /// assert_eq!(apple.similarity(&Fisn::new("APPLE INC/SH USD")?), 0.75);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    #[must_use]
    pub fn similarity(&self, other: &Fisn) -> f64 {
        similarity(self.as_str(), other.as_str())
    }

    fn split(&self) -> (&str, &str) {
        self.as_str()
            .split_once('/')
            .expect("a short name holds its '/'")
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        if !folded.bytes().all(|byte| matches!(byte, b' '..=b'~')) {
            return Some("expected printable characters");
        }
        match folded.split_once('/') {
            None => Some("expected a '/' between the issuer and the instrument description"),
            Some(("", _)) => Some("expected an issuer name before the '/'"),
            Some((_, "")) => Some("expected an instrument description after the '/'"),
            Some(_) => None,
        }
    }
}

impl fmt::Display for Fisn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Fisn, Fisn, FISN_WIDTH);

/// [`Fisn::similarity`] over two texts: `1` where both are empty. A text
/// past [`FISN_WIDTH`] bytes is scored all the same, its row then on the
/// heap.
pub(crate) fn similarity(left: &str, right: &str) -> f64 {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    // The row runs over the shorter text.
    let (longer, shorter) = if left.len() < right.len() {
        (right, left)
    } else {
        (left, right)
    };
    if longer.is_empty() {
        return 1.0;
    }
    let mut row: SmallVec<[usize; FISN_WIDTH + 1]> = (0..=shorter.len()).collect();
    for (at, byte) in longer.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = at + 1;
        for (column, other) in shorter.iter().enumerate() {
            let above = row[column + 1];
            row[column + 1] = (diagonal + usize::from(byte != other))
                .min(above + 1)
                .min(row[column] + 1);
            diagonal = above;
        }
    }
    #[allow(clippy::cast_precision_loss)] // both lengths are at most a few dozen bytes
    let score = 1.0 - row[shorter.len()] as f64 / longer.len() as f64;
    score
}

/// Whether two texts of `left` and `right` bytes cannot be `threshold`
/// similar: their lengths differ by more than `(1 - threshold)` of the
/// longer, which no distance the table would compute can make up.
pub(crate) fn below_threshold(left: usize, right: usize, threshold: f64) -> bool {
    #[allow(clippy::cast_precision_loss)] // both lengths are at most a few dozen bytes
    let (difference, longest) = (left.abs_diff(right) as f64, left.max(right) as f64);
    difference > (1.0 - threshold) * longest
}

/// The Arrow extension name of a financial instrument short name.
pub(crate) const FISN_EXTENSION_NAME: &str = "yggdryl.fisn";

/// The most bytes a financial instrument short name may be.
pub(crate) const FISN_WIDTH: usize = 35;

impl DataType {
    /// Creates ISO 18774's financial instrument short name datatype: at
    /// most thirty-five bytes.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::fisn(), DataType::Fisn);
    /// assert_eq!(DataType::fisn().to_string(), "fisn");
    /// assert_eq!(DataType::fisn().code_width(), Some(35));
    /// ```
    #[must_use]
    pub const fn fisn() -> Self {
        Self::Fisn
    }
}

define_field_types!(FisnType, Fisn);

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/fisn.rs` pins and a caller cannot reach.

    /// [`Fisn::similarity`](super::Fisn::similarity) over two texts.
    #[must_use]
    pub fn similarity(left: &str, right: &str) -> f64 {
        super::similarity(left, right)
    }

    /// Whether lengths `left` and `right` are below `threshold` by length
    /// alone.
    #[must_use]
    pub fn below_threshold(left: usize, right: usize, threshold: f64) -> bool {
        super::below_threshold(left, right, threshold)
    }
}
