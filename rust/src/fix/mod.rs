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
//! | branch | `fix:branch` | [`FixBranch`] | the dictionary this field belongs to; absent is [`FixBranch::STANDARD`] |
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
//! [`FixId`] packs one tag and one branch digest into an `i64`. It is derived
//! on every read from `fix:branch` and `fix:tag` and never stored: there is no
//! `fix:id` key on disk. [`FixId::from_parts`] is the one admissibility gate:
//! a non-standard branch may claim only [`FixId::USER_TAG_MIN`] through
//! [`FixId::USER_TAG_MAX`] (exclusive), while [`FixBranch::STANDARD`] may
//! hold every non-negative tag.
//!
//! # Resolution
//!
//! [`FixRegistry`] answers an identifier or a name through two tiers within
//! one branch, and a later tier is consulted only when every earlier one
//! missed:
//!
//! 1. canonical identifier, then alternate identifiers;
//! 2. canonical name folded, then aliases folded.
//!
//! The four hash indexes hold positions into one field vector: identifiers
//! are their packed keys, while canonical names and aliases use independent
//! seeded, ASCII-folded XXH64 digests. Every digest hit is rechecked against
//! the field, so a collision is a miss on read and a typed conflict on
//! mutation. A separate sorted position vector makes iteration tag-major.
//!
//! Names fold ASCII case once, on the way in, so a query spelled in any case
//! finds the field and the answer is always the canonical spelling. A tag
//! query never consults names and a name query never consults tags, and an
//! alias can never take a name away from a field that claims it canonically.
//! An explicit branch pins one dictionary. When omitted, resolution tries the
//! standard dictionary first and then named dictionaries in canonical name
//! order, returning the first match. [`FixKey`] carries any of the three kinds
//! of key through the generic [`FixRegistry::get_field`] /
//! [`FixRegistry::field`] pair, which match once and redirect to the
//! specialized accessor for that kind.
//!
//! # Storage
//!
//! A registry reads and writes through one [`IOBase`](crate::IOBase)
//! folder handle, into two trees plus one branch manifest:
//!
//! ```text
//! <root>/primitive/<shard>.json
//! <root>/primitive/<branch>/<shard>.json
//! <root>/nested/<shard>.json
//! <root>/nested/<branch>/<shard>.json
//! <root>/branches.json
//! ```
//!
//! `primitive` holds the fields whose datatype is one scalar value and
//! `nested` the ones whose datatype carries a subtree - in FIX terms a
//! component, which is a Struct, and a repeating group, which is a List of
//! that Struct. `shard = tag / 100` is unchanged inside each tree, each shard
//! a JSON array of the core field document ordered by canonical identifier,
//! so a tag reaches exactly one shard by arithmetic and an alternate tag
//! never fans a field across shards. `branches.json` records optional dialect,
//! extension-pack and session facts in canonical branch-name order; its
//! absence means bare branch records.
//!
//! Every shard of both trees is loaded on open:
//! [`FixRegistry::from_handle`] reads standard shards directly under each
//! tree, then each named branch folder, because a name has no numeric
//! structure to pick a shard with and a dictionary is small enough that
//! loading it whole costs less than the machinery of loading it lazily. Both trees are
//! optional: a dictionary of only scalars writes no `nested/` at all and a
//! root holding neither loads as the empty registry, which is the laziness
//! contract of every handle. A non-shard leaf is ignored; a field whose
//! datatype contradicts its tree and a shard that exists but does not parse
//! are typed errors naming their URL.
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
//! use yggdryl::{DataType, FixBranch, FixId, FixRegistry};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut symbol = DataType::Utf8.required_field("Symbol");
//! symbol.as_fix_mut().set_tag(55)?;
//! symbol.as_fix_mut().set_aliases(["Ticker"])?;
//!
//! let cme = FixBranch::from_str("cme")?;
//! let mut trade = DataType::Utf8.required_field("TradeID");
//! trade.as_fix_mut().set_id(&cme, 5001)?;
//!
//! let registry = FixRegistry::from_fields([symbol, trade])?;
//! assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
//! assert_eq!(registry.field_by_name("ticker", None)?.name(), "Symbol");
//! assert_eq!(registry.field("SYMBOL")?.as_fix().id()?, Some(FixId::standard(55)));
//! // An identifier pins its branch; an omitted branch infers the best match.
//! assert_eq!(registry.field(FixId::from_parts(&cme, 5001)?)?.name(), "TradeID");
//! assert_eq!(registry.field_by_tag(5001)?.name(), "TradeID");
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, SmolStrBuilder, format_smolstr};

