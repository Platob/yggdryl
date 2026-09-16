//! The definitions this crate invents, above every published tag.
//!
//! A capture states things about a message that no dictionary publishes: what
//! its bytes hash to, which way its line moved, which version it was read at,
//! the derived facts a store is organised by - one instrument symbol that is
//! the same across venues, its ISIN, the market it traded on, the state the
//! order is in, and the hour a partition is cut on - and the facts a
//! bridge's own log states about the line it wrote: the session and the
//! message context it handled the message under, the plugin that logged the
//! line and the one the message came through before it, and the sessions
//! the message moved between. Each belongs in a column: a scalar registers
//! as a field, and the identifiers Map as a group of entries.
//!
//! # Why 65000, and why each is a tag and a name
//!
//! Their tags sit above every tag FIX publishes and above the user-defined
//! ranges venues share, so they collide with nothing a dictionary declares
//! and belong to no dialect: `updatedat` is one identity in every
//! dictionary, a bridge row spelling `SESSIONID` lands on the crate's own
//! column, and a name every registry carries is never an unknown key.
//! [`is_crate_tag`] is the whole test. A field is its tag and its name, so
//! each is declared as both - `PLUGINID_TAG_NAME` is `(65_009, "pluginid")` -
//! and a caller reads the half it needs.
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
//! answers `updatedat` and `sessionid` alike - and a store never writes them,
//! because they are the crate's rather than the store's. A stored copy is
//! read past for the same reason: the crate's own definition is the one that
//! types a row. Folding another dictionary in never counts them either.
//!
//! Thirty-four scalar fields, one Struct and one Map group, each registered
//! by its shape.

use std::sync::LazyLock;

use crate::{DataType, Field, Result};

/// The first tag this crate claims.
pub const CRATE_TAG_MIN: i32 = 65_000;

/// One past the last tag this crate claims.
///
/// Bounded rather than open-ended, because a derived definition tag lives
/// above it: see [`crate::FixId::DEFINITION_TAG_MIN`]. A hundred slots is
/// several times the definitions this crate holds, so the block has room
/// to grow without ever reaching the one above it.
pub const CRATE_TAG_MAX: i32 = 65_100;

/// The tag and name carrying the FIX version a message was read at.
pub const VERSION_TAG_NAME: (i32, &str) = (65_001, "version");

/// The tag and name carrying one instrument symbol that is the same across
/// venues.
pub const SYMBOLTICKER_TAG_NAME: (i32, &str) = (65_002, "symbolticker");

/// The tag and name carrying the settled message or snapshot grid instant.
pub const UPDATEDAT_TAG_NAME: (i32, &str) = (65_003, "updatedat");

/// The tag and name carrying the client order identifier this one descends
/// from.
pub const PARENTCLORDID_TAG_NAME: (i32, &str) = (65_005, "parentclordid");

/// The tag and name carrying the venue order identifier this one descends
/// from.
pub const PARENTORDERID_TAG_NAME: (i32, &str) = (65_006, "parentorderid");

/// The tag and name carrying the session a message came from, as the message
/// states it.
pub const SENDERSESSIONID_TAG_NAME: (i32, &str) = (65_007, "sendersessionid");

/// The tag and name carrying the message context a bridge handled the
/// message in.
pub const MSGCTXID_TAG_NAME: (i32, &str) = (65_008, "msgctxid");

/// The tag and name carrying the plugin that logged the line, as a bridge
/// names it.
pub const PLUGINID_TAG_NAME: (i32, &str) = (65_009, "pluginid");

/// The tag and name carrying the plugin the message came through before the
/// one that logged it, as a bridge names it.
pub const PREVPLUGINID_TAG_NAME: (i32, &str) = (65_010, "prevpluginid");

/// The tag and name carrying the name of the session a message came from.
pub const SENDERSESSIONNAME_TAG_NAME: (i32, &str) = (65_011, "sendersessionname");

/// The tag and name carrying the name of the session a message went to.
pub const TARGETSESSIONNAME_TAG_NAME: (i32, &str) = (65_012, "targetsessionname");

/// The tag and name carrying the instrument's ISIN.
pub const ISINCODE_TAG_NAME: (i32, &str) = (65_013, "isincode");

/// The tag and name carrying the market the message names, as an ISO 10383
/// MIC.
pub const MICCODE_TAG_NAME: (i32, &str) = (65_014, "miccode");

