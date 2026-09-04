//! FIX field definitions: the `fix:` vocabulary, a registry that resolves
//! them, the shards it persists to, the process-wide default, and the
//! message value that is typed against one.
//!
//! A FIX field is a [`Field`](crate::Field) whose metadata carries the `fix:`
//! namespace, read and written through [`FixField`](crate::FixField) and
//! [`FixFieldMut`](crate::FixFieldMut) - nobody spells `fix:` at a call site.
//! The canonical name is the field's own `name()`, the datatype its own
//! `dtype()`, and the display name the generic `display` key; the namespace
//! adds only what FIX states beyond a field:
//!
//! | property | key | type | meaning |
//! | --- | --- | --- | --- |
//! | namespace | `fix:namespace` | [`FixNamespace`] | the dictionary this field belongs to; absent is [`FixNamespace::STANDARD`] |
//! | tag | `fix:tag` | `i32` | canonical FIX tag |
//! | tags | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
//! | aliases | `fix:aliases` | ordered name list | alternate names, highest priority first |
//! | description | `fix:description` | text | the specification's own wording |
//!
//! Nesting needs no second type: a component is a Struct field whose
//! children are its members, a repeating group is a List field whose item is
//! that Struct, and the group's counter tag is the group field's own `fix:tag`.
//!
//! # Identity
//!
//! [`FixId`] is a namespace and a tag, rendered `namespace:tag`. It is derived
//! on every read from `fix:namespace` and `fix:tag` and never stored: there is
//! no `fix:id` key, on disk or in the map, so the two facts it is computed
//! from cannot disagree with a third. [`FixId::from_parts`] is the one place
//! the standard-tag rule lives - a tag below [`FixId::STANDARD_TAG_LIMIT`] is
//! assigned by the FIX specification, so it forces
//! [`FixNamespace::STANDARD`] - which makes an inadmissible identifier
//! unconstructible rather than refused in several places.
//!
//! # Resolution
//!
//! [`FixRegistry`] answers an identifier or a name through two tiers within
//! one namespace, and a later tier is consulted only when every earlier one
//! missed:
//!
//! 1. canonical identifier, then alternate identifiers;
//! 2. canonical name folded, then aliases folded.
//!
//! Names fold ASCII case once, on the way in, so a query spelled in any case
//! finds the field and the answer is always the canonical spelling. A tag
//! query never consults names and a name query never consults tags, an alias
//! can never take a name away from a field that claims it canonically, and no
//! query ever crosses a namespace: a bare tag and a bare name are the standard
//! namespace, and a vendor field is reached by [`FixId`] or through the
//! namespace-qualified name accessors. [`FixKey`] carries any of the three
//! kinds of key through the generic [`FixRegistry::get_field`] /
//! [`FixRegistry::field`] pair, which match once and redirect to the
//! specialized accessor for that kind.
//!
//! # Storage
//!
//! A registry reads and writes through one [`IOBase`](crate::io::IOBase)
//! folder handle. Shards live at `<root>/records/<namespace>/<shard>.json`
//! with `shard = tag / 100`, each a JSON array of the core field document
//! ordered by canonical identifier, so a tag reaches exactly one shard by
//! arithmetic and an alternate tag never fans a field across shards. Every
//! shard is loaded on open: [`FixRegistry::from_handle`] lists `records/`,
//! reads each namespace folder and inserts every shard's fields, because a
//! name has no numeric structure to pick a shard with, and a dictionary is
//! small enough that loading it whole costs less than the machinery of
//! loading it lazily. A folder that does not exist loads as the empty
//! registry, which is the laziness contract of every handle; a leaf directly
//! under `records/`, a folder whose name is not a namespace, and a shard that
//! exists but does not parse are all typed errors naming their URL.
//!
//! # The process default
//!
//! [`FixRegistry::global`] is the registry every caller gets when it does not
//! name one, resolved once on the first call and never before. The order is
//! fixed: a registry installed by [`FixRegistry::install_global`], then the
//! folder `YGGDRYL_FIX_REGISTRY` names, then `~/.config/fix`, then the empty
//! registry - and only the third step treats absence as empty, because a
//! machine with no dictionary installed is an ordinary first-run state.
//! Every other failure is loud.
//!
//! ```
//! use yggdryl::{DataType, FixId, FixNamespace, FixRegistry};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut symbol = DataType::Utf8.required_field("Symbol");
//! symbol.as_fix_mut().set_tag(55)?;
//! symbol.as_fix_mut().set_aliases(["Ticker"])?;
//!
//! let mut trade = DataType::Utf8.required_field("TradeID");
//! trade.as_fix_mut().set_id(&FixId::from_str("cme:5001")?)?;
//!
//! let registry = FixRegistry::from_fields([symbol, trade])?;
//! assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
//! assert_eq!(registry.field_by_name(&FixNamespace::STANDARD, "ticker")?.name(), "Symbol");
//! assert_eq!(registry.field("SYMBOL")?.as_fix().id()?, Some(FixId::standard(55)));
//! // A vendor field is addressed by its identifier, never by a bare tag.
//! assert_eq!(registry.field(&FixId::from_str("cme:5001")?)?.name(), "TradeID");
//! assert!(registry.get_field_by_tag(5001).is_none());
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, SmolStrBuilder, format_smolstr};

