//! The fields this crate invents, on a branch of its own.
//!
//! A capture states things about a message that no dictionary publishes: what
//! its bytes hash to, which way its line moved, which version it was read at,
//! and the two derived facts a store is organised by - one instrument symbol
//! that is the same across venues, and one timestamp a partition is cut on.
//! Each belongs in a column, so each is an ordinary field: they lift, column,
//! serialize and resolve like every other field with no special case anywhere.
//!
//! # Why a branch
//!
//! Their tags are in FIX's user-defined range - high in it, from 30001 up,
//! rather than down in the 5000s where venues actually crowd. That is not
//! enough on its own: a venue is free to define its own 30001. They are
//! therefore carried on this crate's own branch, so the
//! [identifier](super::FixId) differs even where the tag does not - same
//! tag, different branch, different identity.
//!
//! # And why one of them is not here
//!
//! `MsgDirection` is not invented: FIX publishes it at tag 385, and a field
//! the specification already has is never given a second tag. What this crate
//! adds there is the *type* - the packed four bytes rather than a string -
//! which the dictionary carries like any other coded field.
//!
//! One mechanism, seven fields.

use std::sync::LazyLock;

use crate::{DataType, DigestAlgorithm, Field, Result, TimeUnit, Timezone};

use super::FixBranch;

/// The branch this crate's own fields are defined on.
pub const CRATE_BRANCH: &str = "yggdryl";

/// The tag carrying a message's value digest.
pub const MSGHASH_TAG: i32 = 30001;

/// The tag carrying the FIX version a message was read at.
pub const VERSION_TAG: i32 = 30002;

/// The tag carrying one instrument symbol that is the same across venues.
pub const SYMBOLTICKER_TAG: i32 = 30003;

/// The tag carrying the timestamp a capture is ordered by.
pub const TIMESTAMP_TAG: i32 = 30004;

/// The tag carrying the partition that timestamp falls in.
pub const UNIXPARTITION_TAG: i32 = 30005;

/// The tag carrying the client order identifier this one descends from.
pub const PARENTCLORDID_TAG: i32 = 30006;

/// The tag carrying the venue order identifier this one descends from.
pub const PARENTORDERID_TAG: i32 = 30007;

/// FIX's own tag for which way a message moved.
///
/// Published, not invented: the specification has spelled this `MsgDirection`
/// since 4.4, and a field it already declares is never given a second tag.
pub const MSGDIRECTION_TAG: i32 = 385;

/// The algorithm a message digest is taken with.
const DIGEST_ALGORITHM: DigestAlgorithm = DigestAlgorithm::Xxh128;

/// The digest's width in bytes, which is the algorithm's.
const DIGEST_WIDTH: i32 = 16;

/// How wide a partition is by default, in seconds.
///
/// An hour. A day is too coarse to prune a capture with - a session's whole
/// traffic lands in one partition - and a minute makes a day of capture
/// fourteen hundred of them, which is more files than rows in the quiet ones.
pub const DEFAULT_PARTITION_SECONDS: i64 = 3_600;

/// This crate's branch, built once.
fn branch() -> Result<FixBranch> {
    FixBranch::from_str(CRATE_BRANCH)
}

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| build().ok());

/// One field on the crate's branch, with its folded identity and FIX-style display.
///
/// Display and description use generic field metadata rather than the `fix:`
/// scheme because every catalog the crate writes to understands them.
fn crated(
    name: &str,
    display: &str,
    tag: i32,
    dtype: DataType,
    description: &str,
) -> Result<Field> {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_id(&branch()?, tag)?;
    field.set_display(display)?;
    field.set_description(description)?;
    Ok(field)
}