/// The tag and name carrying the state the order is in.
pub const STATE_TAG_NAME: (i32, &str) = (65_015, "state");

/// The tag and name carrying the message's time/content identity bytes.
pub const MSGHASH_TAG_NAME: (i32, &str) = (65_017, "msghash");

/// The tag and name carrying the event chain's identity bytes.
pub const MSGPHASH_TAG_NAME: (i32, &str) = (65_018, "msgphash");

/// The tag and name carrying the session a message went to, as the message
/// states it.
pub const TARGETSESSIONID_TAG_NAME: (i32, &str) = (65_019, "targetsessionid");

/// The tag and name of the Map group carrying the message's identifiers.
pub const ALTIDS_TAG_NAME: (i32, &str) = (65_020, "altids");

/// The tag and name carrying the preceding message's updatedat in its event
/// chain.
pub const PREVUPDATEDAT_TAG_NAME: (i32, &str) = (65_021, "prevupdatedat");

/// The tag and name carrying the preceding message's identity bytes in its
/// event chain.
pub const PREVMSGHASH_TAG_NAME: (i32, &str) = (65_022, "prevmsghash");

/// The tag and name carrying the message's creation instant.
pub const CREATEDAT_TAG_NAME: (i32, &str) = (65_023, "createdat");

/// The tag and name carrying the event chain's exact UTF-8 name.
pub const CODE_TAG_NAME: (i32, &str) = (65_024, "code");

/// The tag and name carrying the real event instant captured by a snapshot.
pub const SNAPSHOTAT_TAG_NAME: (i32, &str) = (65_025, "snapshotat");

/// The tag and name carrying the object one line was read from.
///
/// Where a message was read from is not FIX and is exactly what a monitor
/// orders, joins and prunes on: which file of a day's capture a row came out
/// of, and therefore which file to re-read when a row is disputed. A text
/// read already answers it - its own `sourceurl` column is a URL - and this
/// is the FIX field that column fills, so a capture naming it states the
/// object once per line and the row carries it typed rather than as text
/// nobody can resolve.
pub const SOURCEURL_TAG_NAME: (i32, &str) = (65_026, "sourceurl");

/// The tag and name counting the arrival records one message carried.
///
/// The counter of the `fixentries` group, and a counter in the ordinary FIX
/// sense: `NoPartyIDs` counts `Parties`, and this counts the pairs a line
/// stated. A reader prunes on it without opening the list, which is what a
/// count column is for.
pub const NOFIXENTRIES_TAG_NAME: (i32, &str) = (65_027, "nofixentries");

/// The tag and name carrying the instant the capture itself recorded the line.
pub const RECORDEDAT_TAG_NAME: (i32, &str) = (65_028, "recordedat");

/// The tag and name carrying the instant the message says it stops being good.
pub const EXPIREDAT_TAG_NAME: (i32, &str) = (65_029, "expiredat");

/// The tag and name carrying the currency the bid lane is denominated in.
pub const BIDCURRENCY_TAG_NAME: (i32, &str) = (65_030, "bidcurrency");

/// The tag and name carrying the currency the offer lane is denominated in.
pub const OFFERCURRENCY_TAG_NAME: (i32, &str) = (65_031, "offercurrency");

/// The tag and name carrying the session instance a bridge handled a line on.
///
/// Not `sessionid`: that spelling is a bridge row's own `SESSIONID` key,
/// which names the counterparty session a message states it is on, and
/// [`SENDERSESSIONID_TAG_NAME`] already owns it. This is the bridge's own
/// connection instance, and the two are separate facts - one message states
/// its session once, while two connections to one counterparty are two
/// instances.
pub const BRIDGESESSIONID_TAG_NAME: (i32, &str) = (65_032, "bridgesessionid");

/// The tag and name carrying the instrument's Bloomberg identifier.
pub const BLOOMBERGCODE_TAG_NAME: (i32, &str) = (65_033, "bloombergcode");

/// The tag and name carrying the instrument's CUSIP.
pub const CUSIPCODE_TAG_NAME: (i32, &str) = (65_034, "cusipcode");

/// The tag and name carrying the instrument's SEDOL.
pub const SEDOLCODE_TAG_NAME: (i32, &str) = (65_035, "sedolcode");

/// The tag and name carrying every identifier the instrument is known by.
pub const INSTIDS_TAG_NAME: (i32, &str) = (65_036, "instids");

