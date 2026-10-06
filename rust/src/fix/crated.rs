//! The definitions this crate invents, above every published tag.
//!
//! A message is a market event - an [`Event`](crate::graph::Event) that is
//! also a [`Market`](crate::graph::Market) - with a FIX body around it, and
//! what this crate owns is only the facts
//! the event states that *no dictionary publishes*: its identity and the one
//! it has across its lifecycle and the code that names it there, the names
//! it goes by, its parents, the lines it was read from, the code its content
//! digests to, when it happened, was created, executed, recorded and was read
//! as a snapshot, the element it follows and its place at its instant. Beside
//! them stand the facts a capture states
//! about the line - where it was read from and how many pairs it carried -
//! and the ones a bridge's own log states about the line it wrote: the
//! session instance, the message context and the plugin that logged it,
//! and the session event the first two join to with the message's own type
//! and sequence. Each belongs in a column: a scalar registers as a field,
//! and the identifiers Map as a group of entries.
//!
//! The standard owns prices, quantities and classification. Four normalized
//! instrument identifiers have crate columns because their FIX sources depend
//! on an identifier source or exchange context: ISIN, Bloomberg and FIGI are
//! views of the message's security identifiers, and MIC is the market. A
//! CUSIP or a SEDOL is one more security identifier under its own key, never
//! a column of its own. The columns read the existing market holder; they add
//! no second value. `CFICode(461)` already names its classification, so it keeps
//! its standard tag. `marketdatakind` projects the message component's
//! symbolic `FIX:msgcat` metadata through the registry code set into the
//! [`crate::MarketDataKind`] member it names. State and expiry are event facts derived from the standard's
//! status and expiry fields, then carried by lifecycle; a newer explicit expiry
//! replaces the previous deadline.
//!
//! The element and event facts are the columns every graph element and
//! event is stated in, [`ElementColumn`] and [`EventColumn`]: each crate
//! field here takes that column's datatype, display and wording, so a text
//! line's batch, a FIX row and a chained message carry one column under one
//! name, one datatype and one sentence, and join on it. Eight of the fifteen
//! say more than the column can - they name the FIX fields a value is read
//! off, or what the FIX parse does with it, which is this module's to know
//! and no other medium's - and those eight spell their own wording beside
//! the tag. The market and operation facts are [`MarketColumn`]s and
//! [`OperationColumn`]s, whose datatype and display a crate field takes the
//! same way, spelling its FIX wording beside them.
//!
//! # A market column FIX states under a name of its own is derived
//!
//! A market or an operation fact FIX already states under another name -
//! the ticker in `Symbol(55)`, the ask in `OfferPx(133)`, what the order
//! asked for in `OrderQty(38)` - is stated a second time under the name
//! every market row states it by, so the fixed row opens with the same
//! columns a `marketdata` row does. Such a column is derived: the row
//! states what the message's fields say, and a row read back that states
//! one is the row's word, as a lifecycle row states what its walk folded
//! forward. It is listed here and tagged like every
//! other, and no registry holds it ([`is_derived_tag`]), so no key a bridge
//! writes lands on it: `AskPx` still reaches `OfferPx(133)` through the
//! dictionary's word aliases. A market or operation column FIX names alike -
//! `Price(44)`, `Side(54)`, `TimeInForce(59)` - is that FIX field, and is
//! never tagged twice.
//!
//! # Why 65000, and why each is a tag and a name
//!
//! Their tags sit above every tag FIX publishes and above the user-defined
//! ranges venues share, so they collide with nothing a dictionary declares
//! and belong to no dialect: `currunix` is one identity in every dictionary, a
//! bridge row spelling `SESSIONID` lands on the crate's own column, and a
//! name every registry carries is never an unknown key. [`is_crate_tag`] is
//! the whole test. A field is its tag and its name, so each is declared as
//! both - `MSGPLUGINID_TAG_NAME` is `(65_041, "msgpluginid")` - and a
//! caller reads the half it needs.
//!
//! # What is FIX's own is not here
//!
//! `MsgDirection` is not invented: FIX publishes it at tag 385, and a field
//! the specification already has is never given a second tag. What this crate
//! adds there is the *type* - the packed four bytes rather than a string -
//! which the dictionary carries like any other coded field.
//!
//! Every market fact is FIX's. The numbers a message is about are
//! `Price(44)`, `OrderQty(38)`, `Quantity(53)`, `LastPx(31)`, `LastQty(32)`,
//! `AvgPx(6)`, `CumQty(14)` and `LeavesQty(151)`, which a message holds
//! lifted beside its header, and so are the identifiers `ClOrdID(11)`,
//! `OrigClOrdID(41)`, `OrderID(37)`, `SecondaryOrderID(198)`, `ExecID(17)`,
//! `QuoteID(117)`, `QuoteReqID(131)`, `MDReqID(262)` and `TradeID(1003)`.
//! The currency, side and classification are `Currency(15)`, `Side(54)` and
//! `CFICode(461)`, the quote's `BidPx(132)`, `OfferPx(133)`,
//! `BidSize(134)` and `OfferSize(135)`, the instrument's identifiers
//! `SecurityID(48)` under `SecurityIDSource(22)` and the `SecurityAltID`
//! group, the market `SecurityExchange(207)`, `ExDestination(100)` and
//! `LastMkt(30)`, the unit `UnitOfMeasure(996)`, the state `OrdStatus(39)`
//! and `ExecType(150)`. The crate names what FIX does not.
//!
//! # Every registry holds them
//!
//! [`FixRegistry::new`](super::FixRegistry::new) inserts them - the derived
//! columns excepted - before anything else, so a dictionary loaded from a
//! store, built from fields or left empty answers `currunix` and
//! `msgpluginid` alike. A store writes their definitions
//! so another consumer sees the whole row, but a read passes those stored
//! copies over: the crate's own definition is the one that types a row.
//! Folding another dictionary in never counts them either.
//!
//! Fifty definitions - scalar fields and Map groups, each registered by
//! its shape, the derived columns excepted - and every one of them a row of
//! [`CRATED`].
//!
//! # Numbered in the fixed row's order
//!
//! The tags run contiguously from `65001` in the order [the fixed
//! row](super::fix_schema_tags) states its crate columns - the element's,
//! the event's, the market's and the operation's facts, then the message's
//! own: which message and session, which instrument - and then the
//! definitions no column of that row is: the capture's `sourceurl` and the
//! `fixmsg` document.
//! A retired definition leaves no gap, and a number never names two
//! definitions over time: the dictionary a store holds is written by this
//! crate and read back through it.

use std::sync::{Arc, LazyLock};

use smol_str::SmolStr;

use crate::graph::{ElementColumn, EventColumn, MarketColumn, OperationColumn};
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
pub const CURRUNIX_TAG_NAME: (i32, &str) = (65_007, "currunix");

/// The tag and name carrying the message context a bridge handled the
/// message in.
pub const MSGCTXID_TAG_NAME: (i32, &str) = (65_043, "msgctxid");

/// The tag and name carrying the plugin that logged the line, as a bridge
/// names it.
pub const MSGPLUGINID_TAG_NAME: (i32, &str) = (65_041, "msgpluginid");

