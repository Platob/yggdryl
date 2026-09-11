//! What a string datatype declares beyond its layout.

use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use super::StringLayout;
use crate::{Charset, Error, Result};

/// The name this crate's string datatypes ride Arrow under.
pub const STRING_EXTENSION_NAME: &str = "yggdryl.string";

/// A string layout, the charset its bytes are written in, and its bound.
///
/// Arrow carries none of the three together: its string layouts imply UTF-8
/// and declare no length at all, and its binary layouts declare neither. So
/// all three ride here, and cross an Arrow boundary as this crate's own
/// extension metadata under `yggdryl.string`.
///
/// The bound counts **bytes of the stored encoding**, not scalars. That is
/// the number the buffer holds, the number Arrow's offsets measure, and - for
/// every single-byte charset - the scalar count as well. Counting scalars
/// instead would make a bound a walk of the value rather than a subtraction
/// of two offsets.
///
/// One number carries both bounds because a string is one shape or the other:
/// on [`StringLayout::FixedString`] it is the exact width every value fills,
/// and on every other layout it is the most bytes a value may hold. So
/// [`Self::fixed`] and [`Self::max`] are two readings of one fact, and
/// exactly one of them ever answers.
///
/// ```
/// use yggdryl::types::{StringLayout, StringParameters};
/// use yggdryl::{Charset, DataType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let parameters = StringParameters::new(StringLayout::String, Charset::Cp1252).try_with_bound(32)?;
/// assert_eq!(parameters.charset(), Charset::Cp1252);
/// assert_eq!(parameters.max(), Some(32));
/// assert_eq!(parameters.fixed(), None);
///
/// let dtype = DataType::string(parameters)?;
/// assert_eq!(dtype.to_string(), "string(windows-1252,32)");
/// assert_eq!(DataType::from_str(&dtype.to_string())?, dtype);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StringParameters {
    layout: StringLayout,
    charset: Charset,
    bound: Option<NonZeroU32>,
}

impl StringParameters {
    /// An unbounded string in one layout and charset.
    #[must_use]
    pub const fn new(layout: StringLayout, charset: Charset) -> Self {
        Self {
            layout,
            charset,
            bound: None,
        }
    }

    /// An unbounded UTF-8 string in one layout.
    #[must_use]
    pub const fn utf8(layout: StringLayout) -> Self {
        Self::new(layout, Charset::Utf8)
    }

    /// The layout the values are stored in.
    #[must_use]
    pub const fn layout(self) -> StringLayout {
        self.layout
    }

    /// The charset the stored bytes are written in.
    #[must_use]
    pub const fn charset(self) -> Charset {
        self.charset
    }

    /// The declared byte bound, whichever shape the layout gives it.
    #[must_use]
    pub const fn bound(self) -> Option<u32> {
        match self.bound {
            Some(bound) => Some(bound.get()),
            None => None,
        }
    }

    /// The exact bytes every value fills, on a fixed layout.
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
    pub const fn with_layout(mut self, layout: StringLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Return these parameters in another charset.
    #[must_use]
    pub const fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = charset;
        self
    }

    /// Return these parameters bounded to `bound` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a bound of zero: a string of no
    /// bytes is a column of one value, which is a declaration nobody means.
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

    /// Check that the layout and the bound agree.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a fixed layout with no width:
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

    /// Return whether these parameters add nothing to the layout alone.
    ///
    /// Unbounded UTF-8 is already a datatype for three of the five layouts,
    /// and unbounded US-ASCII for two more, so parameters answering `true`
    /// here redirect to those rather than becoming a second spelling of them.
    #[must_use]
    pub const fn is_plain(self) -> bool {
        !self.is_bounded()
    }

    /// The extension metadata an Arrow field carries these in.
    ///
    /// Arrow has nowhere else to put them: neither a string nor a binary
    /// array declares a charset, a length bound, or which of the two view
    /// layouts it is, so all three ride the `ARROW:extension:metadata`
    /// document beside the `yggdryl.string` name. The layout is written
    /// whole rather than inferred, because a reader that only knows Arrow
    /// sees one view layout where this crate declares two.
    #[must_use]
    pub fn extension_json(self) -> String {
        let mut rendered = String::with_capacity(64);
        rendered.push_str("{\"layout\":\"");
        rendered.push_str(self.layout.as_str());
        rendered.push_str("\",\"charset\":\"");
        rendered.push_str(self.charset.as_str());
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

    /// The metadata key the bound is written under.
    const fn bound_word_key(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "fixed",
            false => "max",
        }
    }

    /// Read parameters back out of Arrow extension metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the document is not an object
    /// naming a layout and a charset this crate knows, or bounds a layout the
    /// wrong way.
    pub fn from_extension_json(value: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Document {
            layout: SmolStr,
            charset: SmolStr,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let document: Document = serde_json::from_str(value)
            .map_err(|error| invalid(format_smolstr!("expected string parameters, got {error}")))?;
        let layout = StringLayout::from_str(&document.layout)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        let charset = Charset::from_str(&document.charset)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        let mut parameters = Self::new(layout, charset);
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
        kind: "string",
        reason: reason.into(),
    }
}

impl Default for StringParameters {
    fn default() -> Self {
        Self::utf8(StringLayout::String)
    }
}

impl From<Charset> for StringParameters {
    fn from(value: Charset) -> Self {
        Self::new(StringLayout::String, value)
    }
}

impl From<StringLayout> for StringParameters {
    fn from(value: StringLayout) -> Self {
        Self::utf8(value)
    }
}

impl fmt::Display for StringParameters {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    ///
    /// UTF-8 is the default, so a UTF-8 string renders under the layout's
    /// `utf8` name with no charset to state; every other charset renders
    /// under the `string` name and states it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let utf8 = self.charset.is_utf8();
        formatter.write_str(match utf8 {
            true => self.layout.as_utf8_str(),
            false => self.layout.as_str(),
        })?;
        match (utf8, self.bound) {
            (true, None) => Ok(()),
            (true, Some(bound)) => write!(formatter, "({bound})"),
            (false, None) => write!(formatter, "({})", self.charset),
            (false, Some(bound)) => write!(formatter, "({},{bound})", self.charset),
        }
    }
}

impl Serialize for StringParameters {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let declared = usize::from(self.bound.is_some());
        let mut state = serializer.serialize_struct("StringParameters", 2 + declared)?;
        state.serialize_field("layout", &self.layout)?;
        state.serialize_field("charset", &self.charset)?;
        if let Some(bound) = self.bound {
            state.serialize_field(self.bound_word_key(), &bound.get())?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for StringParameters {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Representation {
            layout: StringLayout,
            charset: Charset,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let value = Representation::deserialize(deserializer)?;
        let mut parameters = Self::new(value.layout, value.charset);
        if let Some(bound) = value.fixed.or(value.max) {
            parameters = parameters
                .try_with_bound(bound)
                .map_err(serde::de::Error::custom)?;
        }
        parameters.validate().map_err(serde::de::Error::custom)?;
        Ok(parameters)
    }
}