/// The tag and name naming one message within its session.
pub const SESSIONMSGID_TAG_NAME: (i32, &str) = (65_037, "sessionmsgid");

/// The tag and name naming one occurrence of one message within its session.
pub const SESSIONMSGSEQID_TAG_NAME: (i32, &str) = (65_038, "sessionmsgseqid");

/// Whether a tag is one of this crate's own.
#[must_use]
pub const fn is_crate_tag(tag: i32) -> bool {
    tag >= CRATE_TAG_MIN && tag < CRATE_TAG_MAX
}

/// FIX's own tag and name for which way a message moved.
///
/// Published, not invented: the specification has spelled this `MsgDirection`
/// since 4.4, and a field it already declares is never given a second tag.
pub const MSGDIRECTION_TAG_NAME: (i32, &str) = (385, "MsgDirection");

/// How `isincode` derives from the message where it states none, as the
/// crate field declares it in its `fix:derivation`: the
/// primary identifier under the ISIN source, else the alternate identifier
/// whose source says ISIN - the primary first, so a message stating both
/// states its ISIN in `SecurityID` and the alternate answers only where the
/// primary did not. Each is read through `try_cast(... as isin)`, so a
/// primary no check digit closes is null rather than an answer, and the
/// alternate is consulted behind it; a spelling neither closes is silence.
/// How an instrument identifier derives, for one `SecurityIDSource` code and
/// the type the identifier is.
///
/// One shape for all four, because there is only one rule: the message's own
/// `SecurityID` when its source says this is what it is, else the first
/// `SecurityAltID` occurrence whose source says so. `isincode` has read this
/// way already; `cusipcode`, `sedolcode` and `bloombergcode` are
/// the same sentence with a different letter in it, and writing them as a
/// second mechanism in Rust would be four hand-maintained copies of a
/// declaration the registry already evaluates.
fn identifier_derivation(sources: &[&str], code: &str) -> String {
    let arms: Vec<String> = sources
        .iter()
        .flat_map(|source| {
            [
                format!(
                    "case when securityidsource = '{source}' then \
                     try_cast(securityid as {code}) end"
                ),
                format!(
                    "try_cast(secaltidgrp[securityaltidsource = '{source}'][0].securityaltid \
                     as {code})"
                ),
            ]
        })
        .collect();
    format!("coalesce({})", arms.join(", "))
}

/// How `miccode` derives: the exchange the instrument is listed on, the
/// destination it was routed to, or the market it last traded on, the first
/// stated.
const MICCODE_DERIVATION: &str = "coalesce(securityexchange, exdestination, lastmkt)";

/// How `state` derives: the order's status, else what the report said
/// happened, both read as the crate's one lifecycle vocabulary.
const STATE_DERIVATION: &str = "coalesce(ordstatus, exectype)";

/// What dated the line, where the capture did not.
///
/// The capture's own instant reaches this column by name (see the aliases on
/// the field), so this is only the fallback: a message whose line carried no
/// timestamp is recorded at the instant it says it was sent. Enrichment fills
/// and never overwrites, so a line that *was* dated keeps that date.
const RECORDEDAT_DERIVATION: &str = "sendingtime";

/// What a message says about when it stops being good, strongest first.
///
/// `ExpireTime(126)` is the order's own instant and the only one of these
/// that is already a point in time, so it leads. `ValidUntilTime(62)` is the
/// same statement made by a quote. `ExpireDate(432)` is the order's expiry
/// stated as a day rather than an instant, which is weaker because it needs a
/// session close to mean anything exact. `MaturityDate(541)` is last and is
/// not the order's statement at all - it is the instrument's, and an order
/// cannot outlive the thing it trades, so it bounds the answer when nothing
/// closer was said.
const EXPIREDAT_DERIVATION: &str = "coalesce(expiretime, validuntiltime, expiredate, maturitydate)";

/// What denominates a quote lane.
///
/// FIX states no currency per lane: a two-sided quote carries one
/// `Currency(15)` and both lanes are in it, with `SettlCurrency(120)` behind
/// it for a message that separates settlement from quotation. So both lanes
/// derive from the same pair and answer the same value on an ordinary
/// message - which is the point, because the column is there for the dialect
/// that *does* split them. Enrichment fills and never overwrites, so a bridge
/// stating one lane's currency keeps it and only the other lane derives.
const LANE_CURRENCY_DERIVATION: &str = "coalesce(currency, settlcurrency)";

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| match build() {
    Ok(fields) => Some(fields),
    Err(error) => {
        log::warn!("building FIX crate definitions: {error}");
        None
    }
});