/// Builds every field this crate defines.
///
/// Two of them declare more than a type. `msghash` is a digest holder, so it
/// says which algorithm filled it and what it read; `unixpartition` is a
/// derived partition column, so it says which column it derives from and how.
/// Both are said in the protocols the crate already has - `digest:` and
/// `partition:` beside `iceberg:` - rather than in a spelling only a FIX
/// reader would know to look for.
fn build() -> Result<Vec<Field>> {
    // `FixedSizeBinary`, big-endian, because a digest is not a string and
    // must not become one. Big-endian is the one layout where byte order and
    // numeric order agree on every machine: a little-endian digest sorts
    // differently than it compares, and someone eventually sorts it.
    let mut msghash = crated(
        "msghash",
        "MsgHash",
        MSGHASH_TAG,
        DataType::fixed_size_binary(DIGEST_WIDTH)?,
        "The xxh128 digest of what the message said, over the arrival \
             record with the envelope tags left out.",
    )?;
    // Holder first: the algorithm and the sources are both refused on a field
    // that has not said it holds a digest.
    msghash.as_digest_mut().set_holder()?;
    msghash.as_digest_mut().set_algorithm(DIGEST_ALGORITHM)?;
    // The arrival record, because that is what the digest actually reads: the
    // columns are one reading of a message and `entries` is the message.
    msghash
        .as_digest_mut()
        .set_sources([super::ENTRIES_COLUMN])?;

    // The partition that timestamp falls in, as whole seconds since the
    // epoch. An integer rather than a rendered date: a partition value is
    // compared and ranged over, and a string would sort lexically.
    let mut unixpartition = crated(
        "unixpartition",
        "UnixPartition",
        UNIXPARTITION_TAG,
        DataType::Int64,
        "The partition the market timestamp falls in, as whole seconds \
             since the epoch floored to the partition width.",
    )?;
    // Named by the column it reads, which is the tag, because that is what
    // the column is called in a row.
    //
    // Written through the protocol view rather than through
    // `PartitionFieldMut::set_sources`, because that half of the partition
    // layer is built only with Arrow and these fields exist whether or not it
    // is. The rendering is the crate's one canonical spelling either way, and
    // the metadata write validates it exactly as the setter's would.
    let sources = crate::metadata::render_source_list(
        crate::metadata::PARTITION_SOURCES_KEY,
        [super::schema::rendered(TIMESTAMP_TAG)],
    )?;
    unixpartition
        .as_partition_mut()
        .insert("sources", sources)?;
    // `truncate[3600]`, not `hour`: the value is seconds floored to a multiple
    // of the width, which is what Iceberg's truncate transform means, whereas
    // its `hour` yields hours since the epoch and the grammar's own `hour`
    // yields the clock hour. The transform that says what the column holds is
    // the one that goes on it.
    //
    // Written through the protocol view rather than through the Iceberg
    // builder, because these fields exist whether or not the crate was built
    // with Iceberg and a declaration is text either way.
    unixpartition
        .as_iceberg_mut()
        .insert("transform", partition_transform())?;

    Ok(vec![
        msghash,
        // The version the message was *read* at, which is not always the one
        // its `BeginString` claims: a venue that mislabels its session still
        // produces rows, and the column says which dictionary answered them.
        crated(
            "version",
            "Version",
            VERSION_TAG,
            DataType::Utf8,
            "The FIX version the message was read at, which is not always \
             the one its BeginString claims.",
        )?,
        // One symbol for one instrument, whatever the venue called it.
        crated(
            "symbolticker",
            "SymbolTicker",
            SYMBOLTICKER_TAG,
            DataType::Utf8,
            "One instrument symbol that is the same across venues, qualified \
             by its scheme and its exchange where the message states them.",
        )?,
        // The timestamp a capture is ordered by, in UTC because a capture
        // spans venues and a local time cannot be compared across them.
        crated(
            "timestamp",
            "Timestamp",
            TIMESTAMP_TAG,
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            },
            "The market timestamp a capture is ordered by: the first clock \
             the message answers, in decreasing exactness.",
        )?,
        unixpartition,
        // Where an order came from. FIX threads a replace chain through
        // `OrigClOrdID(41)`, which says what this message *replaces* - not
        // what it descends from. A slice of a parent order, or a leg of a
        // basket, has a parent that no standard tag names, and a desk that
        // cannot roll its children up to it cannot answer for the order it
        // actually took.
        crated(
            "parentclordid",
            "ParentClOrdID",
            PARENTCLORDID_TAG,
            DataType::Utf8,
            "The client order identifier this order descends from, which no \
             standard tag names.",
        )?,
        crated(
            "parentorderid",
            "ParentOrderID",
            PARENTORDERID_TAG,
            DataType::Utf8,
            "The venue order identifier this order descends from, which no \
             standard tag names.",
        )?,
    ])
}

/// The default partition width, spelled as the transform that produces it.
///
/// One rendering in one place, so the declared transform and the value the row
/// carries can never say different things.
fn partition_transform() -> String {
    format!("truncate[{DEFAULT_PARTITION_SECONDS}]")
}

/// The fields this crate defines, in tag order.
///
/// Registering them is a caller's choice rather than a load-time side effect:
/// a dictionary read from a store is what that store held, and a reader that
/// silently gained seven fields would write them back out again.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let held = yggdryl::fix_crate_fields()?;
/// assert_eq!(held[0].name(), "msghash");
/// assert_eq!(held[0].display(), Some("MsgHash"));
/// // Same tag as a venue's own 30001 would be, and a different identity.
/// let mine = held[0].as_fix().id()?.expect("an identity");
/// assert_ne!(mine, yggdryl::FixId::standard(30001));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the schema grammar's refusal when the crate's own branch or one of
/// the datatypes does not build, which is a defect in this module rather than
/// anything a caller did.
pub fn fix_crate_fields() -> Result<&'static [Field]> {
    FIELDS
        .as_deref()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: CRATE_BRANCH.into(),
            reason: crate::text::expected_got("the crate's own fields", "a build failure"),
        })
}

