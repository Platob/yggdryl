//! The definitions this crate invents, above every published tag.
//!
//! A message is a market event - a [`MarketEvent`](crate::graph::MarketEvent)
//! with a FIX body around it - and the facts the event states that no
//! dictionary publishes are columns of the row: its identity and the one it
//! has across its lifecycle and the code that names it there, the names it
//! goes by, its parents, the code its content digests to, when it happened, was created, expires and was read
//! as a snapshot, the element it follows, its place in the chain, the state
//! it stands in, its price, quantity and unit, the currency and unit of each
//! quote lane, and the instrument and the market as the crate's own codes.
//! Beside them stand the facts a capture states about the line - where it
//! was read from, when the capture recorded it, how many pairs it carried -
//! and the ones a bridge's own log states about the line it wrote: the
//! session instance, the message context and the plugin that logged it.
//! Each belongs in a column: a scalar registers as a field, and the
//! identifiers Map as a group of entries.
//!
//! # Why 65000, and why each is a tag and a name
//!
//! Their tags sit above every tag FIX publishes and above the user-defined
//! ranges venues share, so they collide with nothing a dictionary declares
//! and belong to no dialect: `unix` is one identity in every dictionary, a
//! bridge row spelling `SESSIONID` lands on the crate's own column, and a
//! name every registry carries is never an unknown key. [`is_crate_tag`] is
//! the whole test. A field is its tag and its name, so each is declared as
//! both - `PLUGINID_TAG_NAME` is `(65_009, "pluginid")` - and a caller reads
//! the half it needs.
//!
//! # What is FIX's own is not here
//!
//! `MsgDirection` is not invented: FIX publishes it at tag 385, and a field
//! the specification already has is never given a second tag. What this crate
//! adds there is the *type* - the packed four bytes rather than a string -
//! which the dictionary carries like any other coded field. The event's
//! currency, side and classification are FIX's `Currency(15)`, `Side(54)`
//! and `CFICode(461)`, and its quote lanes' prices and sizes FIX's
//! `BidPx(132)`, `OfferPx(133)`, `BidSize(134)` and `OfferSize(135)`; the
//! crate names what FIX does not.
//!
//! # Every registry holds them
//!
//! [`FixRegistry::new`](super::FixRegistry::new) inserts them before anything
//! else, so a dictionary loaded from a store, built from fields or left empty
//! answers `unix` and `pluginid` alike - and a store never writes them,
//! because they are the crate's rather than the store's. A stored copy is
//! read past for the same reason: the crate's own definition is the one that
//! types a row. Folding another dictionary in never counts them either.
//!
//! Thirty-two scalar fields and two Map groups, each registered by its shape.

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

/// The tag and name carrying when the message happened: the settled
/// instant, nanoseconds since the Unix epoch, UTC.
pub const UNIX_TAG_NAME: (i32, &str) = (65_003, "unix");

/// The tag and name carrying the message context a bridge handled the
/// message in.
pub const MSGCTXID_TAG_NAME: (i32, &str) = (65_008, "msgctxid");

/// The tag and name carrying the plugin that logged the line, as a bridge
/// names it.
pub const PLUGINID_TAG_NAME: (i32, &str) = (65_009, "pluginid");

/// The tag and name carrying the instrument's ISIN.
pub const ISINCODE_TAG_NAME: (i32, &str) = (65_013, "isincode");

/// The tag and name carrying the market the message names, as an ISO 10383
/// MIC.
pub const MICCODE_TAG_NAME: (i32, &str) = (65_014, "miccode");

/// The tag and name carrying the state the order is in.
pub const STATE_TAG_NAME: (i32, &str) = (65_015, "state");

/// The tag and name carrying the code the message's content digests to:
/// the XXH3-64 of what the event states and the named FIX content behind it.
pub const HASHCODE_TAG_NAME: (i32, &str) = (65_017, "hashcode");

/// The tag and name carrying the cross hash code: the XXH3-64 of the cross
/// code, zero where the message names none.
pub const CROSSHASHCODE_TAG_NAME: (i32, &str) = (65_018, "crosshashcode");

