//! The fields this crate invents, on the standard branch above every published tag.
//!
//! A capture states things about a message that no dictionary publishes: what
//! its bytes hash to, which way its line moved, which version it was read at,
//! the derived facts a store is organised by - one instrument symbol that is
//! the same across venues, its ISIN, the market it traded on, the state the
//! order is in, and one timestamp a partition is cut on - and the facts a
//! bridge's own log states about the line it wrote: the session and the
//! message context it handled the message under, the plugin that logged the
//! line and the one the message came through before it, and the sessions
//! the message moved between. Each belongs in a column, so each is an
//! ordinary field: they lift, column, serialize and resolve like every other
//! field with no special case anywhere.
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

/// The first tag this crate claims.
pub const CRATE_TAG_MIN: i32 = 65_000;

/// One past the last tag this crate claims.
///
/// Bounded rather than open-ended, because a derived definition tag lives
/// above it: see [`crate::FixId::DEFINITION_TAG_MIN`]. A hundred slots is
/// several times the twenty fields this crate defines, so the block has room
/// to grow without ever reaching the one above it.
pub const CRATE_TAG_MAX: i32 = 65_100;

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

/// The tag carrying the session a message came from, as the message states it.
pub const SENDERSESSIONID_TAG: i32 = 65_007;

/// The tag carrying the message context a bridge handled the message in.
pub const MSGCTXID_TAG: i32 = 65_008;

/// The tag carrying the plugin that logged the line, as a bridge names it.
pub const PLUGINID_TAG: i32 = 65_009;

/// The tag carrying the plugin the message came through before the one that
/// logged it, as a bridge names it.
pub const PREVPLUGINID_TAG: i32 = 65_010;

/// The tag carrying the name of the session a message came from.
pub const SENDERSESSIONNAME_TAG: i32 = 65_011;

/// The tag carrying the name of the session a message went to.
pub const TARGETSESSIONNAME_TAG: i32 = 65_012;

/// The tag carrying the instrument's ISIN.
pub const ISINCODE_TAG: i32 = 65_013;

/// The tag carrying the market the message names, as an ISO 10383 MIC.
pub const MICCODE_TAG: i32 = 65_014;

/// The tag carrying the state the order is in.
pub const STATE_TAG: i32 = 65_015;

/// The tag carrying the instrument's own identity.
pub const INSTID_TAG: i32 = 65_016;

/// The tag carrying the message's own time-coupled identity.
pub const ID_TAG: i32 = 65_017;

/// The tag carrying the order chain's identity.
pub const PERSISTENTID_TAG: i32 = 65_018;

/// The tag carrying the session a message went to, as the message states it.
pub const TARGETSESSIONID_TAG: i32 = 65_019;

/// The column the timestamp takes, which is also what its partition names.
pub const TIMESTAMP_NAME: &str = "timestamp";