impl super::FixRegistry {
    /// Adds this crate's own fields, so they resolve by tag and by name.
    ///
    /// A dictionary that has them can type a `msghash` or `timestamp` column
    /// from the registry like any other. One that does not is unchanged -
    /// nothing in reading a message needs them, because all of them are facts
    /// about the capture rather than about the wire.
    ///
    /// # Errors
    ///
    /// Returns the registry's own refusal when a field collides with
    /// something already held, which cannot happen on a dictionary that does
    /// not already declare this crate's branch.
    pub fn with_crate_fields(mut self) -> Result<Self> {
        for field in fix_crate_fields()? {
            self.insert(field.clone())?;
        }
        Ok(self)
    }
}

/// FIX's own tag for a message's type.
pub const MSGTYPE_TAG: i32 = 35;

impl super::FixRegistry {
    /// Registers a message code and returns its registry-owned Struct definition.
    ///
    /// Existing names and descriptions are preserved; new spellings become aliases.
    /// Unknown codes retain their full text and acquire an empty message Struct.
    /// Code and message insertion succeed atomically, including collision checks.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// use yggdryl::{DataType, FixRegistry};
    /// let mut field = DataType::Utf8.nullable_field("msgtype");
    /// field.as_fix_mut().set_tag(35)?;
    /// let mut registry = FixRegistry::from_fields([field])?;
    /// let message = registry.register_msgtype("P Report Ack", Some("AllocationReportAck"), None)?;
    /// assert_eq!(message.as_str(), "P Report Ack");
    /// assert!(matches!(message.as_field().dtype(), DataType::Struct(_)));
    /// assert!(std::ptr::eq(
    ///     registry.msgtype("P Report Ack", None)?,
    ///     registry.msgtype("AllocationReportAck", None)?,
    /// ));
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_msgtype(
        &mut self,
        spelling: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<&super::MsgType> {
        let field = self.field_by_tag(MSGTYPE_TAG)?;
        let view = field.as_fix();
        let mut codes: Vec<super::FixCode> = view
            .codes()
            .map(|code| code.map(super::FixCode::from))
            .collect::<Result<_>>()?;
        // Every spelling this registration states, the value's own first.
        let named = name.unwrap_or(spelling);
        let (value, at) = match view.code_value(spelling) {
            // Already spelled, by name, alias or wire value: the set answers,
            // and this states whatever it did not already hold.
            Some(held) => {
                let value = held.to_owned();
                let at = codes
                    .iter()
                    .position(|code| code.value() == value.as_str())
                    .ok_or_else(|| {
                        crate::Error::absent("fix code", format_args!("value {held:?} on tag 35"))
                    })?;
                (value, at)
            }
            None => {
                let value = spelling.to_owned();
                codes.push(super::FixCode::new(named, value.as_str()));
                (value, codes.len() - 1)
            }
        };
        // A spelling another code already answers to is refused rather than
        // added: two codes one spelling reaches resolve to neither.
        for spelling in [named, spelling] {
            if let Some(taken) = view.code_value(spelling) {
                if taken != value.as_str() {
                    return Err(crate::Error::Conflict {
                        expected: "a free message type spelling",
                        actual: "one another code answers to",
                        path: crate::text::expected_got(
                            format_args!("{spelling:?} at {:?}", value.as_str()),
                            format_args!("{taken:?}"),
                        ),
                    });
                }
            }
            if !codes[at].is_spelled(spelling) {
                codes[at].push_alias(spelling);
            }
        }
        if codes[at].description().is_none() {
            if let Some(description) = description {
                codes[at] = codes[at].clone().with_description(description);
            }
        }
        let mut next = self.clone();
        let mut field = field.clone();
        field.as_fix_mut().set_codes(&codes)?;
        next.update(field)?;
        let branch = super::FixBranch::STANDARD;
        if next.get_msgtype(&value, Some(&branch)).is_none() {
            let normalized = crate::types::normalized(codes[at].name());
            let canonical = if !normalized.is_empty()
                && !matches!(normalized.as_str(), "." | "..")
                && normalized
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
            {
                normalized.to_string()
            } else {
                let mut name = String::from("message");
                for byte in value.bytes() {
                    use std::fmt::Write;
                    write!(name, "{byte:02x}")
                        .map_err(|error| crate::Error::absent("message name", error))?;
                }
                name
            };
            let mut message = crate::DataType::from_fields([])?.required_field(canonical);
            message.as_fix_mut().set_msgtype(&value)?;
            next.create_definition(crate::FixCategory::Messages, message)?;
        }
        // Resolution also rejects a code shared by several contextual definitions.
        next.msgtype(&value, Some(&branch))?;
        *self = next;
        self.msgtype(&value, Some(&branch))
    }
}