/// The tag and name of the Map group carrying the names the message goes by,
/// each under the field that stated it.
pub const IDENTIFIERS_TAG_NAME: (i32, &str) = (65_020, "identifiers");

/// The tag and name carrying when the message this one follows happened.
pub const PREVUNIX_TAG_NAME: (i32, &str) = (65_021, "prevunix");

/// The tag and name carrying the identity of the message this one follows.
pub const PREVUUID_TAG_NAME: (i32, &str) = (65_022, "prevuuid");

/// The tag and name carrying when the message was created.
pub const CREATUNIX_TAG_NAME: (i32, &str) = (65_023, "creatunix");

/// The tag and name carrying the grid instant a walk read the message as
/// the snapshot of.
pub const SNAPUNIX_TAG_NAME: (i32, &str) = (65_025, "snapunix");

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

/// The tag and name carrying when the message stops being good.
pub const EXPIRUNIX_TAG_NAME: (i32, &str) = (65_029, "expirunix");

/// The tag and name carrying the currency the bid lane is quoted in.
pub const BIDCURRENCY_TAG_NAME: (i32, &str) = (65_030, "bidcurrency");

/// The tag and name carrying the currency the ask lane is quoted in.
pub const ASKCURRENCY_TAG_NAME: (i32, &str) = (65_031, "askcurrency");

/// The tag and name carrying the session instance a bridge handled a line on.
///
/// Not `sessionid`: that spelling is a bridge row's own `SESSIONID` key,
/// which names the counterparty session a message states it is on. This is
/// the bridge's own connection instance, and the two are separate facts -
/// one message states its session once, while two connections to one
/// counterparty are two instances.
pub const MSGSESSIONID_TAG_NAME: (i32, &str) = (65_032, "msgsessionid");

/// The tag and name carrying the instrument's Bloomberg identifier.
pub const BLOOMBERGCODE_TAG_NAME: (i32, &str) = (65_033, "bloombergcode");

/// The tag and name carrying the instrument's CUSIP.
pub const CUSIPCODE_TAG_NAME: (i32, &str) = (65_034, "cusipcode");

/// The tag and name carrying the instrument's SEDOL.
pub const SEDOLCODE_TAG_NAME: (i32, &str) = (65_035, "sedolcode");

/// The tag and name carrying the message's identity: the UUIDv7 its instant
/// and its code derive.
pub const CURRUUID_TAG_NAME: (i32, &str) = (65_039, "curruuid");

/// The tag and name carrying the identity every message of one lifecycle
/// shares, where the message names one.
pub const CROSSUUID_TAG_NAME: (i32, &str) = (65_040, "crossuuid");

/// The tag and name carrying the identities of the messages this one
/// descends from.
pub const PARENTUUIDS_TAG_NAME: (i32, &str) = (65_041, "parentuuids");

/// The tag and name carrying the message's place in its chain: how many
/// came before it.
pub const SEQNUM_TAG_NAME: (i32, &str) = (65_042, "seqnum");

/// The tag and name carrying the price the message states.
pub const PX_TAG_NAME: (i32, &str) = (65_043, "px");

/// The tag and name carrying the quantity the message states.
pub const QTY_TAG_NAME: (i32, &str) = (65_044, "qty");

/// The tag and name carrying the unit the quantity is counted in.
pub const UNIT_TAG_NAME: (i32, &str) = (65_045, "unit");

/// The tag and name carrying the unit the bid lane's size is counted in.
pub const BIDUNIT_TAG_NAME: (i32, &str) = (65_046, "bidunit");

/// The tag and name carrying the unit the ask lane's size is counted in.
pub const ASKUNIT_TAG_NAME: (i32, &str) = (65_047, "askunit");

/// The tag and name carrying the cross code: the identifier every message
/// of one lifecycle shares, as the message spells it - its `OrderID`, else
/// its `ClOrdID`, `OrigClOrdID`, `QuoteID`, `QuoteReqID` or `MDReqID`, the
/// first stated - and what the cross hash code and the cross identity
/// derive from.
pub const CROSSCODE_TAG_NAME: (i32, &str) = (65_048, "crosscode");

/// The tag and name of the Map group carrying what a bridge stated under
/// its own namespaces - the `TECH.` and `AMON.` keys and their kin - each
/// under the key as the bridge spelled it, folded, in sorted order.
pub const METADATA_TAG_NAME: (i32, &str) = (65_049, "metadata");