use crate::{Error, Result};

mod field;
mod global;
mod msg;
mod registry;
mod store;
#[cfg(test)]
mod tests;

pub use field::FixAliases;
pub use msg::FixMsg;
pub use registry::{FixFieldIter, FixRegistry};

/// The dictionary one FIX field belongs to.
///
/// A namespace separates the FIX specification's own fields from a venue's:
/// `standard` is the specification, and any other spelling names a dictionary
/// that defines its own tags and names beside it. The type exists rather than
/// a bare string because it enforces what a string cannot - a leading ASCII
/// letter, a grammar with no `:` or `,` to confuse an identifier or a list,
/// ASCII case folded exactly once on the way in, and a length that keeps every
/// clone a `memcpy` - so `CME` and `cme` are one namespace and a registry
/// probe carrying one allocates nothing.
///
/// ```
/// use yggdryl::FixNamespace;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(FixNamespace::from_str("CME")?.as_str(), "cme");
/// assert_eq!(FixNamespace::default(), FixNamespace::STANDARD);
/// assert!(FixNamespace::from_str("standard")?.is_standard());
/// assert!(FixNamespace::from_str("2cme").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixNamespace(SmolStr);

impl FixNamespace {
    /// The FIX specification's own dictionary, and what an absent
    /// `fix:namespace` means.
    pub const STANDARD: Self = Self(SmolStr::new_static("standard"));

    /// The longest a namespace may be, in bytes.
    ///
    /// This is `smol_str`'s inline capacity, which is what makes a namespace
    /// clone a `memcpy` and keeps the registry's identifier and name probes
    /// allocation-free. Raising it past that bound would move the hot lookup
    /// path onto the heap.
    pub const MAX_LENGTH: usize = 23;

    /// Parses and validates a namespace, folding ASCII case.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the text does
    /// not start with an ASCII letter, holds a byte outside the grammar, or
    /// is longer than [`Self::MAX_LENGTH`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Returns the canonical lowercase spelling without allocating.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns whether this is the FIX specification's own dictionary.
    pub fn is_standard(&self) -> bool {
        self.0 == Self::STANDARD.0
    }
}

impl FromStr for FixNamespace {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let bytes = value.as_bytes();
        if bytes.first().is_none_or(|byte| !byte.is_ascii_alphabetic()) {
            return Err(Error::Parse {
                target: "fix namespace",
                position: 0,
                reason: "a fix namespace must start with an ASCII letter".into(),
            });
        }
        if let Some(position) = bytes
            .iter()
            .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_')))
        {
            return Err(Error::Parse {
                target: "fix namespace",
                position,
                reason:
                    "a fix namespace may contain only ASCII letters, digits, hyphen, dot, or underscore"
                        .into(),
            });
        }
        if value.len() > Self::MAX_LENGTH {
            return Err(Error::Parse {
                target: "fix namespace",
                position: Self::MAX_LENGTH,
                reason: format_smolstr!("a fix namespace is at most {} bytes", Self::MAX_LENGTH),
            });
        }
        if value.eq_ignore_ascii_case(Self::STANDARD.as_str()) {
            return Ok(Self::STANDARD);
        }
        if value.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            return Ok(Self(SmolStr::new(value)));
        }
        let mut folded = SmolStrBuilder::new();
        for byte in value.bytes() {
            folded.push(char::from(byte.to_ascii_lowercase()));
        }
        Ok(Self(folded.into()))
    }
}

