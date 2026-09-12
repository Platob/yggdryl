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
//! | tag | `fix:tag` | `i32` | canonical FIX tag |
//! | branches | `fix:branches` | ordered name list | the dictionaries that contributed this field, folded and sorted; absent for a field the specification alone defines |
//! | tags | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
//! | aliases | `fix:aliases` | ordered name list | alternate names, highest priority first |
//! | description | `description` | text | the specification's own wording, on the key every catalog reads |
//! | lineage | `fix:lineage` | canonical JSON, oldest first | what this field was called and typed at each FIX version |
//! | codes | `fix:codes` | canonical JSON, by wire value | enumeration definitions owned by the field |
//! | replacements | `fix:replacements` | canonical JSON, in order | how a value of this field is restated at a later version: the fields it fills and the values they take |
//! | counter | `fix:counter` | `i32` | the wire field counting a group's occurrences |
//! | component | `fix:component` | name | the component defining a group occurrence |
//!
//! Scalar wire fields, messages, components and groups are
//! separate catalog categories. A component or message is a Struct Field;
//! a group is a List of a non-null component. Its `fix:counter` references a
//! separate int32 wire field: `NoPartyIDs` is tag 453, while `Parties` contains
//! `Party` values. Each field keeps its enumeration in `fix:codes` metadata.
//!
//! # Identity
//!
//! [`FixId`] is one `i32`: the digest of a field's tag and its folded name
//! together. It is derived on every read from `fix:tag` and the field's
//! name and never stored - there is no `fix:id` key on disk - because the
//! registry, the catalog and the store rename a field after it is built.
//! A dictionary is not a namespace: the fields a dialect contributed carry
//! its name in `fix:branches`, which a merge unions and resolution never
//! consults.
//!
//! # Resolution
//!
//! [`FixRegistry`] holds one namespace and answers a tag, an identity or a
//! name in one order: canonical before alternate for tags, canonical name
//! before alias for names, the fold always. A tag held by two fields under
//! different names answers the first holder; the other is reached by its
//! name or its identity. The hash indexes hold positions into one field
//! vector - tags and identities are their own keys, names and aliases
//! independent ASCII-folded XXH64 digests - and every digest hit is rechecked
//! against the field, so a collision is a miss on read and a typed conflict
//! on mutation. A separate sorted position vector makes iteration tag-major,
//! then by identity.
//!
//! # Versions
//!
//! The registry is version-agnostic: it holds every tag ever defined, and a
//! version is a filter on the read, which is what "defined in one version,
//! available in the others" means. [`FixField::lineage`](crate::FixField)
//! carries what a field was called and typed at each version; `since`,
//! `until` and deprecation derive from it rather than sit beside it, and
//! [`FixRegistry::field_at`] filters one read by it. There is no
//! registry-wide default version, and "FIX Latest" is never stored as one:
//! [`FixRegistry::newest`] resolves it to the real pedigree the dictionary
//! carries.
//!
//! The lineage carries enough to rename and retype a field between versions.
//! The expression-driven normalization layer - conditions, lookups and value
//! mappings - is not here and needs an evaluator; "transcoding" names both
//! and only the lineage-driven half lives in this module.
//!
//! Names fold once, on the way in - ASCII case, and the `_`, `-` and space
//! separators - so a query spelled in any case or with any separator finds
//! the field and the answer is always the canonical spelling. A tag
//! query never consults names and a name query never consults tags, and an
//! alias can never take a name away from a field that claims it canonically.
//! [`FixKey`] carries any of the three kinds of key through the generic
//! [`FixRegistry::get_field`] / [`FixRegistry::field`] pair, which match once
//! and redirect to the specialized accessor for that kind.
//!
//! # Storage
//!
//! One IOBase folder contains `fields`, `messages`, `components`, `groups`.
//! Scalar fields use `<tag / 100>.json` arrays; other categories use
//! `<name>.json` native Field documents.
//!
//! Referenced children persist as Null-typed native Fields carrying
//! `fix:field`, `fix:component` or `fix:group`. Loading resolves the graph
//! once into typed fields and rejects missing or cyclic references. Writing
//! compacts these references again.
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
//! use yggdryl::{DataType, FixId, FixRegistry};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut symbol = DataType::utf8().required_field("Symbol");
//! symbol.as_fix_mut().set_tag(55)?;
//! symbol.as_fix_mut().set_aliases(["Ticker"])?;
//!
//! let mut trade = DataType::utf8().required_field("TradeID");
//! trade.as_fix_mut().set_tag(5001)?;
//! trade.as_fix_mut().set_branches(["cme"])?;
//!
//! let registry = FixRegistry::from_fields([symbol, trade])?;
//! assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
//! assert_eq!(registry.field_by_name("ticker")?.name(), "Symbol");
//! assert_eq!(registry.field("SYMBOL")?.as_fix().id()?, Some(FixId::of(55, "Symbol")?));
//! // An identity is a tag and a name; membership says who contributed it.
//! assert_eq!(registry.field(FixId::of(5001, "TradeID")?)?.name(), "TradeID");
//! assert!(registry.field_by_tag(5001)?.as_fix().has_branch("CME"));
//! # Ok(())
//! # }
//! ```

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

