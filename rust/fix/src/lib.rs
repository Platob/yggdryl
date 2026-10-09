//! FIX field definitions: the `FIX:` vocabulary, a registry that resolves
//! them, the shards it persists to, the process-wide default, and the
//! message value that is typed against one.
//!
//! A FIX field is a [`Field`](yggdryl::Field) whose metadata carries the `FIX:`
//! namespace, read and written through [`FixField`] and
//! [`FixFieldMut`] - nobody spells `FIX:` at a call site.
//! The canonical name is the field's own `name()`, the datatype its own
//! `dtype()`, and the display name the generic `display` key; the namespace
//! adds only what FIX states beyond a field:
//!
//! | property | key | type | meaning |
//! | --- | --- | --- | --- |
//! | tag | `FIX:tag` | `i32` | canonical FIX tag |
//! | sources | `FIX:sources` | JSON array of ids | the sources that contributed this field - each the id of an entry of the registry's [sources catalog](FixRegistry::sources), which `sources.json` holds in a store - folded and sorted; absent for a field the specification alone defines |
//! | tags | `FIX:tags` | JSON array of `i32` | alternate tags, highest priority first |
//! | names | `FIX:names` | JSON array of names | alternate names, highest priority first |
//! | identifiers | `FIX:identifiers` | ordered member name list | a component's direct scalar identifiers, in declaration order |
//! | description | `description` | text | the specification's own wording, on the key every catalog reads |
//! | code set | `FIX:codeset` | name | the registry-owned vocabulary this field reads by |
//! | directions | `FIX:directions` | canonical JSON, in stated order | on tag 385: per code of the set, the `regex::bytes` patterns that name it from the prose in front of a payload; absent reads by the built-in defaults |
//! | counter | `FIX:counter` | `i32` | on a group: the NumInGroup tag framing it on the wire, written from the group's length and never a field beside it |
//! | component | `FIX:component` | name | the component defining a group occurrence |
//!
//! The categories are scalar wire fields, components and groups. A message
//! is a component carrying `FIX:msgtype`. Serie groups hold non-null Struct
//! occurrences, and their length is their count: `Parties` contains `Party`
//! values and is framed on the wire by `NoPartyIDs(453)`, a dictionary field
//! no component, message or row lists beside the group. A crate-owned Map
//! group holds its native entries under its own counter. The registry keeps each enumeration once; fields name it through
//! `FIX:codeset`.
//!
//! # Identity
//!
//! [`FixId`] is one `i32`: the digest of a field's tag and its folded name
//! together. It is derived on every read from `FIX:tag` and the field's
//! name and never stored - there is no `FIX:id` key on disk - because the
//! registry, the catalog and the store rename a field after it is built.
//! A dictionary is not a namespace: the fields a source contributed carry
//! its id in `FIX:sources`, which a merge unions and resolution never
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
//! The registry is version-blind: it holds every tag ever defined and filters
//! by none. A dictionary is one reading of the protocol rather than a history
//! of it, so a field is the field, under the one name and datatype the
//! dictionary gives it. A spelling an earlier version used reaches it as one
//! of its alternate [names](crate::FixField::names), stated by whoever built the
//! dictionary; nothing here derives one from a date.
//!
//! What a *value* was is the field's own business and stays: a
//! [code](FixCode) an older version declared is a code of the set like any
//! other. What the specification retired, and what stands in for it, is the
//! crate's own table, which every [parse](FixCodec::parse_line) applies; so
//! are the fields a message implies but did not carry. A registry carries
//! neither kind of rule: it holds the fields they read and write, and the
//! rules are the crate's, the same for every dictionary.
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
//! One IOBase folder contains `fields`, `components`, `groups`; a message is a
//! component carrying `FIX:msgtype`.
//! Scalar fields use `<tag / 100>.json` arrays; other categories use
//! `<name>.json` native Field documents.
//!
//! Referenced children persist as Null-typed native Fields carrying
//! `FIX:field`, `FIX:component` or `FIX:group`. Loading resolves the graph
//! once into typed fields and rejects missing or cyclic references. Writing
//! compacts these references again.
//!
//! # The process default
//!
//! [`FixRegistry::from_env`] is the registry every caller gets when it does not
//! name one, resolved once on the first call and never before. The order is
//! fixed: a registry installed by [`FixRegistry::install_env`], then the
//! folder `YGGDRYL_FIX_REGISTRY` names, then `~/.config/fix`, then the empty
//! registry - and only the third step treats absence as empty, because a
//! machine with no dictionary installed is an ordinary first-run state.
//! Every other failure is loud.
//!
//! ```
//! use yggdryl::DataType;
//! use yggdryl_fix::{FixField, FixFieldMut, FixId, FixRegistry};
//!
//! # fn main() -> yggdryl::Result<()> {
//! #     yggdryl_fix::install().unwrap();
//! let mut symbol = DataType::utf8().required_field("Symbol");
//! FixFieldMut::new(&mut symbol).set_tag(55)?;
//! FixFieldMut::new(&mut symbol).set_names(["Ticker"])?;
//!
//! let mut trade = DataType::utf8().required_field("TradeID");
//! FixFieldMut::new(&mut trade).set_tag(5001)?;
//! FixFieldMut::new(&mut trade).set_sources(["cme"])?;
//!
//! let registry = FixRegistry::from_fields([symbol, trade])?;
//! assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
//! assert_eq!(registry.field_by_name("ticker")?.name(), "Symbol");
//! assert_eq!(FixField::new(registry.field("SYMBOL")?).id()?, Some(FixId::of(55, "Symbol")?));
//! // An identity is a tag and a name; membership says who contributed it.
//! assert_eq!(registry.field(FixId::of(5001, "TradeID")?)?.name(), "TradeID");
//! assert!(FixField::new(registry.field_by_tag(5001)?).has_source("CME"));
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use yggdryl::{DataType, Error, Result, TimeUnit, Timezone};