/// The crate's `version` field, non-null, built once, for the reason the
/// clock's is: every built message carries one and looking it up per line
/// would be a dictionary probe per line.
static VERSION_FIELD: LazyLock<Option<Field>> = LazyLock::new(|| {
    let held = FIELDS
        .as_deref()?
        .iter()
        .find(|field| field.as_fix().tag().ok().flatten() == Some(VERSION_TAG_NAME.0))?;
    let mut field = held.clone();
    field.set_nullable(false);
    Some(field)
});

/// The crate's `version` field, non-null, built once.
pub(super) fn version_field() -> Option<&'static Field> {
    VERSION_FIELD.as_ref()
}

/// The crate's own columns that are about *this message* rather than about
/// the instrument its chain follows, and so never carry forward.
///
/// The clocks, because a later message has its own; the identities, because
/// they are computed from the message that carries them; `code`, because the
/// lifecycle stamps it from the chain rather than from the message before;
/// `state`, because carrying an order's last state onto a message that did
/// not state one would report a life the venue never described;
/// `nofixentries` and `sourceurl` and `version`, because they are facts about
/// the line this row was read from.
///
/// Everything else the crate owns is about the instrument or the session -
/// `isincode`, `miccode`, `symbolticker`, `altids`, the lane currencies,
/// `expiredat`, the session names and ids - and carries.
const SETTLED_TO_ONE_MESSAGE: [i32; 12] = [
    UPDATEDAT_TAG_NAME.0,
    CREATEDAT_TAG_NAME.0,
    SNAPSHOTAT_TAG_NAME.0,
    RECORDEDAT_TAG_NAME.0,
    PREVUPDATEDAT_TAG_NAME.0,
    MSGHASH_TAG_NAME.0,
    MSGPHASH_TAG_NAME.0,
    PREVMSGHASH_TAG_NAME.0,
    CODE_TAG_NAME.0,
    STATE_TAG_NAME.0,
    NOFIXENTRIES_TAG_NAME.0,
    SOURCEURL_TAG_NAME.0,
];

/// One field of the crate's own, from its tag and name, with a FIX-style
/// display.
///
/// Display and description use generic field metadata rather than the `fix:`
/// scheme because every catalog the crate writes to understands them.
fn crated(
    (tag, name): (i32, &str),
    display: &str,
    dtype: DataType,
    description: &str,
) -> Result<Field> {
    let mut field = Field::new(name, dtype, !super::identity::is_mandatory(tag));
    field.as_fix_mut().set_tag(tag)?;
    field.set_display(display)?;
    field.set_description(description)?;
    if SETTLED_TO_ONE_MESSAGE.contains(&tag) || tag == VERSION_TAG_NAME.0 {
        field.as_fix_mut().set_transient(false)?;
    }
    Ok(field)
}

/// One field the message implies where it states none, deriving as
/// `derivation` spells it in the expression grammar over the message's
/// fields.
fn derived(
    identity: (i32, &str),
    display: &str,
    dtype: DataType,
    description: &str,
    derivation: &str,
) -> Result<Field> {
    let mut field = crated(identity, display, dtype, description)?;
    field.as_fix_mut().set_derivation(&derivation.parse()?)?;
    Ok(field)
}

/// One field a bridge spells under its own names *and* that derives where
/// none of them arrived - the two halves of a fill, on one field.
fn aliased_derived(
    identity: (i32, &str),
    display: &str,
    dtype: DataType,
    description: &str,
    aliases: &[&str],
    derivation: &str,
) -> Result<Field> {
    let mut field = derived(identity, display, dtype, description, derivation)?;
    field.as_fix_mut().set_names(aliases.iter().copied())?;
    Ok(field)
}

/// One field a bridge spells under its own names, which resolve to it.
fn aliased(
    identity: (i32, &str),
    display: &str,
    dtype: DataType,
    description: &str,
    aliases: &[&str],
) -> Result<Field> {
    let mut field = crated(identity, display, dtype, description)?;
    field.as_fix_mut().set_names(aliases.iter().copied())?;
    Ok(field)
}