mod anomaly;
// Batching is the crate's Arrow surface seen from FIX, so it exists exactly
// where that surface does.
#[cfg(feature = "arrow")]
mod batch;
mod build;
mod catalog;
mod cfb;
mod codec;
mod codes;
mod component;
mod constants;
mod crated;
mod digest;
mod document;
mod enrich;
mod entry;
mod field;
mod global;
mod group_plan;
mod latest;
mod lifecycle;
mod lift;
mod lineage;
mod memo;
mod messages;
mod msg;
mod msgtype;
mod registry;
mod replacements;
mod schema;
mod store;
#[cfg(test)]
mod tests;
mod ulbridge;

pub use anomaly::{FixAnomalies, FixAnomaly};
pub use codec::DEFAULT_PAYLOAD_COLUMN;
pub use codec::{DEFAULT_NULL_VALUES, FixCodec, SOH};
pub use codes::{FixCode, FixCodeValue, FixCodes};
pub(crate) use component::occurrence_name;
pub use constants::{STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS};
pub use crated::{
    CRATE_TAG_MAX, CRATE_TAG_MIN, DEFAULT_PARTITION_SECONDS, ID_TAG_NAME, INSTID_TAG_NAME,
    ISINCODE_TAG_NAME, MICCODE_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME,
    MSGHASH_TAG_NAME, MSGTYPE_TAG_NAME, PARENTCLORDID_TAG_NAME, PARENTORDERID_TAG_NAME,
    PERSISTENTID_TAG_NAME, PLUGINID_TAG_NAME, PREVPLUGINID_TAG_NAME, SENDERSESSIONID_TAG_NAME,
    SENDERSESSIONNAME_TAG_NAME, STATE_TAG_NAME, SYMBOLTICKER_TAG_NAME, TARGETSESSIONID_TAG_NAME,
    TARGETSESSIONNAME_TAG_NAME, TIMESTAMP_TAG_NAME, UNIXPARTITION_TAG_NAME, VERSION_TAG_NAME,
    fix_crate_fields, is_crate_tag,
};
pub use digest::FixDedup;
pub use document::{Numbers, Words};
pub use entry::FixEntry;
pub use field::FixSpellings;
pub use lifecycle::FixLifecycle;
pub use lift::{FixLift, FixParty, fix_lift, fix_lifts};
pub use lineage::{FixLineage, FixLineageEntry, FixPedigree};
pub use messages::FixMessages;
pub use msg::FixMsg;
pub use msgtype::MsgType;
pub use registry::{FixFieldIter, FixRegistry};
pub use replacements::{
    FixFill, FixFillEntry, FixFillSource, FixFillValue, FixFills, FixReplacement,
    FixReplacementEntry, FixReplacements,
};
pub use ulbridge::{
    ERROR_TAG_NAME, MBEAN_TAG_NAME, OPERATION_TAG_NAME, STATUS_TAG_NAME, ULBRIDGE_DIALECT,
    ULBRIDGE_ROWHEADER, ULBRIDGE_TAG_MIN, UlPlugin, UlPlugins, fix_ulbridge_fields,
};

pub use schema::{
    BODY_TAGS, ENTRIES_COLUMN, GROUP_TAGS, HEADER_TAGS, TRAILER_TAGS, UNMAPPED_COLUMN,
    fix_column_of, fix_column_tags, fix_schema, fix_schema_carrying, fix_schema_tags,
};

/// A digest as everything outside this crate holds it.
///
/// The XXH32 is a `u32` and every carrier of it is an `i32`, which is the
/// same four bytes read as signed: the digest's exact width, and the widest
/// signed integer every exchange format this crate writes can hold, Avro
/// having no unsigned one. A digest above `i32::MAX` therefore reads
/// negative, and reads back as itself.
#[allow(clippy::cast_possible_wrap)]
pub(super) const fn signed(digest: u32) -> i32 {
    digest as i32
}

/// The seed every identity is digested under.
///
/// Fixed and non-zero, so a bare tag's four bytes under an empty name never
/// land on the digest a derived definition tag takes from a name alone.
const IDENTITY_SEED: u32 = 0x5947_4649;