/// The tag and name of the fixed row every message answers as, as a store
/// dumps it: `components/fixmsg.json` states the columns
/// [`fix_schema`](super::fix_schema) builds, each a reference to the field
/// or group that types it. The crate stays the one owner of that row - a
/// reader passes the document over, as it passes the crate's own fields -
/// so the dump is what a consumer reads the row's shape from without
/// running this crate, and nothing this crate reads back.
pub const FIXMSG_TAG_NAME: (i32, &str) = (65_050, "fixmsg");

/// The tag and name carrying the price the statement before this one in its
/// chain stated.
///
/// The last trade has no column of its own beside this one: FIX already
/// names it, at `LastPx(31)` and `LastQty(32)`, and a message holds what
/// those tags state as
/// [`get_lastpx`](crate::graph::MarketElement::get_lastpx) and
/// [`get_lastqty`](crate::graph::MarketElement::get_lastqty), under FIX's
/// own tags. A price *before* the message has no such field -
/// `PrevClosePx` is the market's close rather than the step before in this
/// chain - so these two are the crate's.
pub const PREVPX_TAG_NAME: (i32, &str) = (65_051, "prevpx");

/// The tag and name carrying the quantity that statement stated.
pub const PREVQTY_TAG_NAME: (i32, &str) = (65_052, "prevqty");

/// The tag and name carrying whether the instrument could be traded when
/// the message was sent.
///
/// FIX states this three different ways and never as one answer: the
/// security's own trading status, the session's, and the instrument's
/// listing status, each a code set of its own with a dozen values that are
/// not about trading at all. The column is the answer those add up to, and
/// the field that carries it says how, so a desk that reads a fourth
/// spelling edits the derivation rather than this crate.
pub const TRADABLE_TAG_NAME: (i32, &str) = (65_053, "tradable");

/// The tag and name carrying the ticker the message's instrument is known
/// by.
///
/// `Symbol(55)` is the field, and the dictionary already says how a message
/// that carries no `Symbol` still names one - an exchange symbol under
/// `SecurityID`, a `SecurityAltID` issued by the exchange - as that field's
/// own derivation. This column reads what that settles on and drops FIX's
/// one non-answer, the `[N/A]` a message writes when it means "this has no
/// ticker; it is identified by `SecurityID`", so a consumer reading it
/// never has to know the convention: a null here is an instrument with no
/// ticker, and the codes beside it are what names it.
pub const SYMBOLTICKER_TAG_NAME: (i32, &str) = (65_054, "symbolticker");

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

/// How an instrument identifier derives, for one `SecurityIDSource` code and
/// the type the identifier is.
///
/// One shape for all four, because there is only one rule: the message's own
/// `SecurityID` when its source says this is what it is, else the first
/// `SecurityAltID` occurrence whose source says so. `isincode` has read this
/// way already; `cusipcode`, `sedolcode` and `bloombergcode` are
/// the same sentence with a different letter in it, and writing them as a
/// second mechanism in Rust would be four hand-maintained copies of a
/// declaration the registry already evaluates. Each is read through
/// `try_cast(... as isin)`, so a primary no check digit closes is null
/// rather than an answer, and the alternate is consulted behind it; a
/// spelling neither closes is silence.
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
const EXPIRUNIX_DERIVATION: &str = "coalesce(expiretime, validuntiltime, expiredate, maturitydate)";

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

/// Whether the message says the instrument could be traded, strongest
/// first: the security's own trading status, then the session's, then the
/// instrument's listing status.
///
/// Each arm names the codes that trade and the codes that do not, and
/// answers nothing under a code that is about something else - an imbalance,
/// a price indication, a status a venue spells `Unknown or Invalid`. A
/// silence is not a `false`: a market that said nothing about trading did
/// not say the instrument was closed.
const TRADABLE_DERIVATION: &str = "coalesce(\
     case when securitytradingstatus in (3, 17) then true \
     when securitytradingstatus in (1, 2, 4, 18, 19, 21) then false end, \
     case when tradsesstatus in (2) then true \
     when tradsesstatus in (1, 3, 4, 5, 7) then false end, \
     case when securitystatus in ('1', '3') then true \
     when securitystatus in ('2', '4', '5', '6', '9', '11') then false end)";