use crate::{Error, Result, Version};

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

/// The absent branch occupies four zero bytes in every standard identifier.
const STANDARD_BRANCH_DIGEST: u32 = 0;

/// The dictionary one FIX field belongs to.
///
/// A branch separates the FIX specification's own fields from a venue's:
/// absence is the specification, and any spelling names another dictionary
/// that defines its own tags and names beside it. The type exists rather than
/// a bare string because it enforces what a string cannot - a leading ASCII
/// letter, a grammar with no `:` or `,` to confuse an identifier or a list,
/// ASCII case folded exactly once on the way in, and an inline name - so `CME`
/// and `cme` are one branch and a registry probe carrying one allocates nothing.
///
/// ```
/// use yggdryl::FixBranch;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(FixBranch::from_str("CME")?.name(), "cme");
/// assert_eq!(FixBranch::default(), FixBranch::STANDARD);
/// assert!(FixBranch::from_str("")?.is_standard());
/// assert!(!FixBranch::from_str("std")?.is_standard());
/// assert!(FixBranch::from_str("2cme").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixBranch {
    // Identity comes first. The digest is a pure cache of the name and every
    // other member describes that dictionary rather than participating in
    // the packed field identifier.
    name: SmolStr,
    digest: u32,
    version: Version,
    target_comp_id: SmolStr,
    sender_comp_id: SmolStr,
}

impl FixBranch {
    /// The FIX specification's own dictionary, and what an absent
    /// `fix:branch` means.
    pub const STANDARD: Self = Self {
        name: SmolStr::new_static(""),
        digest: STANDARD_BRANCH_DIGEST,
        version: Version::MIN,
        target_comp_id: SmolStr::new_static(""),
        sender_comp_id: SmolStr::new_static(""),
    };

    /// The longest a branch may be, in bytes.
    ///
    /// This is `smol_str`'s inline capacity, which keeps the identity used by
    /// registry probes off the heap. Raising it would allocate branch names on
    /// the hot lookup path.
    pub const MAX_LENGTH: usize = 23;

    /// Parses and validates a branch, folding ASCII case.
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

    /// Builds a complete branch, validating and folding `name` once.
    pub fn from_parts(
        name: &str,
        version: Version,
        target_comp_id: impl Into<SmolStr>,
        sender_comp_id: impl Into<SmolStr>,
    ) -> Result<Self> {
        let mut branch = Self::from_str(name)?;
        branch.version = version;
        branch.target_comp_id = target_comp_id.into();
        branch.sender_comp_id = sender_comp_id.into();
        Ok(branch)
    }

    /// Returns the canonical lowercase name without allocating.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns the dialect's default FIX version.
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Returns the session target as declared.
    pub fn target_comp_id(&self) -> &str {
        self.target_comp_id.as_str()
    }

    /// Returns the session sender as declared.
    pub fn sender_comp_id(&self) -> &str {
        self.sender_comp_id.as_str()
    }

    /// Returns whether this is the FIX specification's own dictionary.
    pub fn is_standard(&self) -> bool {
        self.name.is_empty()
    }

    /// The cached XXH32 identity of the canonical spelling.
    pub const fn digest(&self) -> u32 {
        self.digest
    }

    /// Returns whether two values name the same dictionary.
    pub(super) fn has_identity(&self, other: &Self) -> bool {
        self.digest == other.digest && self.name == other.name
    }
}