/// The tag and name carrying the code the message's content digests to:
/// the XXH3-64 of what the event states and the named FIX content behind it.
pub const CURRHASHCODE_TAG_NAME: (i32, &str) = (65_004, "currhashcode");

/// The tag and name carrying the cross hash code: the XXH3-64 of the cross
/// code, zero where the message names none.
pub const CROSSHASHCODE_TAG_NAME: (i32, &str) = (65_005, "crosshashcode");

/// The tag and name carrying when the message this one follows happened.
pub const PREVUNIX_TAG_NAME: (i32, &str) = (65_011, "prevunix");

/// The tag and name carrying the identity of the message this one follows.
pub const PREVUUID_TAG_NAME: (i32, &str) = (65_013, "prevuuid");

/// The tag and name carrying when the message was created: what it states,
/// else when it happened. Once walked, a message stating no
/// `SendingTime(52)` that a `TransactTime(60)` stating a clock dates takes a
/// resend's `OrigSendingTime(122)` earlier than that transaction as its
/// creation; the parse reads no `OrigSendingTime`.
pub const CREAUNIX_TAG_NAME: (i32, &str) = (65_008, "creaunix");

/// The tag and name carrying the grid instant a walk read the message as
/// the snapshot of.
pub const SNAPUNIX_TAG_NAME: (i32, &str) = (65_012, "snapunix");

/// The tag and name carrying the object one line was read from.
///
/// Where a message was read from is not FIX and is exactly what a monitor
/// orders, joins and prunes on: which file of a day's capture a row came out
/// of, and therefore which file to re-read when a row is disputed. A text
/// read already answers it - its own `sourceurl` column is a URL - and this
/// is the FIX field that types that column, so a capture naming it states
/// the object once per line, typed rather than as text nobody can resolve.
///
/// It is not a column of [the fixed row](super::fix_schema). Where a line
/// was read from is the reader's word about the line and never the
/// message's about itself, so it travels beside the row as one of the
/// capture's own columns, with the body the line was cut from and its place
/// in the object, and no column of the message restates it.
pub const SOURCEURL_TAG_NAME: (i32, &str) = (65_050, "sourceurl");

/// The tag and name carrying the session instance a bridge handled a line on.
///
/// Not `sessionid`: that spelling is a bridge row's own `SESSIONID` key,
/// which names the counterparty session a message states it is on. This is
/// the bridge's own connection instance, and the two are separate facts -
/// one message states its session once, while two connections to one
/// counterparty are two instances.
pub const MSGSESSIONID_TAG_NAME: (i32, &str) = (65_044, "msgsessionid");

/// The tag and name carrying the message's identity: UUIDv7 ordered by its
/// millisecond and sequence, with a content payload seeded by its cross hash.
pub const CURRUUID_TAG_NAME: (i32, &str) = (65_001, "curruuid");

/// The tag and name carrying the identity every message of one lifecycle
/// shares, where the message names one.
pub const CROSSUUID_TAG_NAME: (i32, &str) = (65_002, "crossuuid");

/// The tag and name carrying the message's place among the messages of its
/// instant, stored as a null at place 0: the parse places by run - the
/// messages it hands over at one instant one after another, with no other
/// instant between - 0 for the first of a run and one more for each next,
/// so a message split off another stands at a later place than it, and a
/// stream coming back to an instant it left starts a run there at 0 again;
/// the lifecycle places what it walks by content, and a message following
/// one dated at its instant or later stands past it.
pub const SEQNUM_TAG_NAME: (i32, &str) = (65_014, "seqnum");

/// The tag and name carrying the cross code: the identifier every message
/// of one lifecycle shares, and what the cross hash code and the cross
/// identity derive from.
///
/// As the message spells it - its `OrderID`, else its `ClOrdID`,
/// `OrigClOrdID`, `QuoteID`, `QuoteReqID` or `MDReqID`, the first stated -
/// save for what the parse splits off: an execution split off a report
/// chains on its `ExecID` as given, else `TradeID=<TradeID>`, else its
/// report's code and content digest, one split off a trade on its side's
/// order and the side's first stated identifier, and a batch entry on the
/// order or entry it names. It is stored after the codes of the category
/// the message files under and of the side an order or an execution takes,
/// `10:1:ORD-1` and `10:2:ORD-1`, so each category, and each side of one
/// order or execution identifier, is a chain of its own; one of side
/// `UNKN`, and every other kind - a quote, whose side is a tag, among them -
/// states side `0`: `14:0:Q-1`, `21:0:T-1`. A followed message carries its
/// chain's.
pub const CROSSCODE_TAG_NAME: (i32, &str) = (65_003, "crosscode");

/// The tag and name of the Map group carrying what a message stated that
/// is no field: the keys a bridge states under its own namespaces - the
/// `TECH.` and `AMON.` keys and their kin - and, in a row, every key no
/// dictionary resolved; each under the key as it was spelled, folded, in
/// sorted order.
pub const METADATA_TAG_NAME: (i32, &str) = (65_036, "metadata");

/// The tag and name of the fixed row every message answers as, as a store
/// dumps it: `components/fixmsg.json` states the columns
/// [`fix_schema`](super::fix_schema) builds, each a reference to the field
/// or group that types it. The crate stays the one owner of that row - a
/// reader passes the document over, as it passes the crate's own fields -
/// so the dump is what a consumer reads the row's shape from without
/// running this crate, and nothing this crate reads back.
pub const FIXMSG_TAG_NAME: (i32, &str) = (65_051, "fixmsg");

/// The tag and name carrying the identities of the elements the message
/// was read from: its provenance, never its lineage.
///
/// A message parsed from a text line names that line's identity, a message
/// the parse split off another names that message's identity beside its
/// sources, and a message parsed from raw bytes names none. Sources travel
/// along no chain - following and restating leave them as they are - and
/// never feed the code the message digests to, because where a message was
/// read from is not what it states.
pub const SRCUUIDS_TAG_NAME: (i32, &str) = (65_006, "srcuuids");

/// The state the event reached: the `uint16` code of a lifecycle-sorted enum,
/// so the column sorts from the first state to the terminal ones.
///
/// `FILLED` on an execution the parse split off, whatever the content it
/// carries states; otherwise read, as a message is built, off the first
/// status field that states one - `OrdStatus(39)`, `ExecType(150)`,
/// `ExecAckStatus(1036)`, `TrdRptStatus(939)`, `QuoteStatus(297)`,
/// `AllocStatus(87)`, `ConfirmStatus(665)`, `AffirmStatus(940)`,
/// `MassActionResponse(1375)`, `MassCancelResponse(531)` - else off what the
/// message type asks for, and `UNKNOWN` where nothing states one; the
/// furthest its chain knows once the lifecycle followed it,
/// and a row stating one is the row's word. A column, because a monitor
/// asking which orders are still live reads one ranked column rather than
/// ten code sets. The intrinsic `statecodeset` names what each code stands
/// for.
pub const STATE_TAG_NAME: (i32, &str) = (65_015, "state");

/// The crate-owned vocabulary the `state` column reads by: every member of
/// [`crate::State`], its stored name and the code it stores.
pub(super) const STATE_CODESET_NAME: &str = "statecodeset";