mod aliases;
mod anomaly;
mod cfi;
// Batching is the crate's Arrow surface seen from FIX, so it exists exactly
// where that surface does.
mod batch;
mod build;
pub(crate) mod catalog;
mod cfb;
pub(crate) mod codec;
pub(crate) mod codes;
pub(crate) mod component;
mod constants;
mod crated;
mod digest;
mod direction;
mod directions;
pub(crate) mod document;
pub(crate) mod enrich;
mod entry;
mod field;
mod fix_category;
pub(crate) mod forex;
pub(crate) mod global;
pub(crate) mod group_plan;
pub(crate) mod identity;
mod idmap;
mod latest;
mod market;
pub(crate) mod memo;
mod messages;
mod msg;
pub(crate) mod msgtype;
mod native_derivations;
pub(crate) mod registry;
pub(crate) mod retired;
pub(crate) mod schema;
mod source;
pub mod state;
pub(crate) mod store;
mod ulbridge;

pub use anomaly::FixAnomaly;
pub use codec::DEFAULT_PAYLOAD_COLUMN;
pub use codec::{DEFAULT_NULL_VALUES, DEFAULT_REFUSED_MSGTYPES, FixCodec, SOH};
pub use codes::{FixCode, FixCodeSet, FixCodeValue, FixCodes};
pub(crate) use component::occurrence_name;
pub use constants::{STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS};
pub use crated::{
    ASKCCY_TAG_NAME, ASKPX_TAG_NAME, ASKQTY_TAG_NAME, BIDCCY_TAG_NAME, BIDQTY_TAG_NAME,
    BLOOMBERGCODE_TAG_NAME, CONVERSATIONID_TAG_NAME, CRATE_TAG_MAX, CRATE_TAG_MIN,
    CREAUNIX_TAG_NAME, CROSSCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME, CROSSUUID_TAG_NAME,
    EXECUNIX_TAG_NAME, EXPRUNIX_TAG_NAME, FIGICODE_TAG_NAME, FIXMSG_TAG_NAME, FOREXCODE_TAG_NAME,
    FORWARDPOINTS_TAG_NAME, FXRATES_TAG_NAME, HASHCODE_TAG_NAME, HIDDENQTY_TAG_NAME,
    IDENTIFIERS_TAG_NAME, ISINCODE_TAG_NAME, MARKETDATAKIND_TAG_NAME, MARKETDATATYPE_TAG_NAME,
    METADATA_TAG_NAME, MICCODE_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME,
    MSGORIGINATOR_TAG_NAME, MSGPLUGINID_TAG_NAME, MSGPLUGINSIDE_TAG_NAME, MSGSESSEVENTID_TAG_NAME,
    MSGSESSIONID_TAG_NAME, MSGTYPE_TAG_NAME, ORDQTY_TAG_NAME, ORIGCCY_TAG_NAME, PARTYIDS_TAG_NAME,
    PREVPX_TAG_NAME, PREVQTY_TAG_NAME, PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME, SECURITYIDS_TAG_NAME,
    SENDUNIX_TAG_NAME, SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME, SOURCEURL_TAG_NAME, SPOTRATE_TAG_NAME,
    SRCUUIDS_TAG_NAME, STATE_TAG_NAME, STRIKEPX_TAG_NAME, TICKER_TAG_NAME, TRADABLE_TAG_NAME,
    TRANSUNIX_TAG_NAME, UNIT_TAG_NAME, UUID_TAG_NAME, fix_crate_fields, is_crate_tag,
    is_derived_tag,
};
pub use digest::FixDedup;
pub use direction::{MsgDirection, RECEIVE_PATTERNS, SEND_PATTERNS};
pub use directions::{FixDirection, FixDirectionEntry, FixDirections, FixPatterns};
pub use document::{Words, from_fix_document, into_fix_document};
pub use entry::FixEntry;
pub use field::{FixField, FixFieldMut, FixSpellings};
pub use identity::{FIX_TYPED_TAGS, FixCapture, FixHeader, FixLifted, LiftedFx};
pub use idmap::{FixIdMapKind, FixIdSource, FixIdSources};
pub use market::FixMarketIterator;
pub use messages::FixMessages;
pub use msg::FixMsg;
pub use msgtype::MsgType;
pub use registry::{FixDrop, FixFailure, FixFieldIter, FixMerge, FixRegistry};
pub use source::{FixSource, plugin_side};
pub use store::FixCommit;
pub use ulbridge::ULBRIDGE_ROWHEADER;