/// Builds every field this crate defines.
///
/// The partition column declares, in the protocols every catalog reads, that
/// it is one and how it derives: `field:partition` marks it, and an Iceberg
/// spec built from the fixed schema partitions by identity on it.
fn build() -> Result<Vec<Field>> {
    let mut altids = crated(
        ALTIDS_TAG_NAME,
        "AltIds",
        DataType::map_of(DataType::utf8(), DataType::utf8(), true)?,
        "The identifiers this message states at its own level, keyed by canonical \
         field name in sorted order; repeating-group members are not flattened.",
    )?;
    altids.as_fix_mut().set_counter(ALTIDS_TAG_NAME.0)?;
    Ok(vec![
        // The version the message was *read* at, which is not always the one
        // its `BeginString` claims: a venue that mislabels its session still
        // produces rows, and the column says which dictionary answered them.
        crated(
            VERSION_TAG_NAME,
            "Version",
            DataType::utf8(),
            "The FIX version the message was read at, which is not always \
             the one its BeginString claims.",
        )?,
        // One symbol for one instrument, whatever the venue called it.
        crated(
            SYMBOLTICKER_TAG_NAME,
            "SymbolTicker",
            DataType::utf8(),
            "One instrument symbol that is the same across venues, qualified \
             by its scheme and its exchange where the message states them.",
        )?,
        // The settled timeline, normalized by the lifecycle without reading now.
        crated(
            UPDATEDAT_TAG_NAME,
            "UpdatedAt",
            super::schema::CLOCK_DATATYPE,
            "The settled message instant, truncated to the snapshot grid by the lifecycle.",
        )?,
        // Where an order came from. FIX threads a replace chain through
        // `OrigClOrdID(41)`, which says what this message *replaces* - not
        // what it descends from. A slice of a parent order, or a leg of a
        // basket, has a parent that no standard tag names, and a desk that
        // cannot roll its children up to it cannot answer for the order it
        // actually took.
        crated(
            PARENTCLORDID_TAG_NAME,
            "ParentClOrdID",
            DataType::utf8(),
            "The client order identifier this order descends from, which no \
             standard tag names.",
        )?,
        crated(
            PARENTORDERID_TAG_NAME,
            "ParentOrderID",
            DataType::utf8(),
            "The venue order identifier this order descends from, which no \
             standard tag names.",
        )?,
        // The session a message came from, and half of a pair whose target
        // side takes the block's next free tag below. A bridge row spells only
        // its own `SESSIONID`, which this side keeps as an alias; the instance
        // a bridge's row header brackets is a second fact and has a column of
        // its own ([`BRIDGESESSIONID_TAG_NAME`]).
        aliased(
            SENDERSESSIONID_TAG_NAME,
            "SenderSessionId",
            DataType::utf8(),
            "The session a message came from, as the message states it.",
            &["SessionId"],
        )?,
        // The message context a bridge handled the line in, from the bracket
        // its row header writes after the clock.
        crated(
            MSGCTXID_TAG_NAME,
            "MsgCtxId",
            DataType::utf8(),
            "The message context a bridge handled the message in, as its own \
             log names it.",
        )?,
        // The plugins a message passed through inside a bridge, as the bridge
        // names them. FIX publishes the counterparties in `SenderCompID` and
        // `TargetCompID`; the plugin that logged a line, and the one the
        // message came through before it, are facts about the bridge that
        // FIX never states. Both are stated by a row's own column and never
        // derived: `pluginid` is the bracket the bridge's row header writes
        // in front of every line
        // ([`FixCodec::parse_text_line`](super::FixCodec::parse_text_line));
        // `prevpluginid` is only ever a column of that name.
        crated(
            PLUGINID_TAG_NAME,
            "PluginId",
            DataType::utf8(),
            "The plugin that logged the line inside a bridge, as the bridge \
             names it: the row's own pluginid column, never derived.",
        )?,
        crated(
            PREVPLUGINID_TAG_NAME,
            "PrevPluginId",
            DataType::utf8(),
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
            SENDERSESSIONNAME_TAG_NAME,
            "SenderSessionName",
            DataType::utf8(),
            "The name of the session a message came from, as the bridge row \
             states it.",
            &["ULFromSessionName"],
        )?,
        aliased(
            TARGETSESSIONNAME_TAG_NAME,
            "TargetSessionName",
            DataType::utf8(),
            "The name of the session a message went to, as the bridge row \
             states it.",
            &["ULToSessionName"],
        )?,
        // The instrument and the market, one spelling each: an ISIN as a
        // bridge row states it or as `SecurityID` with an ISIN source, and
        // the market as the MIC the message names first. Each declares how
        // it derives on the field itself, and the enriching pass and the row
        // fill evaluate that declaration and nothing else.
        derived(
            ISINCODE_TAG_NAME,
            "ISINCode",
            DataType::Isin,
            "The instrument's ISIN: the message's own, else SecurityID or a \
             SecurityAltID whose source is ISIN.",
            &identifier_derivation(&["4"], "isin"),
        )?,
        derived(
            MICCODE_TAG_NAME,
            "MICCode",
            DataType::Mic,
            "The market the message names, as an ISO 10383 MIC: the message's \
             own, else SecurityExchange, ExDestination or LastMkt.",
            MICCODE_DERIVATION,
        )?,
        // The state the order is in, whatever code set or word stated it.
        derived(
            STATE_TAG_NAME,
            "State",
            DataType::State,
            "The state the order is in: OrdStatus, else ExecType, read as one \
             lifecycle vocabulary.",
            STATE_DERIVATION,
        )?,
        // Sixteen plain bytes, big-endian, with no version or variant bit:
        // every lake engine reads `fixed[16]` and none reads `uuid` the same
        // way twice, so the FIX identities state the bytes themselves.
        crated(
            MSGHASH_TAG_NAME,
            "MsgHash",
            super::identity::IDENTITY_DATATYPE,
            "The message's sixteen bytes: signed updatedat nanoseconds with \
             the sign bit flipped, then all 64 bits of the canonical named \
             message content's XXH64.",
        )?,
        crated(
            MSGPHASH_TAG_NAME,
            "MsgPHash",
            super::identity::IDENTITY_DATATYPE,
            "The event chain's sixteen bytes: the big-endian XXH3-128 of code \
             alone.",
        )?,
        // The target side of the session pair, which the block's next free tag
        // takes rather than displacing a tag already published.
        crated(
            TARGETSESSIONID_TAG_NAME,
            "TargetSessionId",
            DataType::utf8(),
            "The session a message went to, as the message states it.",
        )?,
        altids,
        crated(
            PREVUPDATEDAT_TAG_NAME,
            "PrevUpdatedAt",
            super::schema::CLOCK_DATATYPE,
            "The preceding message's updatedat in the selected event chain.",
        )?,
        crated(
            PREVMSGHASH_TAG_NAME,
            "PrevMsgHash",
            super::identity::IDENTITY_DATATYPE,
            "The preceding message's sixteen identity bytes in the selected \
             event chain.",
        )?,
        crated(
            CREATEDAT_TAG_NAME,
            "CreatedAt",
            super::schema::CLOCK_DATATYPE,
            "The creation instant, preserved after initial materialization.",
        )?,
        crated(
            CODE_TAG_NAME,
            "Code",
            DataType::utf8(),
            "The exact event-chain name; empty means unknown.",
        )?,
        crated(
            SNAPSHOTAT_TAG_NAME,
            "SnapshotAt",
            super::schema::CLOCK_DATATYPE,
            "The instant a reading of this chain was taken at, ungridded; empty \
             on every row that is not a snapshot.",
        )?,
        // Where the line was read from, typed as the URL it is. A text read
        // names its own column `sourceurl` too, so a capture fills this
        // field by position without anyone spelling a mapping.
        crated(
            SOURCEURL_TAG_NAME,
            "SourceUrl",
            DataType::Url,
            "The object this message's line was read from.",
        )?,
        // The arrival record's counter. The group it counts is not a registry
        // definition - a `fixentry` contains `fixentries`, and a definition
        // that referenced itself would be a cycle - so the counter is here
        // and the group is built beside the fixed row it closes.
        crated(
            NOFIXENTRIES_TAG_NAME,
            "NoFixEntries",
            DataType::Int32,
            "How many pairs the message carried, in arrival order.",
        )?,
        // When the capture wrote the line down, which is a fact about the
        // capture and not about the message - so it is outside the content
        // hash for the same reason `sourceurl` is: one day's log re-cut,
        // replayed or copied records the same message at a second instant,
        // and both copies must digest alike.
        // The text reader already carries the line's own instant, under the
        // name it gives that column - so the fill is an alias, the way every
        // other capture column reaches its field, rather than a second
        // mapping. `mtime` is the reader's own spelling and the rest are the
        // ones it accepts for it, so a batch from anywhere lands here too.
        // What no line dated is the message's own `SendingTime`, which is
        // where the derivation picks up.
        aliased_derived(
            RECORDEDAT_TAG_NAME,
            "RecordedAt",
            super::schema::CLOCK_DATATYPE,
            "The instant the capture recorded this line: the line's own text \
             timestamp, else SendingTime.",
            // `mtime` only. The text reader accepts `timestamp`, `time`,
            // `ts`, `written_at` and `event_time` for that column too, but
            // those are its *intake* spellings and this is a dictionary: an
            // alias here is a name the registry answers for, and
            // `event_time` folds onto the shipped `EventTime(1145)`, which
            // is that field's name and not this one's to take.
            &["mtime"],
            RECORDEDAT_DERIVATION,
        )?,
        derived(
            EXPIREDAT_TAG_NAME,
            "ExpiredAt",
            super::schema::CLOCK_DATATYPE,
            "The instant the message stops being good: ExpireTime, else \
             ValidUntilTime, else ExpireDate, else the instrument's \
             MaturityDate.",
            EXPIREDAT_DERIVATION,
        )?,
        derived(
            BIDCURRENCY_TAG_NAME,
            "BidCurrency",
            DataType::Currency,
            "The currency the bid lane is denominated in: the message's own \
             Currency, else SettlCurrency.",
            LANE_CURRENCY_DERIVATION,
        )?,
        derived(
            OFFERCURRENCY_TAG_NAME,
            "OfferCurrency",
            DataType::Currency,
            "The currency the offer lane is denominated in: the message's own \
             Currency, else SettlCurrency.",
            LANE_CURRENCY_DERIVATION,
        )?,
        // The session instance the bridge handled a line on, which its own
        // row header states and no FIX message carries: `SenderCompID` names
        // a counterparty, and two connections to one counterparty are two
        // sessions. It is filled from the bracket alone, because the joined
        // identifiers below must be the same string on every leg of one
        // message and a leg that spelled its own session would split them.
        crated(
            BRIDGESESSIONID_TAG_NAME,
            "BridgeSessionId",
            DataType::utf8(),
            "The session instance a bridge handled a line on, as its own row \
             header brackets it - never what the message states about itself.",
        )?,
        // The three identifiers beside `isincode`, each typed as the code it
        // is so a row joins on it rather than on text that looks like one.
        derived(
            BLOOMBERGCODE_TAG_NAME,
            "BloombergCode",
            DataType::Bloomberg,
            "The instrument's Bloomberg identifier: the message's own, else a \
             SecurityID or SecurityAltID whose source is Bloomberg.",
            &identifier_derivation(&["A", "S"], "bloomberg"),
        )?,
        derived(
            CUSIPCODE_TAG_NAME,
            "CUSIPCode",
            DataType::Cusip,
            "The instrument's CUSIP: the message's own, else a SecurityID or \
             SecurityAltID whose source is CUSIP.",
            &identifier_derivation(&["1"], "cusip"),
        )?,
        derived(
            SEDOLCODE_TAG_NAME,
            "SEDOLCode",
            DataType::Sedol,
            "The instrument's SEDOL: the message's own, else a SecurityID or \
             SecurityAltID whose source is SEDOL.",
            &identifier_derivation(&["2"], "sedol"),
        )?,
        instids()?,
        // One message within its session, and one occurrence of it. Built by
        // concatenation rather than hashed, because a reader grepping a log
        // for `e7254b20:9f015ed023` must find the same string the row holds.
        crated(
            SESSIONMSGID_TAG_NAME,
            "SessionMsgId",
            DataType::utf8(),
            "The session and the message context that name one message within \
             it, joined by a colon.",
        )?,
        crated(
            SESSIONMSGSEQID_TAG_NAME,
            "SessionMsgSeqId",
            DataType::utf8(),
            "The session, the message context and the sequence number that \
             name one occurrence of one message, joined by colons.",
        )?,
    ])
}