/// The canonical state document every registry shares.
static STATE_CODESET: LazyLock<Option<Arc<str>>> = LazyLock::new(|| {
    let codes = crate::State::ALL
        .iter()
        .map(|state| {
            super::FixCode::new(state.as_str(), state.code().to_string())
                .with_description(state.description())
        })
        .collect::<Vec<_>>();
    match super::FixCodes::render(&codes) {
        Ok(document) => Some(Arc::from(document)),
        Err(error) => {
            log::warn!("building FIX state code set: {error}");
            None
        }
    }
});

pub(super) fn state_codeset() -> Option<Arc<str>> {
    STATE_CODESET.as_ref().map(Arc::clone)
}

/// When the message stops being good, where it does.
///
/// `ExpireTime(126)`, else `ValidUntilTime(62)`, else the end of the day
/// `ExpireDate(432)` names - the last day an order can trade - the first
/// stated, as a message is built; the previous deadline when the next event
/// states none; a newer explicit deadline replaces it, including when it
/// shortens the lifetime. `MaturityDate(541)` is the instrument's, no
/// message's deadline.
pub const EXPRUNIX_TAG_NAME: (i32, &str) = (65_010, "exprunix");

/// The tag and name carrying the business category of the message type, as
/// the member of [`crate::MarketDataKind`] its code stores: the one
/// [`Market::marketdatakind`](crate::graph::Market::marketdatakind) answers on a message, under the
/// [`MarketColumn`] name every market row states it by.
pub const MARKETDATAKIND_TAG_NAME: (i32, &str) = (65_016, "marketdatakind");
/// The crate-owned vocabulary registered for the `marketdatakind` column to
/// read by:
/// every member of [`crate::MarketDataKind`], its stored name, the code it
/// stores and what it stands for. Intrinsic and immutable in a registry.
pub(super) const MARKETDATAKIND_CODESET_NAME: &str = "marketdatakindcodeset";

/// The canonical `marketdatakind` document every registry shares.
static MARKETDATAKIND_CODESET: LazyLock<Option<Arc<str>>> = LazyLock::new(|| {
    let codes = crate::MarketDataKind::ALL
        .iter()
        .map(|kind| {
            super::FixCode::new(kind.as_str(), kind.code().to_string())
                .with_description(kind.description())
        })
        .collect::<Vec<_>>();
    match super::FixCodes::render(&codes) {
        Ok(document) => Some(Arc::from(document)),
        Err(error) => {
            log::warn!("building FIX marketdatakind code set: {error}");
            None
        }
    }
});

pub(super) fn marketdatakind_codeset() -> Option<Arc<str>> {
    MARKETDATAKIND_CODESET.as_ref().map(Arc::clone)
}

/// The type of its kind the message is, as the member of
/// [`crate::MarketDataType`] its code stores: its order type (`ORDLIMIT`),
/// quote type, trade type or book entry type, read off `OrdType(40)`,
/// `QuoteType(537)`, `TrdType(828)` or `MDEntryType(269)` - the one its kind
/// names first - or any field a registry maps through `FIX:marketdatatype`.
pub const MARKETDATATYPE_TAG_NAME: (i32, &str) = (65_017, "marketdatatype");

/// The crate-owned vocabulary the `marketdatatype` column reads by: every
/// member of [`crate::MarketDataType`], its stored name, the code it stores
/// and what it stands for. Intrinsic and immutable in a registry.
pub(super) const MARKETDATATYPE_CODESET_NAME: &str = "marketdatatypecodeset";

/// The canonical market data type document every registry shares.
static MARKETDATATYPE_CODESET: LazyLock<Option<Arc<str>>> = LazyLock::new(|| {
    let codes = crate::MarketDataType::ALL
        .iter()
        .map(|mdtype| {
            super::FixCode::new(mdtype.as_str(), mdtype.code().to_string())
                .with_description(mdtype.description())
        })
        .collect::<Vec<_>>();
    match super::FixCodes::render(&codes) {
        Ok(document) => Some(Arc::from(document)),
        Err(error) => {
            log::warn!("building FIX market data type code set: {error}");
            None
        }
    }
});

pub(super) fn marketdatatype_codeset() -> Option<Arc<str>> {
    MARKETDATATYPE_CODESET.as_ref().map(Arc::clone)
}
/// The tag and name carrying the normalized ISIN the message identifies.
pub const ISINCODE_TAG_NAME: (i32, &str) = (65_021, "isincode");
/// The tag and name carrying the normalized Bloomberg identifier the message identifies.
pub const BLOOMBERGCODE_TAG_NAME: (i32, &str) = (65_048, "bloombergcode");
/// The tag and name carrying the normalized market MIC the message identifies.
pub const MICCODE_TAG_NAME: (i32, &str) = (65_022, "miccode");
/// The tag and name carrying the normalized FIGI the message identifies.
pub const FIGICODE_TAG_NAME: (i32, &str) = (65_049, "figicode");
/// The tag and name carrying when the message last executed, where one of
/// its FIX facts states it - a `TransactTime(60)` stating a day alone
/// states none - else - on a message reporting an execution that states
/// none and follows nothing - when the message happened; the latest its
/// chain reached once followed.
pub const EXECUNIX_TAG_NAME: (i32, &str) = (65_023, "execunix");

/// The tag and name carrying when the message was recorded: by its carrier,
/// where a carrier states one, else by its sender, where it states its
/// `SendingTime(52)`.
pub const RECDUNIX_TAG_NAME: (i32, &str) = (65_009, "recdunix");

/// The tag and name carrying the session event a bridge delivered the
/// message as: its `MsgType(35)`, session instance, message context and
/// `MsgSeqNum(34)` joined by `:`, where all four are stated.
pub const MSGSESSEVENTID_TAG_NAME: (i32, &str) = (65_045, "msgsesseventid");

/// The tag and name carrying the plugin a message came into a bridge
/// through, as the bridge's own log line names it: provenance the capture
/// holds, never content and never a digest input.
pub const MSGORIGINATOR_TAG_NAME: (i32, &str) = (65_042, "msgoriginator");

/// The tag and name carrying the conversation a bridge filed the message
/// under, as it stated it: provenance the capture holds, never content.
pub const CONVERSATIONID_TAG_NAME: (i32, &str) = (65_046, "conversationid");

/// The tag and name carrying the currency pair the message is about,
/// canonical `CCY1/CCY2`: the `FOREX` entry of the message's security
/// identifiers, a view of `get_securityids()` - the pair `get` answered
/// when the row was written - detected off `Symbol(55)`, from `derived`,
/// where the message states no other class. Read back from a row with no
/// `securityids` column, a pair the message's reading answers states
/// nothing, the pair its symbol names where nothing else names one is that
/// detection, and any other replaces its type's answer, the base key.
pub const FOREXCODE_TAG_NAME: (i32, &str) = (65_047, "forexcode");

/// The tag and name carrying the part of the quantity an iceberg keeps from
/// the market, which the row derives: the quantity past `DisplayQty(1138)`,
/// else past `MaxFloor(111)`.
pub const HIDDENQTY_TAG_NAME: (i32, &str) = (65_018, "hiddenqty");

/// The tag and name carrying the unit the quantity is counted in, which
/// the row derives from `UnitOfMeasure(996)`.
pub const UNIT_TAG_NAME: (i32, &str) = (65_019, "unit");