impl Default for FixNamespace {
    /// The standard namespace, which is what an absent `fix:namespace` means.
    fn default() -> Self {
        Self::STANDARD
    }
}

impl AsRef<str> for FixNamespace {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for FixNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What separates a namespace from a tag in a rendered identifier.
const IDENTIFIER_SEPARATOR: char = ':';

/// What an identifier is, spelled once for every refusal.
const IDENTIFIER_SHAPE: &str = "a fix identifier is a namespace, a colon, and a decimal tag";

/// One FIX field's identity: the dictionary it belongs to and its tag.
///
/// Derived from `fix:namespace` and `fix:tag` on every read and never stored,
/// so the identity cannot drift from the two facts it is computed from. It
/// renders and parses as `namespace:tag` - `standard:35`, `cme:5001` - and
/// orders namespace-major, which is the order a registry iterates and a store
/// writes in.
///
/// ```
/// use yggdryl::{FixId, FixNamespace};
///
/// # fn main() -> yggdryl::Result<()> {
/// let id = FixId::from_str("CME:5001")?;
/// assert_eq!(id.to_string(), "cme:5001");
/// assert_eq!(id.tag(), 5001);
/// assert_eq!(FixId::standard(35).to_string(), "standard:35");
/// assert!(!id.is_standard());
///
/// // A tag the FIX specification assigns belongs to the standard namespace.
/// let refused = FixId::from_parts(FixNamespace::from_str("cme")?, 35).unwrap_err();
/// assert!(refused.to_string().contains("fix:namespace"), "{refused}");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixId {
    // Declared first so the derived `Ord` is namespace-major.
    namespace: FixNamespace,
    tag: i32,
}

impl FixId {
    /// The first tag the FIX specification does not assign itself.
    ///
    /// Tags 0-4999 are assigned by the FIX specification; 5000-9999 is its
    /// user-defined range and everything above is vendor space. Only the
    /// first of those three belongs to the standard namespace by rule.
    pub const STANDARD_TAG_LIMIT: i32 = 5_000;

    /// The identifier of `tag` in the standard namespace.
    ///
    /// Not a `const fn`: [`FixNamespace`] holds a `SmolStr`, which has a
    /// `Drop` impl, and nothing needs this in a const context.
    pub fn standard(tag: i32) -> Self {
        Self {
            namespace: FixNamespace::STANDARD,
            tag,
        }
    }