/// Every identifier one instrument is known by, as one column.
///
/// A Struct rather than five columns beside each other, because they are one
/// fact with five spellings: a reader joining two captures wants "the same
/// instrument", and a venue that publishes an ISIN while another publishes a
/// SEDOL is describing the same thing. The five stay *also* available as
/// their own columns, because a table partitions and filters on a column and
/// not on a struct member - this is the joined view of them, not a second
/// owner: [`FixMsg::instrument_ids`](super::FixMsg) fills it from those
/// columns rather than from anything of its own.
///
/// Built once and shared: the members are this crate's own types and owe
/// nothing to a dictionary, so every registry and every row holds the same
/// field.
fn instids() -> Result<Field> {
    static FIELD: LazyLock<Option<Field>> = LazyLock::new(|| match build_instids() {
        Ok(field) => Some(field),
        Err(error) => {
            log::warn!("building the FIX instrument identifier struct: {error}");
            None
        }
    });
    FIELD.clone().ok_or_else(|| crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new_static(INSTIDS_TAG_NAME.1),
        reason: crate::text::expected_got("an instrument identifier struct", "a refused build"),
    })
}

/// The members, in the order a reader reads them: what the instrument *is*,
/// then what each registry calls it.
fn build_instids() -> Result<Field> {
    let member = |name: &str, dtype: DataType| dtype.nullable_field(name);
    let mut field = DataType::from_fields([
        member(CFICODE_MEMBER, DataType::Cfi),
        member(ISINCODE_TAG_NAME.1, DataType::Isin),
        member(BLOOMBERGCODE_TAG_NAME.1, DataType::Bloomberg),
        member(CUSIPCODE_TAG_NAME.1, DataType::Cusip),
        member(SEDOLCODE_TAG_NAME.1, DataType::Sedol),
    ])?
    .nullable_field(INSTIDS_TAG_NAME.1);
    field.as_fix_mut().set_tag(INSTIDS_TAG_NAME.0)?;
    field.set_display("InstIds")?;
    field.set_description(
        "Every identifier this instrument is known by, filled from the columns \
         beside it.",
    )?;
    Ok(field)
}