/// How `symbolticker` derives: what `Symbol(55)` settles on, which is the
/// field's own derivation and not restated here, minus the `[N/A]` a
/// message writes to say it names no ticker at all.
///
/// Both spellings of that non-answer are named because a line ends a field
/// at a closing bracket - a bridge wraps its frames in them - so `[N/A]` on
/// the wire reaches a row as `[N/A`.
const SYMBOLTICKER_DERIVATION: &str =
    "case when symbol <> '[N/A]' and symbol <> '[N/A' then symbol end";

/// What the quantity is counted in: the unit of measure the message states.
const UNIT_DERIVATION: &str = "unitofmeasure";

/// What the message says about the price before its own, where it says
/// anything: the market's close. Named rather than hard-coded, so the term
/// resolves against the dictionary and a venue's own spelling of
/// `PrevClosePx` fills the column too.
const PREVPX_DERIVATION: &str = "prevclosepx";

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| match build() {
    Ok(fields) => Some(fields),
    Err(error) => {
        log::warn!("building FIX crate definitions: {error}");
        None
    }
});

/// The crate's own columns that are about *this message* rather than about
/// the instrument its chain follows, and so never carry forward.
///
/// The instants, because a later message has its own; the identities and
/// the codes, because they are computed from the message that carries them;
/// the place in the chain and the element it follows, because a walk states
/// them per message; `state`, because carrying an order's last state onto a
/// message that did not state one would report a life the venue never
/// described; `nofixentries` and `sourceurl`, because they are facts about
/// the line this row was read from.
///
/// Everything else the crate owns is about the instrument or the session -
/// `isincode`, `miccode`, the identifiers, the lane currencies and units,
/// the price, the quantity - and carries.
const SETTLED_TO_ONE_MESSAGE: [i32; 16] = [
    UNIX_TAG_NAME.0,
    CREATUNIX_TAG_NAME.0,
    SNAPUNIX_TAG_NAME.0,
    RECORDEDAT_TAG_NAME.0,
    PREVUNIX_TAG_NAME.0,
    PREVUUID_TAG_NAME.0,
    HASHCODE_TAG_NAME.0,
    CROSSHASHCODE_TAG_NAME.0,
    CURRUUID_TAG_NAME.0,
    CROSSUUID_TAG_NAME.0,
    CROSSCODE_TAG_NAME.0,
    SEQNUM_TAG_NAME.0,
    PARENTUUIDS_TAG_NAME.0,
    STATE_TAG_NAME.0,
    NOFIXENTRIES_TAG_NAME.0,
    SOURCEURL_TAG_NAME.0,
];