/// The tag and name carrying the security identifiers the message names,
/// each a type, a source and a code, which the row derives from `SecurityID(48)` under
/// `SecurityIDSource(22)`, the `SecurityAltID` group and the crate's
/// normalized instrument codes.
pub const SECURITYIDS_TAG_NAME: (i32, &str) = (65_020, "securityids");

/// The tag and name carrying the price the step before the message settled
/// on, which the row derives from `PrevClosePx(140)`.
pub const PREVPX_TAG_NAME: (i32, &str) = (65_024, "prevpx");

/// The tag and name carrying the quantity the step before the message
/// settled on, which only a lifecycle states.
pub const PREVQTY_TAG_NAME: (i32, &str) = (65_025, "prevqty");

/// The tag and name carrying the spot part of an FX forward price, which
/// the row derives from `LastSpotRate(194)`; a quote's `BidSpotRate(188)`
/// and `OfferSpotRate(190)` are its legs', which stay its own fields.
pub const SPOTRATE_TAG_NAME: (i32, &str) = (65_026, "spotrate");

/// The tag and name carrying the forward points of an FX forward price,
/// which the row derives from `LastForwardPoints(195)`; a quote's
/// `BidForwardPoints(189)` and `OfferForwardPoints(191)` are its legs',
/// which stay its own fields.
pub const FORWARDPOINTS_TAG_NAME: (i32, &str) = (65_027, "forwardpoints");

/// The tag and name carrying the quantity bid, which the row derives from
/// `BidSize(134)`.
pub const BIDQTY_TAG_NAME: (i32, &str) = (65_028, "bidqty");

/// The tag and name carrying the currency the bid is stated in, which the
/// row derives from a bridge's bid currency, else the message's own where
/// it states a bid.
pub const BIDCCY_TAG_NAME: (i32, &str) = (65_029, "bidccy");

/// The tag and name carrying the ask price, which the row derives from
/// `OfferPx(133)`.
pub const ASKPX_TAG_NAME: (i32, &str) = (65_030, "askpx");

/// The tag and name carrying the quantity offered, which the row derives
/// from `OfferSize(135)`.
pub const ASKQTY_TAG_NAME: (i32, &str) = (65_031, "askqty");

/// The tag and name carrying the currency the ask is stated in, which the
/// row derives from a bridge's ask currency, else the message's own where
/// it states an ask.
pub const ASKCCY_TAG_NAME: (i32, &str) = (65_032, "askccy");

/// The tag and name carrying the FX rates the message states, target
/// currency to the rate an amount is divided by, which no FIX field fills.
pub const FXRATES_TAG_NAME: (i32, &str) = (65_033, "fxrates");

/// The tag and name carrying the ticker the instrument goes by, which the
/// row derives from `Symbol(55)`.
pub const TICKER_TAG_NAME: (i32, &str) = (65_034, "ticker");

/// The tag and name carrying the strike price of the option the message
/// identifies, which the row derives from `StrikePrice(202)`.
pub const STRIKEPX_TAG_NAME: (i32, &str) = (65_035, "strikepx");

/// The tag and name carrying the quantity the operation ordered, which the
/// row derives from `OrderQty(38)`.
pub const ORDQTY_TAG_NAME: (i32, &str) = (65_037, "ordqty");

/// The tag and name carrying whether the instrument can trade, which the
/// row derives from `SecurityTradingStatus(326)`, `TradingSessionStatus(340)`
/// or `SecurityStatus(965)`.
pub const TRADABLE_TAG_NAME: (i32, &str) = (65_038, "tradable");

/// The tag and name carrying the operation's alternate identifiers, each
/// typed under the `FIX` source, which the row derives from the fields the
/// dictionary files under `FIX:idmap`.
pub const IDENTIFIERS_TAG_NAME: (i32, &str) = (65_039, "identifiers");

/// The tag and name carrying the parties the operation names, each typed by
/// its role and sourced by its issuer, which the row derives from the
/// `Parties` groups and `Account(1)`.
pub const PARTYIDS_TAG_NAME: (i32, &str) = (65_040, "partyids");

/// The graph element column one crate tag is, for the six that are one.
///
/// The element facts a row states are read and written through the column,
/// [`ElementColumn::fact`] and [`ElementColumn::record`], so a FIX row and
/// a text line's batch answer one cell for one fact. [`CRATED`] is where
/// the pairing is stated, so a column and its tag are never written twice.
#[must_use]
pub(super) fn element_column_of(tag: i32) -> Option<ElementColumn> {
    match crated(tag)?.holds {
        Holds::Element(column) => Some(column),
        _ => None,
    }
}

/// The graph event column one crate tag is, for the nine that are one,
/// read and written through [`EventColumn::fact`] and
/// [`EventColumn::record`] as the element columns are through theirs.
#[must_use]
pub(super) fn event_column_of(tag: i32) -> Option<EventColumn> {
    match crated(tag)?.holds {
        Holds::Event(column) => Some(column),
        _ => None,
    }
}

/// The graph market column one crate tag is, for the ones the crate tags:
/// every [`MarketColumn`] no dictionary field already carries under its
/// name, read and written through [`MarketColumn::fact`] and
/// [`MarketColumn::record`].
#[must_use]
pub(super) fn market_column_of(tag: i32) -> Option<MarketColumn> {
    match crated(tag)?.holds {
        Holds::Market(column) => Some(column),
        _ => None,
    }
}

/// The graph operation column one crate tag is, for the four that are one,
/// read and written through [`OperationColumn::fact`] and
/// [`OperationColumn::record`].
#[must_use]
pub(super) fn operation_column_of(tag: i32) -> Option<OperationColumn> {
    match crated(tag)?.holds {
        Holds::Operation(column) => Some(column),
        _ => None,
    }
}

/// The crate tag a column of this name is stated under, where the crate
/// tags one: every element and event column, and the market and operation
/// columns no dictionary field already carries under their name.
pub(super) fn tag_named(name: &str) -> Option<i32> {
    CRATED
        .iter()
        .find(|held| held.tag_name.1 == name)
        .map(|held| held.tag_name.0)
}

/// Whether a crate tag is one of the columns the fixed row derives from
/// the message's own fields: a market or an operation fact the dictionary
/// states under another name, restated under the name every market row
/// states it by. No registry holds one, so no key a bridge or a capture
/// writes lands on it - `AskPx` still reaches `OfferPx(133)` - and a row
/// stating one states the message's fact, as a lifecycle row states what
/// its walk folded forward; a row stating none leaves it to the fields.
#[must_use]
pub fn is_derived_tag(tag: i32) -> bool {
    crated(tag).is_some_and(|held| held.derived)
}