/// One FIX field's identity: its tag and its name, digested into one `i32`.
///
/// A field is identified by its tag and its name together, and by nothing
/// else. The digest is the XXH32 of the tag's four little-endian bytes
/// followed by the name under the crate's one fold - ASCII case dropped,
/// and `_`, `-` and space dropped - so `Msg_Type`, `msgtype` and `MsgType`
/// under tag 35 are one identity, and the identity agrees with what every
/// name lookup already answers. It is derived on every read from `fix:tag`
/// and the field's name and never stored: the registry, the catalog and the
/// store rename a field after it is built, and a stored identity would go
/// stale where a derived one cannot.
///
/// One integer, `Copy`, its own hash key, rendered as its decimal wherever
/// it crosses a boundary. It is a newtype and not an alias so that an
/// integer never has two readings: [`FixKey::from`] on an `i32` is a tag,
/// and an identity is always spelled [`FixKey::Id`].
///
/// ```
/// use yggdryl::FixId;
///
/// # fn main() -> yggdryl::Result<()> {
/// let id = FixId::of(35, "MsgType")?;
/// assert_eq!(id, FixId::of(35, "msg_type")?);
/// assert_ne!(id, FixId::of(35, "MsgSeqNum")?);
/// assert_ne!(id, FixId::of(36, "MsgType")?);
/// assert_eq!(id.to_string(), id.digest().to_string());
/// assert!(FixId::of(-1, "MsgType").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct FixId(i32);

const _: () = assert!(size_of::<FixId>() == size_of::<i32>());

impl FixId {
    /// The first tag a derived definition identity takes.
    ///
    /// Components, groups and messages are named rather than tagged on the
    /// wire, so nothing publishes a tag for them. This block is where the one
    /// they are given is derived, clear of every published tag and above
    /// [`crate::CRATE_TAG_MAX`], which is the last block anything else claims.
    pub const DEFINITION_TAG_MIN: i32 = 100_000;

    /// One past the last tag a derived definition identity takes.
    pub const DEFINITION_TAG_MAX: i32 = 1_100_000;

    /// Whether a tag is a derived definition identity.
    #[must_use]
    pub const fn is_definition_tag(tag: i32) -> bool {
        tag >= Self::DEFINITION_TAG_MIN && tag < Self::DEFINITION_TAG_MAX
    }

    /// The identity of one tag under one name.
    ///
    /// # Errors
    ///
    /// Returns a typed failure naming `fix:tag` for a negative tag, which
    /// no FIX field has.
    pub fn of(tag: i32, name: &str) -> Result<Self> {
        if tag < 0 {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(field::TAG_KEY),
                reason: format_smolstr!("expected a non-negative FIX tag, got {tag}"),
            });
        }
        let mut state = crate::xxhash::Xxh32::with_seed(IDENTITY_SEED);
        state.write_bytes(&tag.to_le_bytes());
        let mut folded = [0_u8; 64];
        let mut held = 0;
        for byte in name.as_bytes() {
            if matches!(byte, b'_' | b'-' | b' ') {
                continue;
            }
            folded[held] = byte.to_ascii_lowercase();
            held += 1;
            if held == folded.len() {
                state.write_bytes(&folded);
                held = 0;
            }
        }
        state.write_bytes(&folded[..held]);
        Ok(Self(signed(state.as_u32())))
    }

    /// The identity as the integer every carrier holds.
    #[must_use]
    pub const fn digest(self) -> i32 {
        self.0
    }

    /// The identity one carried integer names.
    ///
    /// The inverse of [`digest`](Self::digest), for a value read back from a
    /// column or a caller: nothing is checked, because an integer read back
    /// is whatever was written, and a registry lookup is what says whether a
    /// field stands behind it.
    #[must_use]
    pub const fn from_digest(digest: i32) -> Self {
        Self(digest)
    }
}

impl fmt::Display for FixId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// One of the three ways a caller names one FIX field.
///
/// [`From`] carries every spelling a caller reaches for, exactly as the key
/// of [`Field::get_field`](crate::Field::get_field) does, so
/// `registry.field(35)` and `registry.field("MsgType")` are one call rather
/// than two. An integer is a tag and never an identity, because a `From`
/// conversion cannot fail and one integer must not have two readings; an
/// identity is always spelled [`FixKey::Id`]. A colon-bearing string is a
/// name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixKey<'a> {
    /// A canonical or alternate tag.
    Tag(i32),
    /// One field's identity: its tag and its name together.
    Id(FixId),
    /// A canonical name, an alias, or a dotted path, matched with ASCII
    /// case and separators folded.
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
    /// `identifier -1873404312`, `name "MsgType"`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(formatter, "tag {tag}"),
            Self::Id(id) => write!(formatter, "identifier {id}"),
            Self::Name(name) => write!(formatter, "name {name:?}"),
        }
    }
}