impl FromStr for FixBranch {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.is_empty() {
            return Ok(Self::STANDARD);
        }
        let bytes = value.as_bytes();
        if bytes.first().is_none_or(|byte| !byte.is_ascii_alphabetic()) {
            return Err(Error::Parse {
                target: "fix branch",
                position: 0,
                reason: "a fix branch must start with an ASCII letter".into(),
            });
        }
        if let Some(position) = bytes
            .iter()
            .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_')))
        {
            return Err(Error::Parse {
                target: "fix branch",
                position,
                reason:
                    "a fix branch may contain only ASCII letters, digits, hyphen, dot, or underscore"
                        .into(),
            });
        }
        if value.len() > Self::MAX_LENGTH {
            return Err(Error::Parse {
                target: "fix branch",
                position: Self::MAX_LENGTH,
                reason: format_smolstr!("a fix branch is at most {} bytes", Self::MAX_LENGTH),
            });
        }
        let name = if value.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            SmolStr::new(value)
        } else {
            let mut folded = SmolStrBuilder::new();
            for byte in value.bytes() {
                folded.push(char::from(byte.to_ascii_lowercase()));
            }
            folded.into()
        };
        let digest = crate::xxhash::xxh32(name.as_bytes());
        Ok(Self {
            name,
            digest,
            version: Version::default(),
            target_comp_id: SmolStr::default(),
            sender_comp_id: SmolStr::default(),
        })
    }
}

impl Default for FixBranch {
    /// The standard branch, which is what an absent `fix:branch` means.
    fn default() -> Self {
        Self::STANDARD
    }
}

impl AsRef<str> for FixBranch {
    fn as_ref(&self) -> &str {
        self.name()
    }
}

impl fmt::Display for FixBranch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// What separates a tag from a branch in a rendered identifier.
const IDENTIFIER_SEPARATOR: char = ':';

/// What an identifier is, spelled once for every refusal.
const IDENTIFIER_SHAPE: &str = "a fix identifier is a decimal tag, a colon, and a branch";

/// One FIX field's packed tag and branch digest.
///
/// Derived from `fix:branch` and `fix:tag` on every read and never stored,
/// so changing this representation changes no shard. The tag occupies the
/// high 32 bits and the branch's XXH32 digest the low 32 bits. Consequently
/// the value is its own compact hash key and its natural order is tag-major.
///
/// ```
/// use yggdryl::{FixBranch, FixId};
///
/// # fn main() -> yggdryl::Result<()> {
/// let branch = FixBranch::from_str("CME")?;
/// let id = FixId::from_parts(&branch, 5001)?;
/// assert!(id.to_string().ends_with(&format!("#{:08x}", id.branch_digest())));
/// assert_eq!(id.tag(), 5001);
/// assert_eq!(FixId::standard(35).to_string(), "35:");
/// assert!(!id.is_standard());
///
/// // A tag the FIX specification assigns belongs to the standard branch.
/// let refused = FixId::from_parts(&branch, 35).unwrap_err();
/// assert!(refused.to_string().contains("fix:branch"), "{refused}");
/// # Ok(())
/// # }
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixId(i64);

const _: () = assert!(size_of::<FixId>() == size_of::<i64>());

impl FixId {
    /// The inclusive lower bound of FIX's user-defined tag range.
    pub const USER_TAG_MIN: i32 = 5_000;

    /// The exclusive upper bound of FIX's user-defined tag range.
    pub const USER_TAG_MAX: i32 = 40_000;

    /// The identifier of `tag` in the standard branch.
    pub const fn standard(tag: i32) -> Self {
        Self::pack(STANDARD_BRANCH_DIGEST, tag)
    }

