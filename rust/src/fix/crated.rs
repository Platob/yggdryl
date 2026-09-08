//! The fields this crate invents, on the standard branch above every published tag.
//!
//! A capture states things about a message that no dictionary publishes: what
//! its bytes hash to, which way its line moved, which version it was read at,
//! the derived facts a store is organised by - one instrument symbol that is
//! the same across venues, its ISIN, the market it traded on, the state the
//! order is in, and one timestamp a partition is cut on - and the facts a
//! bridge's own log states about the line it wrote: the session and the
//! message context it handled the message under, and the plugins and plugin
//! sessions the message moved between. Each belongs in a column, so each is
//! an ordinary field: they lift, column, serialize and resolve like every
//! other field with no special case anywhere.
//!
//! # Why the standard branch, and why 65000
//!
//! Their tags sit above every tag FIX publishes and above the user-defined
//! ranges venues share, so they collide with nothing a dictionary declares
//! and need no branch of their own: `timestamp` is one identity in every
//! dictionary, a bridge row spelling `SESSIONID` lands on the crate's own
//! column, and a name every registry carries is never an unknown key.
//! [`is_crate_tag`] is the whole test.
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
//! because they are the crate's rather than the store's. A stored copy is
//! read past for the same reason: the crate's own definition is the one that
//! types a row. Folding another dictionary in never counts them either.
//!
//! One mechanism, sixteen fields.

use std::sync::LazyLock;

use crate::{DataType, DigestAlgorithm, Field, Result, TimeUnit, Timezone};

/// The first tag this crate claims: everything from here up is the crate's.
pub const CRATE_TAG_MIN: i32 = 65_000;

/// The tag carrying a message's value digest.
pub const MSGHASH_TAG: i32 = 65_000;

/// The tag carrying the FIX version a message was read at.
pub const VERSION_TAG: i32 = 65_001;

/// The tag carrying one instrument symbol that is the same across venues.
pub const SYMBOLTICKER_TAG: i32 = 65_002;

/// The tag carrying the timestamp a capture is ordered by.
pub const TIMESTAMP_TAG: i32 = 65_003;

/// The tag carrying the partition that timestamp falls in.
pub const UNIXPARTITION_TAG: i32 = 65_004;

/// The tag carrying the client order identifier this one descends from.
pub const PARENTCLORDID_TAG: i32 = 65_005;

/// The tag carrying the venue order identifier this one descends from.
pub const PARENTORDERID_TAG: i32 = 65_006;

/// The tag carrying the session a message belongs to, as the message states it.
pub const SESSIONID_TAG: i32 = 65_007;

/// The tag carrying the message context a bridge handled the message in.
pub const MSGCTXID_TAG: i32 = 65_008;

/// The tag carrying the plugin a message came from, as a bridge names it.
pub const SENDERPLUGINID_TAG: i32 = 65_009;

/// The tag carrying the plugin a message went to, as a bridge names it.
pub const TARGETPLUGINID_TAG: i32 = 65_010;

/// The tag carrying the plugin session a message came from.
pub const SENDERPLUGINSESSION_TAG: i32 = 65_011;

/// The tag carrying the plugin session a message went to.
pub const TARGETPLUGINSESSION_TAG: i32 = 65_012;

/// The tag carrying the instrument's ISIN.
pub const ISINCODE_TAG: i32 = 65_013;

/// The tag carrying the market the message names, as an ISO 10383 MIC.
pub const MICCODE_TAG: i32 = 65_014;

/// The tag carrying the state the order is in.
pub const STATE_TAG: i32 = 65_015;

/// The column the timestamp takes, which is also what its partition names.
pub const TIMESTAMP_NAME: &str = "timestamp";

/// Whether a tag is one of this crate's own.
#[must_use]
pub const fn is_crate_tag(tag: i32) -> bool {
    tag >= CRATE_TAG_MIN
}

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

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| build().ok());