/// What the classification member of [`instids`] is called.
///
/// FIX's own name for it, because that is the column it is filled from.
const CFICODE_MEMBER: &str = "cficode";

/// The fields this crate defines, in tag order.
///
/// Every registry already holds them: [`FixRegistry::new`](super::FixRegistry::new)
/// inserts them first, so this is the listing a schema or a document walks
/// rather than something a caller registers.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let held = yggdryl::fix_crate_fields()?;
/// assert_eq!(held.len(), 36);
/// assert_eq!(held[0].name(), "version");
/// assert_eq!(held[0].display(), Some("Version"));
/// // No partition column: how a layout is cut is the target's to decide -
/// // an Iceberg table takes an `hour` transform over `updatedat` - and a
/// // materialized copy of that instant was a second owner of it.
/// assert!(held.iter().all(|field| !field.is_partition()));
/// assert!(held.iter().all(|field| field.name() != "timepartition"));
/// // The columns a message implies declare how, on the field itself.
/// let state = held.iter().find(|field| field.name() == "state").expect("state");
/// assert_eq!(
///     state.as_fix().derivation()?.map(|term| term.to_string()),
///     Some("coalesce(ordstatus, exectype)".to_owned()),
/// );
/// // Above every tag FIX or a venue publishes, and its tag and name are
/// // its identity.
/// let (tag, name) = yggdryl::VERSION_TAG_NAME;
/// let mine = held[0].as_fix().id()?.expect("an identity");
/// assert_eq!(mine, yggdryl::FixId::of(tag, name)?);
/// assert!(yggdryl::is_crate_tag(yggdryl::STATE_TAG_NAME.0));
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

/// FIX's own tag and name for a message's type.
pub const MSGTYPE_TAG_NAME: (i32, &str) = (35, "MsgType");

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
    /// let mut field = DataType::utf8().nullable_field("msgtype");
    /// field.as_fix_mut().set_tag(35)?;
    /// let mut registry = FixRegistry::from_fields([field])?;
    /// let message = registry.register_msgtype("P Report Ack", Some("AllocationReportAck"), None)?;
    /// assert_eq!(message.as_str(), "P Report Ack");
    /// assert!(matches!(message.as_field().dtype(), DataType::Struct(_)));
    /// assert!(std::ptr::eq(
    ///     registry.msgtype("P Report Ack")?,
    ///     registry.msgtype("AllocationReportAck")?,
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
        let field = self.field_by_tag(MSGTYPE_TAG_NAME.0)?;
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
        if next.get_msgtype(&value).is_none() {
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
            next.create_definition(crate::FixCategory::Components, message)?;
        }
        // Resolution also rejects a code shared by several contextual definitions.
        next.msgtype(&value)?;
        *self = next;
        self.msgtype(&value)
    }
}