    /// Builds an identifier from a branch and tag.
    ///
    /// This is the one place the admissibility rule lives: only the standard
    /// branch may claim tags outside [`Self::USER_TAG_MIN`] through
    /// [`Self::USER_TAG_MAX`]. The standard branch itself holds every valid
    /// tag. A non-standard spelling whose digest collides with zero is also
    /// refused so [`Self::is_standard`] remains total.
    ///
    /// # Errors
    ///
    /// Returns a typed failure naming `fix:branch`, both bounds and the tag
    /// when a specification tag is claimed by another dictionary, or both
    /// spellings and the digest when a branch collides with the standard value.
    pub fn from_parts(branch: &FixBranch, tag: i32) -> Result<Self> {
        if tag < 0 {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(field::TAG_KEY),
                reason: format_smolstr!("expected a non-negative FIX tag, got {tag}"),
            });
        }
        if !branch.is_standard() && branch.digest() == FixBranch::STANDARD.digest() {
            return Err(Error::conflict(
                "FIX branch",
                "FIX branch",
                format_smolstr!(
                    "branches {:?} and {:?} have digest #{:08x}",
                    FixBranch::STANDARD.name(),
                    branch.name(),
                    branch.digest()
                ),
            ));
        }
        if !Self::is_admissible(branch, tag) {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(field::BRANCH_KEY),
                reason: format_smolstr!(
                    "expected the standard branch outside the user-defined tag range [{}, {}), got branch {:?} at tag {tag}",
                    Self::USER_TAG_MIN,
                    Self::USER_TAG_MAX,
                    branch.name()
                ),
            });
        }
        Ok(Self::pack(branch.digest(), tag))
    }

    /// Parses `tag:branch`.
    ///
    /// The branch grammar forbids `:`, so the split is unambiguous, and the
    /// tag is decimal digits only. The branch text is intentionally consumed:
    /// a bare packed id cannot recover it from a one-way digest.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for malformed text, [`FixBranch::from_str`]'s
    /// failure for a bad branch, or [`Self::from_parts`]'s refusal.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Result<Self> {
        <Self as FromStr>::from_str(text)
    }

    /// Returns the tag.
    pub const fn tag(self) -> i32 {
        (self.0 >> 32) as i32
    }

    /// Returns the cached XXH32 digest identifying the dictionary branch.
    pub const fn branch_digest(self) -> u32 {
        self.0 as u32
    }

    /// Returns whether this identifier is in the standard branch.
    pub const fn is_standard(self) -> bool {
        self.branch_digest() == STANDARD_BRANCH_DIGEST
    }

    /// The branch/tag admissibility rule without constructing its refusal.
    pub(super) fn is_admissible(branch: &FixBranch, tag: i32) -> bool {
        branch.is_standard() || (Self::USER_TAG_MIN..Self::USER_TAG_MAX).contains(&tag)
    }

    const fn pack(branch_digest: u32, tag: i32) -> Self {
        Self((tag as i64) << 32 | branch_digest as i64)
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
        let tag = field::parse_tag(head).ok_or(Error::Parse {
            target: "fix identifier",
            position: 0,
            reason: IDENTIFIER_SHAPE.into(),
        })?;
        let branch = FixBranch::from_str(tail).map_err(|error| match error {
            Error::Parse {
                target,
                position,
                reason,
            } => Error::Parse {
                target,
                position: head.len() + IDENTIFIER_SEPARATOR.len_utf8() + position,
                reason,
            },
            other => other,
        })?;
        Self::from_parts(&branch, tag)
    }
}

impl fmt::Display for FixId {
    /// Renders the tag first, then the standard branch or another branch's digest.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_standard() {
            write!(formatter, "{}{IDENTIFIER_SEPARATOR}", self.tag())
        } else {
            write!(
                formatter,
                "{}{IDENTIFIER_SEPARATOR}#{:08x}",
                self.tag(),
                self.branch_digest()
            )
        }
    }
}

/// One of the three ways a caller names one FIX field.
///
/// [`From`] carries every spelling a caller reaches for, exactly as the key
/// of [`Field::get_field`](crate::Field::get_field) does, so
/// `registry.field(35)` and `registry.field("MsgType")` are one call rather
/// than two. A bare tag or name uses the registry's best-match order; a
/// colon-bearing string is a name, never an identifier, because a `From`
/// conversion cannot fail and a silent fallback to a name lookup would be two
/// behaviors under one spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixKey<'a> {
    /// A canonical or alternate tag in the standard branch.
    Tag(i32),
    /// A canonical or alternate identity in any branch.
    Id(FixId),
    /// A canonical name, an alias, or a dotted path, in the standard
    /// branch, matched with ASCII case folded.
    Name(&'a str),
}

impl From<i32> for FixKey<'_> {
    fn from(tag: i32) -> Self {
        Self::Tag(tag)
    }
}

impl From<FixId> for FixKey<'_> {
    fn from(id: FixId) -> Self {
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
    /// `identifier 5001:cme`, `name "MsgType"`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(formatter, "tag {tag}"),
            Self::Id(id) => write!(formatter, "identifier {id}"),
            Self::Name(name) => write!(formatter, "name {name:?}"),
        }
    }
}
