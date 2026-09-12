//! What a byte datatype declares beyond its layout.

use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use super::BytesLayout;
use crate::{Error, Result};

/// The name this crate's bounded byte datatypes ride Arrow under.
pub const BYTES_EXTENSION_NAME: &str = "yggdryl.bytes";

/// A byte layout and its bound.
///
/// Arrow carries the layout and, for the fixed one, the width; it has no
/// `varbinary(n)`. So the maximum rides here, and crosses an Arrow boundary
/// as this crate's own extension metadata under `yggdryl.bytes`.
///
/// One number carries both bounds because a column is one shape or the
/// other: on [`BytesLayout::FixedSizeBinary`] it is the exact width every
/// value fills, and on every other layout it is the most bytes a value may
/// hold. So [`Self::fixed`] and [`Self::max`] are two readings of one fact,
/// and exactly one of them ever answers.
///
/// ```
/// use yggdryl::types::{BytesLayout, BytesParameters};
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let parameters = BytesParameters::new(BytesLayout::Binary).try_with_bound(32)?;
/// assert_eq!(parameters.max(), Some(32));
/// assert_eq!(parameters.fixed(), None);
///
/// let dtype = DataType::bytes(parameters)?;
/// assert_eq!(dtype.to_string(), "binary(32)");
/// assert_eq!(DataType::from_str(&dtype.to_string())?, dtype);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BytesParameters {
    layout: BytesLayout,
    bound: Option<NonZeroU32>,
}

impl BytesParameters {
    /// Unbounded bytes in one layout.
    #[must_use]
    pub const fn new(layout: BytesLayout) -> Self {
        Self {
            layout,
            bound: None,
        }
    }

    /// The layout the values are stored in.
    #[must_use]
    pub const fn layout(self) -> BytesLayout {
        self.layout
    }

    /// The declared byte bound, whichever shape the layout gives it.
    #[must_use]
    pub const fn bound(self) -> Option<u32> {
        match self.bound {
            Some(bound) => Some(bound.get()),
            None => None,
        }
    }

    /// The exact bytes every value fills, on the fixed layout.
    #[must_use]
    pub const fn fixed(self) -> Option<u32> {
        match self.layout.is_fixed() {
            true => self.bound(),
            false => None,
        }
    }

    /// The most bytes a value may hold, on a variable layout.
    #[must_use]
    pub const fn max(self) -> Option<u32> {
        match self.layout.is_fixed() {
            true => None,
            false => self.bound(),
        }
    }