/// Whether a tag is one of this crate's own.
#[must_use]
pub const fn is_crate_tag(tag: i32) -> bool {
    tag >= CRATE_TAG_MIN && tag < CRATE_TAG_MAX
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

/// The crate's `version` field, non-null, built once, for the reason the
/// clock's is: every built message carries one and looking it up per line
/// would be a dictionary probe per line.
static VERSION_FIELD: LazyLock<Option<Field>> = LazyLock::new(|| {
    let held = FIELDS
        .as_deref()?
        .iter()
        .find(|field| field.as_fix().tag().ok().flatten() == Some(VERSION_TAG))?;
    let mut field = held.clone();
    field.set_nullable(false);
    Some(field)
});

/// The crate's `version` field, non-null, built once.
pub(super) fn version_field() -> Option<&'static Field> {
    VERSION_FIELD.as_ref()
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
        // The session a message came from, and half of a pair whose target
        // side takes the block's next free tag below. A bridge row spells only
        // its own `SESSIONID`, which this side keeps as an alias; where a row
        // spells none, the session instance the bridge's own row header
        // brackets fills it, never over a reading the message stated itself.
        aliased(
            "sendersessionid",
            "SenderSessionId",
            SENDERSESSIONID_TAG,
            DataType::Utf8,
            "The session a message came from: the message's own statement, \
             else the session instance its bridge handled the line on.",
            &["SessionId"],
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
        // The plugins a message passed through inside a bridge, as the bridge
        // names them. FIX publishes the counterparties in `SenderCompID` and
        // `TargetCompID`; the plugin that logged a line, and the one the
        // message came through before it, are facts about the bridge that
        // FIX never states. Both are stated by a row's own column and never
        // derived: `pluginid` is the bracket the bridge's row header writes
        // in front of every line - and, where it spells a branch the
        // dictionary declares, the dialect the row is read under
        // ([`FixCodec::parse_text_record`](super::FixCodec::parse_text_record));
        // `prevpluginid` is only ever a column of that name.
        crated(
            "pluginid",
            "PluginId",
            PLUGINID_TAG,
            DataType::Utf8,
            "The plugin that logged the line inside a bridge, as the bridge \
             names it: the row's own pluginid column, never derived.",
        )?,
        crated(
            "prevpluginid",
            "PrevPluginId",
            PREVPLUGINID_TAG,
            DataType::Utf8,
            "The plugin the message came through before the one that logged \
             it, as the bridge names it: the row's own prevpluginid column, \
             never derived.",
        )?,
        // The sessions a message moved between, by name: what a bridge row
        // spells as `ULFROMSESSIONNAME` and `ULTOSESSIONNAME`, and nothing
        // derived - the plugin that logged a line is `pluginid` above, and a
        // session name is filled only by what the message states or by a
        // row column bearing its name. The identifier is `sendersessionid`
        // and its target half; this is what an operator calls it.
        aliased(
            "sendersessionname",
            "SenderSessionName",
            SENDERSESSIONNAME_TAG,
            DataType::Utf8,
            "The name of the session a message came from, as the bridge row \
             states it.",
            &["ULFromSessionName"],
        )?,
        aliased(
            "targetsessionname",
            "TargetSessionName",
            TARGETSESSIONNAME_TAG,
            DataType::Utf8,
            "The name of the session a message went to, as the bridge row \
             states it.",
            &["ULToSessionName"],
        )?,
        // The instrument and the market, one spelling each: an ISIN as a
        // bridge row states it or as `SecurityID` with an ISIN source, and
        // the market as the MIC the message names first.
        crated(
            "isincode",
            "ISINCode",
            ISINCODE_TAG,
            DataType::Isin,
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
        // The three identities a stream implies, stamped by the lifecycle
        // pass: fixed binary, big-endian, for the reason `msghash` is. The
        // two time-coupled ones open with the impact instant so they sort by
        // the market's own clock.
        crated(
            "instid",
            "InstId",
            INSTID_TAG,
            DataType::fixed_size_binary(DIGEST_WIDTH)?,
            "The instrument's own identity: the xxh128 digest of its market, \
             its classification, its ISIN - else its symbol - and its currency.",
        )?,
        crated(
            "id",
            "Id",
            ID_TAG,
            DataType::fixed_size_binary(DIGEST_WIDTH)?,
            "The message's own identity: the instant closest to the market \
             impact in microseconds, then the xxh3 digest of what it said.",
        )?,
        crated(
            "persistentid",
            "PersistentId",
            PERSISTENTID_TAG,
            DataType::fixed_size_binary(DIGEST_WIDTH)?,
            "The order chain's identity: the instant it was created, then the \
             xxh3 digest of its instrument and first identifier, carried by \
             every later message sharing one of its identifiers.",
        )?,
        // The target side of the session pair, which the block's next free tag
        // takes rather than displacing a tag already published.
        crated(
            "targetsessionid",
            "TargetSessionId",
            TARGETSESSIONID_TAG,
            DataType::Utf8,
            "The session a message went to, as the message states it.",
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