/// The row of [`CRATED`] a crate tag is, by its offset from
/// [`CRATE_TAG_MIN`]: what every per-row reading of a crate tag dispatches
/// through, one index rather than a pass over every definition.
fn crated(tag: i32) -> Option<&'static Crated> {
    let at = CRATED_AT.get(usize::try_from(tag.checked_sub(CRATE_TAG_MIN)?).ok()?)?;
    CRATED.get(usize::from(*at))
}

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
/// the place at its instant and the element it follows, because a walk states
/// them per message; the state it reached and when it expires, because a
/// walk folds them per message; the session event it was delivered as,
/// because it joins this message's own type and sequence; `sourceurl` and
/// `srcuuids`, because they are facts about the line this row was read
/// from; and every derived market and operation column, because each is
/// read again off the fields of the message that carries it.
///
/// Everything else the crate owns is about the session or the chain the
/// message stands in - the identifiers it resolved, the keys a bridge
/// stated, the plugin, the context and the session instance - and carries.
const SETTLED_TO_ONE_MESSAGE: [i32; 38] = [
    MARKETDATATYPE_TAG_NAME.0,
    HIDDENQTY_TAG_NAME.0,
    UNIT_TAG_NAME.0,
    SECURITYIDS_TAG_NAME.0,
    PREVPX_TAG_NAME.0,
    PREVQTY_TAG_NAME.0,
    SPOTRATE_TAG_NAME.0,
    FORWARDPOINTS_TAG_NAME.0,
    BIDQTY_TAG_NAME.0,
    BIDCCY_TAG_NAME.0,
    ASKPX_TAG_NAME.0,
    ASKQTY_TAG_NAME.0,
    ASKCCY_TAG_NAME.0,
    FXRATES_TAG_NAME.0,
    TICKER_TAG_NAME.0,
    STRIKEPX_TAG_NAME.0,
    ORDQTY_TAG_NAME.0,
    TRADABLE_TAG_NAME.0,
    IDENTIFIERS_TAG_NAME.0,
    PARTYIDS_TAG_NAME.0,
    CURRUNIX_TAG_NAME.0,
    EXECUNIX_TAG_NAME.0,
    RECDUNIX_TAG_NAME.0,
    MSGSESSEVENTID_TAG_NAME.0,
    CREAUNIX_TAG_NAME.0,
    SNAPUNIX_TAG_NAME.0,
    PREVUNIX_TAG_NAME.0,
    EXPRUNIX_TAG_NAME.0,
    STATE_TAG_NAME.0,
    PREVUUID_TAG_NAME.0,
    CURRHASHCODE_TAG_NAME.0,
    CROSSHASHCODE_TAG_NAME.0,
    CURRUUID_TAG_NAME.0,
    CROSSUUID_TAG_NAME.0,
    CROSSCODE_TAG_NAME.0,
    SEQNUM_TAG_NAME.0,
    SOURCEURL_TAG_NAME.0,
    SRCUUIDS_TAG_NAME.0,
];

/// The crate's own columns every message states: the instants the identity
/// is settled against, the codes and the identity it settles to - the cross
/// code among them, the empty text where the message names none - and the
/// place it holds among the messages of its instant - zero for the first,
/// so never absent.
///
/// The state a message reached is stated on every row a message writes -
/// `UNKNOWN` where nothing states one - but the column admits a null,
/// because a state has no neutral member for an empty cell to read as, and
/// a column no default can fill is not one a row can be required to state.
const ALWAYS_STATED: [i32; 8] = [
    CURRUNIX_TAG_NAME.0,
    CREAUNIX_TAG_NAME.0,
    CURRHASHCODE_TAG_NAME.0,
    CROSSHASHCODE_TAG_NAME.0,
    CROSSCODE_TAG_NAME.0,
    CURRUUID_TAG_NAME.0,
    CROSSUUID_TAG_NAME.0,
    SEQNUM_TAG_NAME.0,
];

/// Whether `tag` is one of the crate's own columns every message states,
/// which the field and the fixed row both declare required.
pub(super) fn is_always_stated(tag: i32) -> bool {
    ALWAYS_STATED.contains(&tag)
}

/// Where one definition's datatype, display and wording come from.
enum Holds {
    /// One of the six [`ElementColumn`]s, which owns all three: a text
    /// line's batch, a FIX row and a `marketdata` row then carry one column
    /// under one name, one datatype and one sentence, and join on it.
    Element(ElementColumn),
    /// One of the nine [`EventColumn`]s, which owns all three the same way.
    Event(EventColumn),
    /// A [`MarketColumn`], which owns the datatype and the display: the
    /// wording is the definition's own, since a market column states none.
    Market(MarketColumn),
    /// An [`OperationColumn`], which owns the datatype and the display the
    /// same way a market column does.
    Operation(OperationColumn),
    /// A fact no graph event states - what a bridge's row header said and
    /// the session event it joins to, where the line was read from, the
    /// normalized identifiers - which therefore spells its
    /// own.
    Own {
        datatype: fn() -> Result<DataType>,
        display: &'static str,
        description: &'static str,
    },
}

/// One definition of the crate's own, declared once.
///
/// A definition is its tag and its name and what it holds; everything else
/// is a rule read off one of those. [`SETTLED_TO_ONE_MESSAGE`] and
/// [`ALWAYS_STATED`] say which columns carry forward and which a row must
/// state, a Map counts itself, and an event column already owns its
/// datatype, its display and its wording. What is left to a row is the tag
/// that reaches the column and the three things only FIX knows: the fields
/// a value is read off, a second spelling, and a vocabulary. So a column is
/// one row of [`CRATED`], and adding one is adding a row.
struct Crated {
    /// The tag and the name, exactly the pair the public constant declares.
    tag_name: (i32, &'static str),
    /// Where the datatype, the display and the wording come from.
    holds: Holds,
    /// What the field says it is, where FIX names the fields a value is read
    /// off and the column - which is every medium's, not this one's - cannot.
    fix_wording: Option<&'static str>,
    /// The spellings a registry also answers this field by.
    names: &'static [&'static str],
    /// The registry vocabulary it reads by, where it reads by one.
    codeset: Option<&'static str>,
    /// Whether the fixed row derives it from the message's own fields
    /// rather than holding it as a field: see [`is_derived_tag`].
    derived: bool,
}