/// The crate's own columns every message states: the instants the identity
/// is settled against, the codes and the identity it settles to.
const ALWAYS_STATED: [i32; 6] = [
    UNIX_TAG_NAME.0,
    CREATUNIX_TAG_NAME.0,
    HASHCODE_TAG_NAME.0,
    CROSSHASHCODE_TAG_NAME.0,
    CURRUUID_TAG_NAME.0,
    CROSSUUID_TAG_NAME.0,
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
    let mut field = Field::new(name, dtype, !ALWAYS_STATED.contains(&tag));
    field.as_fix_mut().set_tag(tag)?;
    field.set_display(display)?;
    field.set_description(description)?;
    if SETTLED_TO_ONE_MESSAGE.contains(&tag) {
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

/// Builds every field this crate defines, in tag order.
fn build() -> Result<Vec<Field>> {
    let clock = || super::schema::CLOCK_DATATYPE;
    let mut metadata = crated(
        METADATA_TAG_NAME,
        "Metadata",
        DataType::map_of(DataType::utf8(), DataType::utf8(), true)?,
        "What a bridge stated under its own namespaces - a `TECH.` or an \
         `AMON.` key - each under the key as the bridge spelled it, folded, \
         in sorted order.",
    )?;
    metadata.as_fix_mut().set_counter(METADATA_TAG_NAME.0)?;
    let mut identifiers = crated(
        IDENTIFIERS_TAG_NAME,
        "Identifiers",
        DataType::map_of(DataType::utf8(), DataType::utf8(), true)?,
        "The names this message goes by, each under the canonical name of the \
         field that stated it, in sorted order; repeating-group members are \
         not flattened.",
    )?;
    identifiers
        .as_fix_mut()
        .set_counter(IDENTIFIERS_TAG_NAME.0)?;
    Ok(vec![
        // When the message happened: the settled instant every clock a
        // message states resolves to, and what its identity opens with.
        crated(
            UNIX_TAG_NAME,
            "Unix",
            clock(),
            "When the message happened: the settled instant, UTC.",
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
        // The plugin that logged a line inside a bridge, as the bridge names
        // it: a fact about the bridge that FIX never states, stated by the
        // row's own column - the bracket the bridge's row header writes in
        // front of every line - and never derived.
        crated(
            PLUGINID_TAG_NAME,
            "PluginId",
            DataType::utf8(),
            "The plugin that logged the line inside a bridge, as the bridge \
             names it: the row's own pluginid column, never derived.",
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
        // The two codes: what the message's content digests to, and what
        // the identifier its lifecycle shares digests to.
        crated(
            HASHCODE_TAG_NAME,
            "HashCode",
            DataType::UInt64,
            "The XXH3-64 of what the event states and the named FIX content \
             behind it.",
        )?,
        crated(
            CROSSHASHCODE_TAG_NAME,
            "CrossHashCode",
            DataType::UInt64,
            "The XXH3-64 of the cross code; zero where the message names none.",
        )?,
        identifiers,
        crated(
            PREVUNIX_TAG_NAME,
            "PrevUnix",
            clock(),
            "When the message this one follows happened, where it follows one.",
        )?,
        crated(
            PREVUUID_TAG_NAME,
            "PrevUuid",
            DataType::Uuid,
            "The identity of the message this one follows, where it follows one.",
        )?,
        crated(
            CREATUNIX_TAG_NAME,
            "CreatUnix",
            clock(),
            "When the message was created: what it states, else when the \
             original was sent, else when it happened; the earliest its \
             chain knows once followed.",
        )?,
        crated(
            SNAPUNIX_TAG_NAME,
            "SnapUnix",
            clock(),
            "The grid instant a walk read this message as the snapshot of; \
             empty on every row no snapshot was taken of.",
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
        // mapping. `mtime` is the reader's own spelling; what no line dated
        // is the message's own `SendingTime`, which is where the derivation
        // picks up.
        aliased_derived(
            RECORDEDAT_TAG_NAME,
            "RecordedAt",
            clock(),
            "The instant the capture recorded this line: the line's own text \
             timestamp, else SendingTime.",
            &["mtime"],
            RECORDEDAT_DERIVATION,
        )?,
        derived(
            EXPIRUNIX_TAG_NAME,
            "ExpirUnix",
            clock(),
            "When the message stops being good: ExpireTime, else \
             ValidUntilTime, else ExpireDate, else the instrument's \
             MaturityDate.",
            EXPIRUNIX_DERIVATION,
        )?,
        derived(
            BIDCURRENCY_TAG_NAME,
            "BidCurrency",
            DataType::Currency,
            "The currency the bid lane is quoted in: the message's own \
             Currency, else SettlCurrency.",
            LANE_CURRENCY_DERIVATION,
        )?,
        derived(
            ASKCURRENCY_TAG_NAME,
            "AskCurrency",
            DataType::Currency,
            "The currency the ask lane is quoted in: the message's own \
             Currency, else SettlCurrency.",
            LANE_CURRENCY_DERIVATION,
        )?,
        // The session instance the bridge handled a line on, which its own
        // row header states and no FIX message carries: `SenderCompID` names
        // a counterparty, and two connections to one counterparty are two
        // sessions. It is filled from the bracket alone.
        crated(
            MSGSESSIONID_TAG_NAME,
            "MsgSessionId",
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
        // The identities: the message's own, the one its lifecycle shares,
        // and the ones it descends from.
        crated(
            CURRUUID_TAG_NAME,
            "CurrUuid",
            DataType::Uuid,
            "The message's identity: the UUIDv7 its instant and its code derive.",
        )?,
        crated(
            CROSSUUID_TAG_NAME,
            "CrossUuid",
            DataType::Uuid,
            "The identity every message of one lifecycle shares, derived from \
             the identifier they share; the message's own where it names none.",
        )?,
        crated(
            PARENTUUIDS_TAG_NAME,
            "ParentUuids",
            DataType::list(DataType::Uuid.required_field("parentuuid")),
            "The identities of the messages this one descends from, in the \
             order it states them.",
        )?,
        crated(
            SEQNUM_TAG_NAME,
            "SeqNum",
            DataType::UInt64,
            "The message's place in its chain: how many came before it.",
        )?,
        // The market's numbers, exact. Price and quantity carry no
        // derivation: the message holds Price, OrderQty, LastPx, LastQty,
        // AvgPx, CumQty and LeavesQty as its own typed facts, and
        // `MarketElement::fill_market` is the one statement of which of them
        // the message is about.
        crated(
            PX_TAG_NAME,
            "Px",
            DataType::DECIMAL,
            "The price the message is about: Price, else LastPx, else AvgPx, \
             else the price its own side's lane quotes.",
        )?,
        crated(
            QTY_TAG_NAME,
            "Qty",
            DataType::DECIMAL,
            "The quantity the message is about: OrderQty, else LastQty, else \
             CumQty, else LeavesQty, else the size its own side's lane quotes.",
        )?,
        derived(
            UNIT_TAG_NAME,
            "Unit",
            DataType::utf8(),
            "The unit the quantity is counted in: UnitOfMeasure.",
            UNIT_DERIVATION,
        )?,
        crated(
            BIDUNIT_TAG_NAME,
            "BidUnit",
            DataType::utf8(),
            "The unit the bid lane's size is counted in, where the lane \
             states it.",
        )?,
        crated(
            ASKUNIT_TAG_NAME,
            "AskUnit",
            DataType::utf8(),
            "The unit the ask lane's size is counted in, where the lane \
             states it.",
        )?,
        // The chain's own name, as the message spells it: what the cross
        // hash code and the cross identity derive from, and what a walk
        // forces onto a message of the chain that spells none.
        crated(
            CROSSCODE_TAG_NAME,
            "CrossCode",
            DataType::utf8(),
            "The identifier every message of one lifecycle shares: OrderID, \
             else ClOrdID, OrigClOrdID, QuoteID, QuoteReqID or MDReqID, the \
             first stated.",
        )?,
        metadata,
        // The step before this message in its chain: what a price moved
        // from, and what the market said about the instrument. Declared
        // last because the list is in tag order and these are the crate's
        // newest columns.
        derived(
            PREVPX_TAG_NAME,
            "PrevPx",
            DataType::DECIMAL,
            "The price stated before this message: PrevClosePx where the \
             message states one, else the price its predecessor in the chain \
             stated.",
            PREVPX_DERIVATION,
        )?,
        crated(
            PREVQTY_TAG_NAME,
            "PrevQty",
            DataType::DECIMAL,
            "The quantity the message's predecessor in the chain stated.",
        )?,
        derived(
            TRADABLE_TAG_NAME,
            "Tradable",
            DataType::Boolean,
            "Whether the instrument could be traded when the message was \
             sent: SecurityTradingStatus, else TradSesStatus, else \
             SecurityStatus; null where none of them says.",
            TRADABLE_DERIVATION,
        )?,
        derived(
            SYMBOLTICKER_TAG_NAME,
            "SymbolTicker",
            DataType::utf8(),
            "The ticker the instrument is known by: what Symbol settles on, \
             null where the message names none.",
            SYMBOLTICKER_DERIVATION,
        )?,
    ])
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
/// assert_eq!(held.len(), 38);
/// assert_eq!(held[0].name(), "unix");
/// assert_eq!(held[0].display(), Some("Unix"));
/// // No partition column: how a layout is cut is the target's to decide -
/// // an Iceberg table takes an `hour` transform over `unix` - and a
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
/// let (tag, name) = yggdryl::UNIX_TAG_NAME;
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
    /// assert!(matches!(message.as_field().dtype(), DataType::Structure(_)));
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