pub use fix_category::FixCategory;
pub use schema::{
    BODY_TAGS, FIXENTRIES_COLUMN, GROUP_TAGS, HEADER_TAGS, TRAILER_TAGS, fix_column_of,
    fix_column_tags, fix_schema, fix_schema_carrying, fix_schema_tags,
};

/// A digest as everything outside this crate holds it.
///
/// The XXH32 is a `u32` and every carrier of it is an `i32`, which is the
/// same four bytes read as signed: the digest's exact width, and the widest
/// signed integer every exchange format this crate writes can hold, Avro
/// having no unsigned one. A digest above `i32::MAX` therefore reads
/// negative, and reads back as itself.
#[allow(clippy::cast_possible_wrap)]
pub(crate) const fn signed(digest: u32) -> i32 {
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
/// name lookup already answers. It is derived on every read from `FIX:tag`
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
/// use yggdryl_fix::FixId;
///
/// # fn main() -> yggdryl::Result<()> {
/// #     yggdryl_fix::install().unwrap();
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
    /// Returns a typed failure naming `FIX:tag` for a nonpositive tag.
    /// Zero belongs only to unresolved arrival entries, never a definition.
    pub fn of(tag: i32, name: &str) -> Result<Self> {
        if tag <= 0 {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(field::TAG_KEY),
                reason: format_smolstr!("expected a positive FIX tag, got {tag}"),
            });
        }
        let mut state = yggdryl::xxhash::Xxh32::with_seed(IDENTITY_SEED);
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
/// of [`Field::get_field`](yggdryl::Field::get_field) does, so
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