    /// Builds an identifier from its two parts, applying the standard-tag
    /// rule.
    ///
    /// This is the one place that rule lives: a tag below
    /// [`Self::STANDARD_TAG_LIMIT`] is the FIX specification's own, so it
    /// forces [`FixNamespace::STANDARD`]. The standard namespace itself holds
    /// any tag. Every producer of an identity reaches the rule through here
    /// and none re-checks it, which is what makes an inadmissible identifier
    /// unconstructible.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidMetadataValue`] naming `fix:namespace`, the
    /// limit and both sides when a specification tag is claimed by another
    /// dictionary.
    pub fn from_parts(namespace: FixNamespace, tag: i32) -> Result<Self> {
        if !Self::is_admissible(&namespace, tag) {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(field::NAMESPACE_KEY),
                reason: format_smolstr!(
                    "expected the standard namespace for tag {tag}, which the FIX specification assigns below {}, got {:?}",
                    Self::STANDARD_TAG_LIMIT,
                    namespace.as_str()
                ),
            });
        }
        Ok(Self { namespace, tag })
    }

    /// Parses `namespace:tag`.
    ///
    /// The namespace grammar forbids `:`, so the split is unambiguous, and
    /// the tag is decimal digits only - `+35`, `-35`, whitespace and an empty
    /// tail are all refused. A bare `35` is not an identifier. The parsed
    /// parts then go through [`Self::from_parts`], so text is held to the same
    /// rule as constructed parts.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the text has no
    /// colon or its tail is not a tag, [`FixNamespace::from_str`]'s failure
    /// for a bad head - whose positions are already this text's, because the
    /// namespace is its prefix - or [`Self::from_parts`]'s refusal.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Result<Self> {
        <Self as FromStr>::from_str(text)
    }

    /// Returns the dictionary this identifier belongs to.
    pub const fn namespace(&self) -> &FixNamespace {
        &self.namespace
    }

    /// Returns the tag.
    pub const fn tag(&self) -> i32 {
        self.tag
    }

    /// Returns whether this identifier is in the standard namespace.
    pub fn is_standard(&self) -> bool {
        self.namespace.is_standard()
    }

    /// The standard-tag rule as a predicate: [`Self::from_parts`] is this
    /// plus the refusal that names both sides.
    ///
    /// A caller that would discard the refusal asks this instead, because
    /// building the refusal's text allocates and the message tier consults it
    /// on every lookup. The rule itself still lives in exactly one place.
    pub(super) fn is_admissible(namespace: &FixNamespace, tag: i32) -> bool {
        tag >= Self::STANDARD_TAG_LIMIT || namespace.is_standard()
    }
}

impl FromStr for FixId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let Some((head, tail)) = text.split_once(IDENTIFIER_SEPARATOR) else {
            return Err(Error::Parse {
                target: "fix identifier",
                position: text.len(),
                reason: IDENTIFIER_SHAPE.into(),
            });
        };
        let namespace = FixNamespace::from_str(head)?;
        let tag = field::parse_tag(tail).ok_or(Error::Parse {
            target: "fix identifier",
            position: head.len() + IDENTIFIER_SEPARATOR.len_utf8(),
            reason: IDENTIFIER_SHAPE.into(),
        })?;
        Self::from_parts(namespace, tag)
    }
}

impl fmt::Display for FixId {
    /// Always `namespace:tag`, the standard namespace included.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}{IDENTIFIER_SEPARATOR}{}",
            self.namespace, self.tag
        )
    }
}

/// One of the three ways a caller names one FIX field.
///
/// [`From`] carries every spelling a caller reaches for, exactly as the key
/// of [`Field::get_field`](crate::Field::get_field) does, so
/// `registry.field(35)` and `registry.field("MsgType")` are one call rather
/// than two. A bare tag and a bare name are the standard namespace; a
/// colon-bearing string is a name, never an identifier, because a `From`
/// conversion cannot fail and a silent fallback to a name lookup would be two
/// behaviors under one spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixKey<'a> {
    /// A canonical or alternate tag in the standard namespace.
    Tag(i32),
    /// A canonical or alternate identity in any namespace.
    Id(&'a FixId),
    /// A canonical name, an alias, or a dotted path, in the standard
    /// namespace, matched with ASCII case folded.
    Name(&'a str),
}

impl From<i32> for FixKey<'_> {
    fn from(tag: i32) -> Self {
        Self::Tag(tag)
    }
}

impl<'a> From<&'a FixId> for FixKey<'a> {
    fn from(id: &'a FixId) -> Self {
        Self::Id(id)
    }
}

impl<'a> From<&'a str> for FixKey<'a> {
    fn from(name: &'a str) -> Self {
        Self::Name(name)
    }
}

impl<'a> From<&'a String> for FixKey<'a> {
    fn from(name: &'a String) -> Self {
        Self::Name(name.as_str())
    }
}

impl fmt::Display for FixKey<'_> {
    /// Renders the key the way an absence names it: `tag 35`,
    /// `identifier cme:5001`, `name "MsgType"`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(formatter, "tag {tag}"),
            Self::Id(id) => write!(formatter, "identifier {id}"),
            Self::Name(name) => write!(formatter, "name {name:?}"),
        }
    }
}
