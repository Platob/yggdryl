//! The fields this crate invents, on a branch of its own.
//!
//! A capture states things about a message that no dictionary publishes: what
//! its bytes hash to, which way its line moved, which version it was read at,
//! the two derived facts a store is organised by - one instrument symbol
//! that is the same across venues, and one timestamp a partition is cut on -
//! and the four facts a bridge's own log states about the line it wrote: the
//! session and the message context it handled the message under, and the
//! plugins the message came from and went to. Each belongs in a column, so
//! each is an ordinary field: they lift, column, serialize and resolve like
//! every other field with no special case anywhere.
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
//! # Every registry holds them
//!
//! [`FixRegistry::new`](super::FixRegistry::new) inserts them before anything
//! else, so a dictionary loaded from a store, built from fields or left empty
//! answers `timestamp` and `sessionid` alike - and a store never writes them,
//! because they are the crate's rather than the store's. A stored copy of
//! this branch is read past for the same reason: the crate's own definition
//! is the one that types a row.
//!
//! One mechanism, eleven fields.

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

/// The tag carrying the session a bridge handled the message under.
pub const SESSIONID_TAG: i32 = 30008;

/// The tag carrying the message context a bridge handled the message in.
pub const MSGCTXID_TAG: i32 = 30009;

/// The tag carrying the plugin a message came from, as a bridge names it.
pub const SENDERPLUGINID_TAG: i32 = 30010;

/// The tag carrying the plugin a message went to, as a bridge names it.
pub const TARGETPLUGINID_TAG: i32 = 30011;

/// The column the timestamp takes, which is also what its partition names.
pub const TIMESTAMP_NAME: &str = "timestamp";

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
        [TIMESTAMP_NAME.to_owned()],
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
            TIMESTAMP_NAME,
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
        // What a bridge's own log states about the line it wrote, in the
        // bracket after its clock: the session and the message context the
        // line was handled under. A row header captures them and the row
        // fills them, so a monitor joins a bridge's own log on them.
        crated(
            "sessionid",
            "SessionId",
            SESSIONID_TAG,
            DataType::Utf8,
            "The session a bridge handled the message under, as its own log \
             names it.",
        )?,
        crated(
            "msgctxid",
            "MsgCtxId",
            MSGCTXID_TAG,
            DataType::Utf8,
            "The message context a bridge handled the message in, as its own \
             log names it.",
        )?,
        // The plugins a message moved between, as a bridge names them. FIX
        // publishes the counterparties in `SenderCompID` and `TargetCompID`;
        // the plugin that carried a message inside a bridge is a fact about
        // the bridge, and one FIX never states.
        crated(
            "senderpluginid",
            "SenderPluginId",
            SENDERPLUGINID_TAG,
            DataType::Utf8,
            "The plugin a message came from inside a bridge, as the bridge \
             names it.",
        )?,
        crated(
            "targetpluginid",
            "TargetPluginId",
            TARGETPLUGINID_TAG,
            DataType::Utf8,
            "The plugin a message went to inside a bridge, as the bridge \
             names it.",
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
/// Every registry already holds them: [`FixRegistry::new`](super::FixRegistry::new)
/// inserts them first, so this is the listing a schema or a document walks
/// rather than something a caller registers.
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

/// FIX's own tag for a message's type.
pub const MSGTYPE_TAG: i32 = 35;

impl super::FixRegistry {
    /// Registers one message type, answering the value it takes.
    ///
    /// A dictionary is never complete. Venues invent message types, bridges
    /// write composite keys like `P Report Ack`, and a reader that refused
    /// what it had not been told about would drop exactly the traffic someone
    /// is trying to understand. So a type the code set does not have is added
    /// to it rather than rejected, and the value it takes is
    /// [`MsgType::coerce`](crate::types::MsgType::coerce)'s - itself where it
    /// fits, a stable synthesized value where it does not.
    ///
    /// `spelling` is what the wire, the bridge or the configuration calls it.
    /// `name` is the symbolic name the set files it under, and the spelling
    /// becomes an alias of it when the two differ, so both still reach the
    /// value; with none named the spelling is the name. `description` is the
    /// source's own wording for it.
    ///
    /// Nothing is hard-coded: the vocabulary is the dictionary's own code set
    /// on tag 35, and this adds to it exactly as a generator would.
    ///
    /// Idempotent, and enriching rather than replacing: registering a type the
    /// dictionary already spells answers its existing value, adds a spelling
    /// the set did not answer to, and fills a description it did not have -
    /// but never rewrites a name or a wording a source already gave it. A
    /// reader may call it per row without growing the code set per row.
    ///
    /// # Errors
    ///
    /// Returns the registry's own refusal when tag 35 is absent, when its
    /// code set will not read, when the synthesized value collides with a
    /// different spelling - which is a real conflict and not something to
    /// resolve by picking one - or when a name or spelling another code
    /// already answers to would be added, because a spelling two codes reach
    /// resolves to neither.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::FixRegistry;
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// let mut registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    ///
    /// // One the specification publishes is already there.
    /// assert_eq!(registry.register_msgtype("NewOrderSingle", None, None)?.as_str(), "D");
    ///
    /// // One a bridge invents is added, and stays put.
    /// let held = registry.register_msgtype("P Report Ack", None, None)?;
    /// assert!(held.is_synthetic());
    /// assert_eq!(registry.register_msgtype("P Report Ack", None, None)?, held);
    ///
    /// // A later source describing the same type fills what it was missing.
    /// registry.register_msgtype("P Report Ack", None, Some("Allocation Report ACK"))?;
    /// let field = registry.field_by_tag(35)?;
    /// assert_eq!(
    ///     field
    ///         .as_fix()
    ///         .code_by_name("P Report Ack")
    ///         .and_then(|code| code.parse_doc().ok().flatten()),
    ///     Some("Allocation Report ACK".to_owned()),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_msgtype(
        &mut self,
        spelling: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<crate::types::MsgType> {
        let field = self.field_by_tag(MSGTYPE_TAG)?;
        let view = field.as_fix();
        let mut codes: Vec<super::FixCode> = view
            .codes()
            .filter_map(std::result::Result::ok)
            .map(super::FixCode::from)
            .collect();
        // Every spelling this registration states, the value's own first.
        let named = name.unwrap_or(spelling);
        let (value, at) = match view.code_value(spelling) {
            // Already spelled, by name, alias or wire value: the set answers,
            // and this states whatever it did not already hold.
            Some(held) => {
                let value = crate::types::MsgType::new(held)?;
                let at = codes
                    .iter()
                    .position(|code| code.value() == value.as_str())
                    .ok_or_else(|| {
                        crate::Error::absent("fix code", format_args!("value {held:?} on tag 35"))
                    })?;
                (value, at)
            }
            None => {
                let value = crate::types::MsgType::coerce(spelling);
                if let Some(taken) = view.code_name(value.as_str()) {
                    return Err(crate::Error::Conflict {
                        expected: "a free message type value",
                        actual: "one another spelling holds",
                        path: crate::text::expected_got(
                            format_args!("{spelling:?} at {:?}", value.as_str()),
                            format_args!("{taken:?}"),
                        ),
                    });
                }
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
        let mut field = field.clone();
        field.as_fix_mut().set_codes(&codes)?;
        self.update(field)?;
        Ok(value)
    }
}