impl<'a> FixKey<'a> {
    /// The key a text a caller typed names: a tag when it spells one the way
    /// the dictionary's own tag reader reads it - ASCII digits alone, `1` to
    /// `2147483647`, a leading zero kept, so `035` is tag `35` - and a name
    /// otherwise, `+35`, `0` and ` 35` among them. Total, like [`From`], so
    /// one text has one reading.
    #[must_use]
    pub fn from_text(text: &'a str) -> Self {
        match field::parse_tag(text) {
            Some(tag) => Self::Tag(tag),
            None => Self::Name(text),
        }
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

/// The FIX Latest datatype names, each over the closest core datatype: a
/// name, never a type, so `price` parses to `float64` and displays as
/// `float64`. The core's seed claims them beside its own names until the FIX
/// crate claims them itself, and `DataType::logical_names` lists them with
/// the rest, in name order.
///
/// The names are stored in their normalized spelling - lowercase, with no
/// `_`, `-`, or space - which is the form `DataType::from_logical_name`
/// folds a caller's spelling into, so `UTCTimestamp`, `utc_timestamp`, and
/// `UTC Timestamp` are one name.
///
/// Five FIX base types already have a meaning in the Arrow/SQL grammar and
/// keep it, because a schema string must not change meaning under a reader:
/// `int` is `int32`, `float` is `float32`, `char` and `string` are `utf8`,
/// and `boolean` is `boolean`. The FIX types derived from `int` and `float`
/// do get names here, and those carry the precision the base type does not.
///
/// | FIX | base | resolves to | why |
/// | --- | --- | --- | --- |
/// | `Language` | String | `fixed_ascii(2)` | ISO 639-1 alpha-2 |
/// | `MonthYear` | String | `fixed_ascii(8)` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
/// | `Tenor` | Pattern | `fixed_ascii(8)` | `D5`, `W2`, `M3`, `Y1` |
/// | `Pattern` | - | `utf8` | the abstract base of `Tenor` and the reserved ranges |
/// | `Length` | int | `int32` | a byte count |
/// | `TagNum` | int | `int32` | a FIX tag |
/// | `SeqNum` | int | `int64` | a session sequence number outgrows `int32` |
/// | `NumInGroup` | int | `int32` | a repeating-group counter |
/// | `DayOfMonth` | int | `int8` | 1 through 31 |
/// | `Reserved100Plus` | Pattern | `int32` | a user-defined enumeration value |
/// | `Reserved1000Plus` | Pattern | `int32` | as above |
/// | `Reserved4000Plus` | Pattern | `int32` | as above |
/// | `Qty` | float | `float64` | the specification states no scale |
/// | `Price` | float | `float64` | as above |
/// | `PriceOffset` | float | `float64` | as above, signed |
/// | `Percentage` | float | `float64` | `0.0525` is 5.25% |
/// | `Amt` | float | `float64` | one width, so the family is arithmetic |
/// | `UTCTimestamp` | String | `datetime64(ns,"UTC")` | the instant, at the finest FIX width |
/// | `TZTimestamp` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant |
/// | `UTCTimeOnly` | String | `time64(ns)` | a time of day with a fraction |
/// | `LocalMktTime` | String | `time64(ns)` | a time of day, one type with `UTCTimeOnly` |
/// | `UTCDateOnly`, `utcdate` | String | `datetime64(ns,"UTC")` | that day at midnight, in UTC |
/// | `LocalMktDate` | String | `datetime64(ns)` | that day at midnight, stating no zone |
/// | `LocalMktDatetime` | String | `datetime64(ns)` | a local instant, stating no zone |
/// | `TZTimeOnly` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant, on the epoch day |
/// | `MultipleCharValue` | char | `utf8` | space-delimited members |
/// | `MultipleStringValue` | String | `utf8` | space-delimited members |
/// | `XID` | String | `utf8` | an XML identifier |
/// | `XIDREF` | String | `utf8` | a reference to one |
/// | `data` | - | `binary` | opaque bytes |
/// | `XMLData` | data | `binary` | an XML document, opaque here |
///
/// The float family is `float64` because the specification declares all five
/// as `float` subtypes and states no scale for any of them anywhere. A table
/// whose job is to say what a FIX datatype *is* must not improve on the
/// specification, and a pinned scale is wrong in both directions: it truncates
/// a venue quoting finer than eight places and pads every value that does not,
/// and which venues quote how is a fact about a counterparty rather than about
/// a datatype. One width also keeps the family arithmetic - `Amt` at 128 bits
/// beside `Qty` at 64 would make every consumer multiplying a quantity by a
/// price cast first, per row, forever.
///
/// The cost, plainly: binary floating point does not hold `0.1`, and a column
/// of `float64` is not where a book's notional should be accumulated over a
/// day. It is 53 bits of mantissa, exact for every integer quantity below
/// `2^53` and for the price grids venues actually quote - and nothing is lost
/// by the choice, because the authority for a value is the entry as it
/// arrived: the typed column is a view for arithmetic, and a consumer needing
/// exact decimal reads the entry or casts the column. A venue needing exact
/// decimal declares its own `decimal(precision,scale)`, which is why these are
/// names over the ordinary constructors and not a second numeric model.
///
/// `TZTimestamp` keeps the instant and drops the local offset, because an
/// Arrow column carries one zone for every row. Read it under
/// `datetime64(ns,"<zone>")` when the local reading is the value.
///
/// `TZTimeOnly` is the same instant under a missing date, and the epoch day
/// supplies it: `07:39+05:30` is `1970-01-01T02:09:00Z`. That keeps the
/// reading arithmetic - two of them subtract, one sorts against another, the
/// offset is resolved rather than carried as text - where the fixed-width
/// ASCII it used to be kept none of it, and did not even hold the type: the
/// widest legal `TZTimeOnly` is eighteen bytes and the box was sixteen.
pub(crate) const LOGICAL_NAMES: &[(&str, DataType)] = &[
    // The names over a fixed US-ASCII width, which is all they need:
    // each width a literal above zero, so the leaf is built without the
    // validation `DataType::fixed_ascii` runs.
    ("language", DataType::FixedAsciiString(2)),
    ("monthyear", DataType::FixedAsciiString(8)),
    ("tenor", DataType::FixedAsciiString(8)),
    ("pattern", DataType::utf8()),
    // The int family, each carrying the range its base type does not.
    ("length", DataType::Int32),
    ("tagnum", DataType::Int32),
    ("seqnum", DataType::Int64),
    ("numingroup", DataType::Int32),
    ("dayofmonth", DataType::Int8),
    ("reserved100plus", DataType::Int32),
    ("reserved1000plus", DataType::Int32),
    ("reserved4000plus", DataType::Int32),
    // The float family, which the specification declares as `float` and
    // states no scale for anywhere: `float64`, for the reasons above.
    ("qty", DataType::Float64),
    ("price", DataType::Float64),
    ("priceoffset", DataType::Float64),
    ("percentage", DataType::Float64),
    ("amt", DataType::Float64),
    // The temporals.
    (
        "utctimestamp",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    (
        "tztimestamp",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    // Every zone-less time of day is one type. A FIX time is ASCII
    // whatever the version, and the two names differ in which clock the
    // value is read against rather than in what it can hold - so pinning
    // one to seconds makes a capture carrying both cast per row to
    // compare them, and loses a millisecond the wire actually sent.
    ("utctimeonly", DataType::Time64(TimeUnit::Nanosecond)),
    ("localmkttime", DataType::Time64(TimeUnit::Nanosecond)),
    // A FIX date is a day, and a day is an instant at midnight rather
    // than a second type to cast through: a capture joining a settlement
    // date to a transact time compares them directly, and a venue that
    // starts sending a time on a field that carried a date widens no
    // column. The zone is the one the name states - a UTC date is UTC,
    // and a local market date states none, so it must not claim one.
    (
        "utcdate",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    (
        "utcdateonly",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    (
        "localmktdate",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
    ),
    // A local market value states no zone, so it must not claim one.
    // `Naive` is that statement made explicitly rather than by omission.
    (
        "localmktdatetime",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
    ),
    (
        "tztimeonly",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    // The remaining text and binary shapes.
    ("multiplecharvalue", DataType::utf8()),
    ("multiplestringvalue", DataType::utf8()),
    ("xid", DataType::utf8()),
    ("xidref", DataType::utf8()),
    ("data", DataType::binary()),
    ("xmldata", DataType::binary()),
];

/// What this crate claims the FIX Latest datatype names as.
const CRATE: &str = "yggdryl-fix";

/// Claims what FIX registers on the core - the FIX Latest datatype names
/// among the logical names - once for the life of the process, after the
/// market crate's kinds, which a dictionary's fields are typed by; a later
/// call returns at once. Every binding's init and the CLI's `main` call it,
/// and so does a Rust caller before the core reads a FIX datatype name.
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed a market kind
/// or one of these names first.
pub fn install() -> yggdryl::Result<()> {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    yggdryl_market::install()?;
    let _installing = INSTALLING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    for (name, dtype) in LOGICAL_NAMES.iter().cloned() {
        yggdryl::DataType::register_logical_name(name, dtype, CRATE)?;
    }
    let _ = INSTALLED.set(());
    Ok(())
}

// GENERATED by scripts/generate_internals.py - do not edit by hand.
/// What the test suite pins and a caller cannot reach.
///
/// Every test lives in `rust/fix/tests/` and reaches the crate through
/// `yggdryl_fix::`. The handful that pin something no caller can name reach it
/// here instead, under a feature no published build turns on. This is not
/// API: it carries no stability promise, and an item is `pub` only because
/// its own module is unreachable without the feature.
#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    pub use crate::catalog::internals as catalog;
    pub use crate::codec::internals as codec;
    pub use crate::codes::internals as codes;
    pub use crate::component::internals as component;
    pub use crate::document::internals as document;
    pub use crate::enrich::internals as enrich;
    pub use crate::forex::internals as forex;
    pub use crate::global::internals as global;
    pub use crate::group_plan::internals as group_plan;
    pub use crate::identity::internals as identity;
    pub use crate::memo::internals as memo;
    pub use crate::msgtype::internals as msgtype;
    pub use crate::registry::internals as registry;
    pub use crate::retired::internals as retired;
    pub use crate::schema::internals as schema;
    pub use crate::store::internals as store;
}
// END GENERATED