impl Crated {
    /// One definition holding `holds` under the crate's own tag.
    const fn holding(tag_name: (i32, &'static str), holds: Holds) -> Self {
        Self {
            tag_name,
            holds,
            fix_wording: None,
            names: &[],
            codeset: None,
            derived: false,
        }
    }

    /// One graph element column under the crate's own tag.
    const fn element(tag_name: (i32, &'static str), column: ElementColumn) -> Self {
        Self::holding(tag_name, Holds::Element(column))
    }

    /// One graph event column under the crate's own tag.
    const fn event(tag_name: (i32, &'static str), column: EventColumn) -> Self {
        Self::holding(tag_name, Holds::Event(column))
    }

    /// One graph market column under the crate's own tag, saying what FIX
    /// states of it: the fields its value is read off, which the column's
    /// own wording cannot name.
    const fn market(
        tag_name: (i32, &'static str),
        column: MarketColumn,
        description: &'static str,
    ) -> Self {
        Self::holding(tag_name, Holds::Market(column)).saying(description)
    }

    /// One graph market column the fixed row derives from the message's
    /// fields, saying which.
    const fn derived_market(
        tag_name: (i32, &'static str),
        column: MarketColumn,
        description: &'static str,
    ) -> Self {
        let mut crated = Self::market(tag_name, column, description);
        crated.derived = true;
        crated
    }

    /// One graph operation column the fixed row derives from the message's
    /// fields, saying which.
    const fn derived_operation(
        tag_name: (i32, &'static str),
        column: OperationColumn,
        description: &'static str,
    ) -> Self {
        let mut crated = Self::holding(tag_name, Holds::Operation(column)).saying(description);
        crated.derived = true;
        crated
    }

    /// One fact no graph event states, spelling its own datatype, display
    /// and wording.
    const fn own(
        tag_name: (i32, &'static str),
        datatype: fn() -> Result<DataType>,
        display: &'static str,
        description: &'static str,
    ) -> Self {
        Self::holding(
            tag_name,
            Holds::Own {
                datatype,
                display,
                description,
            },
        )
    }

    /// The same definition, saying what FIX states and the column cannot:
    /// the fields the value is read off, in the order they are read.
    const fn saying(mut self, description: &'static str) -> Self {
        self.fix_wording = Some(description);
        self
    }

    /// The same definition, which a registry also answers by these spellings.
    const fn also_called(mut self, names: &'static [&'static str]) -> Self {
        self.names = names;
        self
    }

    /// The same definition, reading its values by a registry vocabulary.
    const fn reading(mut self, codeset: &'static str) -> Self {
        self.codeset = Some(codeset);
        self
    }

    /// This definition as the field every registry holds - or, derived, as
    /// the column the fixed row states it in.
    ///
    /// Display and description use generic field metadata rather than the
    /// `FIX:` scheme because every catalog the crate writes to understands
    /// them.
    fn field(&self) -> Result<Field> {
        let (tag, name) = self.tag_name;
        let (dtype, display, description) = match self.holds {
            Holds::Element(column) => (column.datatype(), column.display(), column.description()),
            Holds::Event(column) => (column.datatype(), column.display(), column.description()),
            Holds::Market(column) => (column.datatype(), column.display(), column.description()),
            Holds::Operation(column) => (column.datatype(), column.display(), column.description()),
            Holds::Own {
                datatype,
                display,
                description,
            } => (datatype()?, display, description),
        };
        // A crate Map is a group whose occurrence is its own entries Struct,
        // so the tag counting it is the tag it is: there is no second tag to
        // state, and no row that could state a different one. A derived Map
        // is no group of any message: the event answers it whole.
        let counts_itself =
            !self.derived && matches!(dtype, DataType::Map(_) | DataType::SortedMap(_));
        let mut field = Field::new(name, dtype, !is_always_stated(tag));
        field.as_fix_mut().set_tag(tag)?;
        field.set_display(display)?;
        field.set_description(self.fix_wording.unwrap_or(description))?;
        if counts_itself {
            field.as_fix_mut().set_counter(tag)?;
        }
        if !self.names.is_empty() {
            field.as_fix_mut().set_names(self.names.iter().copied())?;
        }
        if let Some(codeset) = self.codeset {
            field.as_fix_mut().set_codeset(codeset)?;
        }
        if SETTLED_TO_ONE_MESSAGE.contains(&tag) {
            field.as_fix_mut().set_transient(false)?;
        }
        Ok(field)
    }
}

/// Every definition this crate invents, in tag order.
///
/// The order is the tags', because that is the order a schema, a document
/// and [`fix_crate_fields`] all walk them in - and the tags are numbered in
/// the fixed row's band order, so this is that order too. A row that only names a tag
/// and a column is a column this crate adds nothing to but the tag.
const CRATED: [Crated; 50] = [
    Crated::element(CURRUUID_TAG_NAME, ElementColumn::CurrUuid),
    Crated::element(CROSSUUID_TAG_NAME, ElementColumn::CrossUuid),
    Crated::element(CROSSCODE_TAG_NAME, ElementColumn::CrossCode).saying(
        "The identifier every message of one lifecycle shares: OrderID, \
         else ClOrdID, OrigClOrdID, QuoteID, QuoteReqID or MDReqID, the \
         first stated - an execution the parse splits off a report its \
         ExecID, else TradeID=<TradeID>, else its report's code and content \
         digest, one split off a trade its side's order and the side's first \
         stated identifier, a batch entry the order or entry it names - \
         stored after the codes of its category and of the side an order or \
         an execution takes, 10:1:ORD-1, side 0 on one of side UNKN and on \
         every other kind, a quote among them: 14:0:Q-1, 21:0:T-1; its \
         chain's once followed.",
    ),
    Crated::element(CURRHASHCODE_TAG_NAME, ElementColumn::CurrHashCode).saying(
        "The XXH3-64 of what the event states and the named FIX content \
         behind it.",
    ),
    Crated::element(CROSSHASHCODE_TAG_NAME, ElementColumn::CrossHashCode),
    Crated::element(SRCUUIDS_TAG_NAME, ElementColumn::SrcUuids).saying(
        "The identities of the elements this message was read from: the \
         text line it was parsed out of, and for a message the parse split \
         off another that message's identity beside its sources; none for \
         one parsed from raw bytes. Provenance, never lineage: no walk \
         moves it.",
    ),
    Crated::event(CURRUNIX_TAG_NAME, EventColumn::CurrUnix),
    Crated::event(CREAUNIX_TAG_NAME, EventColumn::CreaUnix)
        .saying(
            "When the message was created: what it states, else when it \
             happened; once walked, a message that states no SendingTime \
             and is dated by a TransactTime takes an earlier \
             OrigSendingTime as its creation; the earliest its chain knows \
             once followed.",
        )
        .also_called(&["CreationTime"]),
    Crated::event(RECDUNIX_TAG_NAME, EventColumn::RecdUnix).saying(
        "When the message was recorded: by its carrier, where the carrier \
         states one, else by its sender, where it states a SendingTime; the \
         earliest its statements know.",
    ),
    Crated::event(EXPRUNIX_TAG_NAME, EventColumn::ExprUnix).saying(
        "When the message stops being good: ExpireTime, else \
         ValidUntilTime, else the end of the day ExpireDate names; \
         MaturityDate is the instrument's, no deadline; a newer explicit \
         deadline replaces the one its chain carried.",
    ),
    Crated::event(PREVUNIX_TAG_NAME, EventColumn::PrevUnix),
    Crated::event(SNAPUNIX_TAG_NAME, EventColumn::SnapUnix),
    Crated::event(PREVUUID_TAG_NAME, EventColumn::PrevUuid),
    Crated::event(SEQNUM_TAG_NAME, EventColumn::SeqNum).saying(
        "The message's place among the messages of its instant, never null: \
         0 for the first of each run the parse hands over at that instant \
         with no other instant between, one more for each next, a message \
         split off another at a later place than it; once walked, the \
         place its content took at that instant, past a predecessor dated \
         there or later.",
    ),
    Crated::event(STATE_TAG_NAME, EventColumn::State)
        .saying(
            "The state the message reached, as the code of a lifecycle-sorted \
             enum: FILLED on an execution the parse split off; otherwise the \
             first of OrdStatus, ExecType, ExecAckStatus, TrdRptStatus, \
             QuoteStatus, AllocStatus, ConfirmStatus, AffirmStatus, \
             MassActionResponse or MassCancelResponse that states one, else \
             what its message type asks for, UNKNOWN where none does; the \
             furthest its chain knows once followed, and a row stating one \
             is the row's word.",
        )
        .reading(STATE_CODESET_NAME),
    Crated::market(
        MARKETDATAKIND_TAG_NAME,
        MarketColumn::MarketDataKind,
        "The business category of the message type, as the member of the \
         marketdatakind enum: the dictionary's FIX:msgcat for the type, UNKN \
         where it files none; an execution report its order's ORDR, or its \
         quote's QUOT where it names a QuoteID, where it reports no fill or \
         once the parse split its fill off as an EXEC, and a batch entry the \
         parse split off its item's; a row stating one is the row's word.",
    )
    .reading(MARKETDATAKIND_CODESET_NAME),
    Crated::market(
        MARKETDATATYPE_TAG_NAME,
        MarketColumn::MarketDataType,
        "The type of its kind the message is, as the member of the \
         marketdatatype enum: its order type from OrdType, its quote type \
         from QuoteType, its trade type from TrdType, its book entry type \
         from MDEntryType - the field its kind names first, each value read \
         through the dictionary's FIX:marketdatatype where it maps one - \
         UNKN where none is stated; a row stating one is the row's word.",
    )
    .reading(MARKETDATATYPE_CODESET_NAME),
    Crated::derived_market(
        HIDDENQTY_TAG_NAME,
        MarketColumn::HiddenQty,
        "The part of the quantity an iceberg keeps from the market: the quantity past DisplayQty, else past MaxFloor, where it shows less than all of it. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        UNIT_TAG_NAME,
        MarketColumn::Unit,
        "The unit the quantity is counted in: UnitOfMeasure, the empty unit where none is stated. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        SECURITYIDS_TAG_NAME,
        MarketColumn::SecurityIds,
        "The security identifiers the message names, each a type, a source and a code: SecurityID under SecurityIDSource, the SecurityAltID group, a bridge's instrument key, the keyed aliases a bridge states - ISINCODE, OMS_RICCODE, SEDOL_CODE - and the codes an ISIN embeds. SecurityID, SecurityIDSource and the group stay in fixentries as sent, and a keyed alias under 0:key. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::own(
        ISINCODE_TAG_NAME,
        || Ok(DataType::isin()),
        "ISIN Code",
        "The normalized ISIN the message identifies.",
    ),
    Crated::own(
        MICCODE_TAG_NAME,
        || Ok(DataType::mic()),
        "MIC Code",
        "The normalized market MIC the message identifies: LastMkt, else \
         ExDestination, the market a bridge's instrument key names or \
         SecurityExchange, the first an ISO 10383 MIC or a Reuters \
         mnemonic resolving to one, and XXXX for a currency pair naming \
         none; a bridge's INSTRUMENT[EXCHANGE] states it.",
    )
    .also_called(&["instrument[exchange]"]),
    Crated::market(
        EXECUNIX_TAG_NAME,
        MarketColumn::ExecUnix,
        "When the message last executed: what it states, else \
         ExecutionTimestamp, an execution TrdRegTimestamp, a proprietary \
         EventTimestamp or, on a message reporting an execution, a \
         TransactTime stating a clock, the first one stated - a day alone \
         dates no execution; a message reporting an execution that states \
         none and follows nothing executed at its currunix; the latest its \
         chain reached once followed, and the earliest two statements of it \
         know.",
    ),
    Crated::derived_market(
        PREVPX_TAG_NAME,
        MarketColumn::PrevPx,
        "The price the step before the message settled on: PrevClosePx, the one its lifecycle moved from once followed. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        PREVQTY_TAG_NAME,
        MarketColumn::PrevQty,
        "The quantity the step before the message settled on, which only a lifecycle states. Derived; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        SPOTRATE_TAG_NAME,
        MarketColumn::SpotRate,
        "The spot part of an FX forward price: LastSpotRate. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        FORWARDPOINTS_TAG_NAME,
        MarketColumn::ForwardPoints,
        "The forward points of an FX forward price: LastForwardPoints. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        BIDQTY_TAG_NAME,
        MarketColumn::BidQty,
        "The quantity bid: BidSize. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        BIDCCY_TAG_NAME,
        MarketColumn::BidCcy,
        "The currency the bid is stated in: a bridge's bid currency, else the message's own where it states a bid. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        ASKPX_TAG_NAME,
        MarketColumn::AskPx,
        "The ask price: OfferPx. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        ASKQTY_TAG_NAME,
        MarketColumn::AskQty,
        "The quantity offered: OfferSize. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        ASKCCY_TAG_NAME,
        MarketColumn::AskCcy,
        "The currency the ask is stated in: a bridge's ask currency, else the message's own where it states an ask. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        FXRATES_TAG_NAME,
        MarketColumn::FxRates,
        "The FX rates the message states, target currency to the rate an amount is divided by, which no FIX field fills. Derived; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        TICKER_TAG_NAME,
        MarketColumn::Ticker,
        "The ticker the instrument goes by: Symbol, trimmed. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_market(
        STRIKEPX_TAG_NAME,
        MarketColumn::StrikePx,
        "The strike price of the option the message identifies: StrikePrice, the one its chain names for the same instrument once followed. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::own(
        METADATA_TAG_NAME,
        || DataType::map_of(DataType::utf8(), DataType::utf8(), true),
        "Metadata",
        "What a message stated that is no field and no identifier map holds: \
         a bridge's namespaced keys - a `TECH.` or an `AMON.` key - and every \
         key no dictionary resolved, each under the key as it was spelled, \
         folded, in sorted order; a key an identifier map holds with its \
         value - a bridge's TECH.CLIENTID, its PARENTORDERID - rides \
         fixentries under 0:key as it arrived instead.",
    ),
    Crated::derived_operation(
        ORDQTY_TAG_NAME,
        OperationColumn::OrdQty,
        "The quantity the operation ordered: OrderQty; working, CumQty plus LeavesQty where it states none. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_operation(
        TRADABLE_TAG_NAME,
        OperationColumn::Tradable,
        "Whether the instrument can trade: SecurityTradingStatus, else TradingSessionStatus or SecurityStatus. Derived from the message's fields; a row stating one is the row's word.",
    ),
    Crated::derived_operation(
        IDENTIFIERS_TAG_NAME,
        OperationColumn::Identifiers,
        "The operation's alternate identifiers, each a type, the FIX source and an identifier: the fields the dictionary files under FIX:idmap, the parties that name one, and the keys no dictionary resolved whose names spell one - a bridge's PARENTORDERID, its firm.x.ParentOrderID - which ride fixentries under 0:key. Derived from the message's fields; a set a row states keeps its own and takes what it lacks.",
    ),
    Crated::derived_operation(
        PARTYIDS_TAG_NAME,
        OperationColumn::PartyIds,
        "The parties the operation names, each a role, a source and an identifier: the Parties and RootParties groups, Account under AcctIDSource, and the keys no dictionary resolved whose names spell a party - a bridge's OMS_UserID, its TECH.CLIENTID - which ride fixentries under 0:key. The groups and Account stay in fixentries as sent. Derived from the message's fields; a set a row states keeps its own and takes what it lacks.",
    ),
    Crated::own(
        MSGPLUGINID_TAG_NAME,
        || Ok(DataType::utf8()),
        "Message Plugin ID",
        "The plugin that logged the line inside a bridge, as the bridge \
         names it: the row's own msgpluginid column, never derived.",
    ),
    Crated::own(
        MSGORIGINATOR_TAG_NAME,
        || Ok(DataType::utf8()),
        "Message Originator",
        "The plugin a message came into a bridge through, as the bridge's own \
         log line names it: a message received from (X as ...), an execution \
         report from X, or the plugin that logged a Receiving line. Provenance, \
         never content.",
    ),
    Crated::own(
        MSGCTXID_TAG_NAME,
        || Ok(DataType::utf8()),
        "Message Context ID",
        "The message context a bridge handled the message in, as its own \
         log names it.",
    ),
    Crated::own(
        MSGSESSIONID_TAG_NAME,
        || Ok(DataType::utf8()),
        "Message Session ID",
        "The session instance a bridge handled a line on, as its own row \
         header brackets it - never what the message states about itself.",
    ),
    Crated::own(
        MSGSESSEVENTID_TAG_NAME,
        || Ok(DataType::utf8()),
        "Message Session Event ID",
        "The session event a bridge delivered the message as: MsgType, the \
         session instance, the message context and MsgSeqNum joined by `:`, \
         where all four are stated. Derived and never content: the key two \
         observations of one delivery merge on.",
    ),
    Crated::own(
        CONVERSATIONID_TAG_NAME,
        || Ok(DataType::utf8()),
        "Conversation ID",
        "The conversation a bridge filed the message under, as stated: a \
         CONVERSATIONID field, else the {conversationId: ...} of the log line. \
         Provenance, never content.",
    ),
    Crated::own(
        FOREXCODE_TAG_NAME,
        || Ok(DataType::Forex),
        "Forex Code",
        "The currency pair the message is about, canonical CCY1/CCY2: the \
         FOREX entry of the message's security identifiers, a view of \
         get_securityids() - the pair get answered when the row was written - \
         detected off Symbol(55), from derived, where the message states no \
         other class. Read back from a row with no securityids column, a pair \
         the message's reading answers states nothing, the pair its symbol \
         names where nothing else names one is that detection, and any other \
         replaces its type's answer, the base key.",
    ),
    Crated::own(
        BLOOMBERGCODE_TAG_NAME,
        || Ok(DataType::Bbg),
        "Bloomberg Code",
        "The normalized Bloomberg identifier the message identifies.",
    ),
    Crated::own(
        FIGICODE_TAG_NAME,
        || Ok(DataType::figi()),
        "FIGI Code",
        "The normalized FIGI the message identifies.",
    ),
    Crated::own(
        SOURCEURL_TAG_NAME,
        || Ok(DataType::url()),
        "Source URL",
        "The object this message's line was read from; the capture's own \
         column, carried beside the row and never one of its own.",
    ),
];

/// Each crate tag's row of [`CRATED`], indexed by the tag's offset from
/// [`CRATE_TAG_MIN`], with [`u8::MAX`] where no definition takes the tag.
///
/// Built from [`CRATED`] at compile time, so the table states nothing
/// [`CRATED`] does not: a tag outside the crate's range, or one two
/// definitions take, fails the build rather than answering either row.
const CRATED_AT: [u8; (CRATE_TAG_MAX - CRATE_TAG_MIN) as usize] = {
    let mut at = [u8::MAX; (CRATE_TAG_MAX - CRATE_TAG_MIN) as usize];
    assert!(CRATED.len() < u8::MAX as usize, "every row fits a byte");
    let mut row = 0;
    while row < CRATED.len() {
        let tag = CRATED[row].tag_name.0;
        assert!(is_crate_tag(tag), "a crate definition takes a crate tag");
        let slot = (tag - CRATE_TAG_MIN) as usize;
        assert!(at[slot] == u8::MAX, "no two crate definitions take one tag");
        at[slot] = row as u8;
        row += 1;
    }
    at
};

/// Builds every field this crate defines, in tag order.
fn build() -> Result<Vec<Field>> {
    CRATED.iter().map(Crated::field).collect()
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
/// assert_eq!(held.len(), 50);
/// assert_eq!(held[0].name(), "curruuid");
/// assert_eq!(held[0].display(), Some("Current UUID"));
/// assert_eq!(held[6].name(), "currunix");
/// // No partition column: how a layout is cut is the target's to decide -
/// // an Iceberg table takes an `hour` transform over `currunix` - and a
/// // materialized copy of that instant was a second owner of it.
/// assert!(held.iter().all(|field| !field.is_partition()));
/// assert!(held.iter().all(|field| field.name() != "timepartition"));
/// // The six normalized instrument and market codes have their own columns;
/// // other graph facts remain answers off the FIX fields the message lifted.
/// // Above every tag FIX or a venue publishes, and its tag and name are
/// // its identity.
/// let (tag, name) = yggdryl::CURRUUID_TAG_NAME;
/// let mine = held[0].as_fix().id()?.expect("an identity");
/// assert_eq!(mine, yggdryl::FixId::of(tag, name)?);
/// assert!(yggdryl::is_crate_tag(yggdryl::CROSSCODE_TAG_NAME.0));
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
        // The set tag 35 reads by, which the dictionary owns: a field naming
        // none is registering the first code of a set of its own, and that
        // set is named after the field.
        let stated = field.as_fix().codeset().map(SmolStr::new);
        let set_name = stated.unwrap_or_else(|| super::FixRegistry::derived_codeset_name(field));
        let held = self.get_codeset(&set_name);
        let mut codes: Vec<super::FixCode> = held
            .map(|set| {
                set.codes()
                    .map(|code| code.map(super::FixCode::from))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let spelled = |text: &str| held.and_then(|set| set.code_value(text));
        // Every spelling this registration states, the value's own first.
        let named = name.unwrap_or(spelling);
        let (value, at) = match spelled(spelling) {
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
            if let Some(taken) = spelled(spelling)
                && taken != value.as_str()
            {
                return Err(crate::Error::Conflict {
                    expected: "a free message type spelling",
                    actual: "one another code answers to",
                    path: crate::text::expected_got(
                        format_args!("{spelling:?} at {:?}", value.as_str()),
                        format_args!("{taken:?}"),
                    ),
                });
            }
            if !codes[at].is_spelled(spelling) {
                codes[at].push_alias(spelling);
            }
        }
        if codes[at].description().is_none()
            && let Some(description) = description
        {
            codes[at] = codes[at].clone().with_description(description);
        }
        let mut next = self.clone();
        // The set first: a field may not name a vocabulary the dictionary
        // does not hold, so the members are stated before the field points
        // at them.
        next.set_codeset(&set_name, &codes)?;
        if field.as_fix().codeset().is_none() {
            let mut field = field.clone();
            field.as_fix_mut().set_codeset(&set_name)?;
            next.update(field)?;
        }
        if next.get_msgtype(&value).is_none() {
            let normalized = crate::normalized(codes[at].name());
            // A code carrying no name of its own is named after its wire
            // value, and a wire value folded is not a name: `B` would derive
            // `b`, which is the spelling the *other* FIX message answers to,
            // so the next registration of `b` would find that entry and add
            // nothing. A placeholder therefore takes the value-derived name
            // below, which no spelling can contend.
            let canonical = if codes[at].name() != value.as_str()
                && !normalized.is_empty()
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
            let mut message = crate::DataType::from(crate::StructType::from_fields([])?)
                .required_field(canonical);
            message.as_fix_mut().set_msgtype(&value)?;
            next.create_definition(crate::FixCategory::Components, message)?;
        }
        // Resolution also rejects a code shared by several contextual definitions.
        next.msgtype(&value)?;
        *self = next;
        self.msgtype(&value)
    }
}