/// The crate's `timestamp` field as a built message carries it: non-null,
/// resolved once for every message the builder stamps.
static TIMESTAMP_FIELD: LazyLock<Option<Field>> = LazyLock::new(|| {
    let held = FIELDS
        .as_deref()?
        .iter()
        .find(|field| field.as_fix().tag().ok().flatten() == Some(TIMESTAMP_TAG))?;
    let mut field = held.clone();
    field.set_nullable(false);
    Some(field)
});

/// The crate's `timestamp` field, non-null, built once.
pub(super) fn timestamp_field() -> Option<&'static Field> {
    TIMESTAMP_FIELD.as_ref()
}

/// One field of the crate's own, with its folded identity and FIX-style display.
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
    field.as_fix_mut().set_tag(tag)?;
    field.set_display(display)?;
    field.set_description(description)?;
    Ok(field)
}

/// One field a bridge spells under its own names, which resolve to it.
fn aliased(
    name: &str,
    display: &str,
    tag: i32,
    dtype: DataType,
    description: &str,
    aliases: &[&str],
) -> Result<Field> {
    let mut field = crated(name, display, tag, dtype, description)?;
    field.as_fix_mut().set_aliases(aliases.iter().copied())?;
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
    // Named by the column it reads, because that is what the column is
    // called in a row.
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
            "The timestamp a capture is ordered by: the row's own clock, else \
             the first clock the message answers in decreasing exactness.",
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
        // The session a message belongs to, as the message itself states it:
        // a bridge row spells `SESSIONID`, and the row header's own bracket
        // is the bridge's, not the message's, so it never fills this.
        crated(
            "sessionid",
            "SessionId",
            SESSIONID_TAG,
            DataType::Utf8,
            "The session a message belongs to, as the message states it.",
        )?,
        // The message context a bridge handled the line in, from the bracket
        // its row header writes after the clock.
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
        // The plugin sessions a message moved between: what a bridge row
        // spells as `ULFROMSESSIONNAME` and `ULTOSESSIONNAME`, and what the
        // row header names in front of a line the plugin sent or received.
        aliased(
            "senderpluginsession",
            "SenderPluginSession",
            SENDERPLUGINSESSION_TAG,
            DataType::Utf8,
            "The plugin session a message came from: the bridge row's own \
             statement, else the plugin that logged the line it sent.",
            &["ULFromSessionName"],
        )?,
        aliased(
            "targetpluginsession",
            "TargetPluginSession",
            TARGETPLUGINSESSION_TAG,
            DataType::Utf8,
            "The plugin session a message went to: the bridge row's own \
             statement, else the plugin that logged the line it received.",
            &["ULToSessionName"],
        )?,
        // The instrument and the market, one spelling each: an ISIN as a
        // bridge row states it or as `SecurityID` with an ISIN source, and
        // the market as the MIC the message names first.
        crated(
            "isincode",
            "ISINCode",
            ISINCODE_TAG,
            DataType::Utf8,
            "The instrument's ISIN: the message's own, else SecurityID or a \
             SecurityAltID whose source is ISIN.",
        )?,
        crated(
            "miccode",
            "MICCode",
            MICCODE_TAG,
            DataType::Mic,
            "The market the message names, as an ISO 10383 MIC: the message's \
             own, else SecurityExchange, ExDestination or LastMkt.",
        )?,
        // The state the order is in, whatever code set or word stated it.
        crated(
            "state",
            "State",
            STATE_TAG,
            DataType::State,
            "The state the order is in: OrdStatus, else ExecType, read as one \
             lifecycle vocabulary.",
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
/// // On the standard branch, above every tag FIX or a venue publishes.
/// let mine = held[0].as_fix().id()?.expect("an identity");
/// assert_eq!(mine, yggdryl::FixId::standard(yggdryl::MSGHASH_TAG));
/// assert!(yggdryl::is_crate_tag(yggdryl::STATE_TAG));
/// assert!(!yggdryl::is_crate_tag(35));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the schema grammar's refusal when one of the datatypes does not
/// build, which is a defect in this module rather than anything a caller did.
pub fn fix_crate_fields() -> Result<&'static [Field]> {
    FIELDS
        .as_deref()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: "crate".into(),
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