    /// Return whether every value is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        self.layout.is_fixed()
    }

    /// Return whether the values are bounded at all.
    #[must_use]
    pub const fn is_bounded(self) -> bool {
        self.bound.is_some()
    }

    /// Return these parameters in another layout.
    #[must_use]
    pub const fn with_layout(mut self, layout: BytesLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Return these parameters bounded to `bound` bytes, the bound already
    /// proven non-zero.
    #[must_use]
    pub const fn with_bound(mut self, bound: NonZeroU32) -> Self {
        self.bound = Some(bound);
        self
    }

    /// Return these parameters bounded to `bound` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a bound of zero: a column of
    /// no bytes is a column of one value, which is a declaration nobody
    /// means.
    pub fn try_with_bound(mut self, bound: u32) -> Result<Self> {
        self.bound = Some(NonZeroU32::new(bound).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a {} of at least one byte, got 0",
                self.bound_word()
            ))
        })?);
        Ok(self)
    }

    /// Return these parameters with no bound.
    #[must_use]
    pub const fn without_bound(mut self) -> Self {
        self.bound = None;
        self
    }

    /// Return these parameters with no maximum, keeping a fixed width.
    ///
    /// A maximum is a column's rule and a fixed width is a value's shape, so
    /// this is what a value carries out of a bounded column.
    #[must_use]
    pub const fn without_max(self) -> Self {
        match self.layout.is_fixed() {
            true => self,
            false => self.without_bound(),
        }
    }

    /// Check that the layout and the bound agree.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for the fixed layout with no width:
    /// the width is what makes it fixed, so there is no width-free spelling
    /// of it.
    pub fn validate(self) -> Result<()> {
        if self.layout.is_fixed() && self.bound.is_none() {
            return Err(invalid(format_smolstr!(
                "expected {}(width), got no width",
                self.layout.as_str()
            )));
        }
        Ok(())
    }

    /// The word this layout calls its bound by.
    const fn bound_word(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "width",
            false => "maximum",
        }
    }

    /// The metadata key the bound is written under.
    const fn bound_word_key(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "fixed",
            false => "max",
        }
    }

    /// The extension metadata an Arrow field carries these in.
    ///
    /// Arrow has nowhere to put a maximum: the four binary layouts declare
    /// their offsets and, for the fixed one, the width, and nothing else. So
    /// a maximum rides the `ARROW:extension:metadata` document beside the
    /// `yggdryl.bytes` name, with the layout written whole so a reader can
    /// check it against the storage.
    #[must_use]
    pub fn extension_json(self) -> String {
        let mut rendered = String::with_capacity(48);
        rendered.push_str("{\"layout\":\"");
        rendered.push_str(self.layout.as_str());
        rendered.push('"');
        if let Some(bound) = self.bound {
            rendered.push(',');
            rendered.push('"');
            rendered.push_str(self.bound_word_key());
            rendered.push_str("\":");
            rendered.push_str(&format_smolstr!("{bound}"));
        }
        rendered.push('}');
        rendered
    }

    /// Read parameters back out of Arrow extension metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the document is not an object
    /// naming a layout this crate knows, or bounds a layout the wrong way.
    pub fn from_extension_json(value: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Document {
            layout: SmolStr,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let document: Document = serde_json::from_str(value)
            .map_err(|error| invalid(format_smolstr!("expected bytes parameters, got {error}")))?;
        let layout = BytesLayout::from_str(&document.layout)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        let mut parameters = Self::new(layout);
        match (layout.is_fixed(), document.fixed, document.max) {
            (true, Some(fixed), None) => parameters = parameters.try_with_bound(fixed)?,
            (false, None, Some(max)) => parameters = parameters.try_with_bound(max)?,
            (_, None, None) => {}
            (true, _, Some(max)) => {
                return Err(invalid(format_smolstr!(
                    "expected a fixed width on {layout}, got max={max}"
                )));
            }
            (false, Some(fixed), _) => {
                return Err(invalid(format_smolstr!(
                    "expected a maximum on {layout}, got fixed={fixed}"
                )));
            }
        }
        parameters.validate()?;
        Ok(parameters)
    }
}

/// The refusal every invalid parameter answers with.
fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidDataType {
        kind: "bytes",
        reason: reason.into(),
    }
}

impl Default for BytesParameters {
    fn default() -> Self {
        Self::new(BytesLayout::Binary)
    }
}

impl From<BytesLayout> for BytesParameters {
    fn from(value: BytesLayout) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for BytesParameters {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.layout.as_str())?;
        match self.bound {
            None => Ok(()),
            Some(bound) => write!(formatter, "({bound})"),
        }
    }
}

impl Serialize for BytesParameters {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let declared = usize::from(self.bound.is_some());
        let mut state = serializer.serialize_struct("BytesParameters", 1 + declared)?;
        state.serialize_field("layout", &self.layout)?;
        if let Some(bound) = self.bound {
            state.serialize_field(self.bound_word_key(), &bound.get())?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for BytesParameters {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Representation {
            layout: BytesLayout,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let value = Representation::deserialize(deserializer)?;
        let mut parameters = Self::new(value.layout);
        if let Some(bound) = value.fixed.or(value.max) {
            parameters = parameters
                .try_with_bound(bound)
                .map_err(serde::de::Error::custom)?;
        }
        parameters.validate().map_err(serde::de::Error::custom)?;
        Ok(parameters)
    }
}
