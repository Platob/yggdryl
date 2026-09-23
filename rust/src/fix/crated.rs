//! The definitions this crate invents, above every published tag.
//!
//! A message is a market event - a [`MarketEvent`](crate::graph::MarketEvent)
//! with a FIX body around it - and what this crate owns is only the facts
//! the event states that *no dictionary publishes*: its identity and the one
//! it has across its lifecycle and the code that names it there, the names
//! it goes by, its parents, the lines it was read from, the code its content
//! digests to, when it happened, was created, executed, recorded and was read
//! as a snapshot, the element it follows and its place in the chain. Beside
//! them stand the facts a capture states
//! about the line - where it was read from and how many pairs it carried -
//! and the ones a bridge's own log states about the line it wrote: the
//! session instance, the message context and the plugin that logged it,
//! and the session event the first two join to with the message's own type
//! and sequence. Each belongs in a column: a scalar registers as a field,
//! and the identifiers Map as a group of entries.
//!
//! The standard owns prices, quantities and classification. Six normalized
//! instrument identifiers have crate columns because their FIX sources depend
//! on an identifier source or exchange context. Parsing derives ISIN,
//! Bloomberg, FIGI and MIC; CUSIP and SEDOL are direct semantic-row or setter
//! facts, while FIX source `1` and `2` values stay in `SecurityID` or
//! `secaltids`. The columns read the existing market event holder; they add no
//! second value. `CFICode(461)` already names its classification, so it keeps
//! its standard tag. `MsgCat` projects the message component's symbolic
//! `FIX:msgcat` metadata through the registry code set into the generic
//! market operation's stable integer ID. State and expiry are event facts derived from the standard's
//! status and expiry fields, then carried by lifecycle; a newer explicit expiry
//! replaces the previous deadline.
//!
//! The event facts are the columns every graph event is
//! stated in, [`EventColumn`]: each crate field here takes that column's
//! datatype, display and wording, so a text line's batch, a FIX row and a
//! chained message carry one column under one name, one datatype and one
//! sentence, and join on it. Nine of the seventeen say more than the column
//! can - they name the FIX fields a value is read off, which is this
//! module's to know and no other medium's - and those nine spell their own
//! wording beside the tag.
//!
//! # Why 65000, and why each is a tag and a name
//!
//! Their tags sit above every tag FIX publishes and above the user-defined
//! ranges venues share, so they collide with nothing a dictionary declares
//! and belong to no dialect: `currunix` is one identity in every dictionary, a
//! bridge row spelling `SESSIONID` lands on the crate's own column, and a
//! name every registry carries is never an unknown key. [`is_crate_tag`] is
//! the whole test. A field is its tag and its name, so each is declared as
//! both - `MSGPLUGINID_TAG_NAME` is `(65_009, "msgpluginid")` - and a
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
//! `CFICode(461)`, the quote lanes `BidPx(132)`, `OfferPx(133)`,
//! `BidSize(134)` and `OfferSize(135)`, the instrument's identifiers
//! `SecurityID(48)` under `SecurityIDSource(22)` and the `SecurityAltID`
//! group, the market `SecurityExchange(207)`, `ExDestination(100)` and
//! `LastMkt(30)`, the unit `UnitOfMeasure(996)`, the state `OrdStatus(39)`
//! and `ExecType(150)`. The crate names what FIX does not.
//!
//! # Every registry holds them
//!
//! [`FixRegistry::new`](super::FixRegistry::new) inserts them before anything
//! else, so a dictionary loaded from a store, built from fields or left empty
//! answers `currunix` and `msgpluginid` alike. A store writes their definitions
//! so another consumer sees the whole row, but a read passes those stored
//! copies over: the crate's own definition is the one that types a row.
//! Folding another dictionary in never counts them either.
//!
//! Twenty-nine scalar fields and two Map groups, each registered by its shape,
//! and every one of them a row of [`CRATED`].

use std::sync::{Arc, LazyLock};

use smol_str::SmolStr;

use crate::graph::{EventColumn, MarketColumn};
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
pub const CURRUNIX_TAG_NAME: (i32, &str) = (65_003, "currunix");

/// The tag and name carrying the message context a bridge handled the
/// message in.
pub const MSGCTXID_TAG_NAME: (i32, &str) = (65_008, "msgctxid");

/// The tag and name carrying the plugin that logged the line, as a bridge
/// names it.
pub const MSGPLUGINID_TAG_NAME: (i32, &str) = (65_009, "msgpluginid");

/// The tag and name carrying the code the message's content digests to:
/// the XXH3-64 of what the event states and the named FIX content behind it.
pub const CURRHASHCODE_TAG_NAME: (i32, &str) = (65_017, "currhashcode");

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
pub const CREAUNIX_TAG_NAME: (i32, &str) = (65_023, "creaunix");

/// The tag and name carrying the grid instant a walk read the message as
/// the snapshot of.
pub const SNAPUNIX_TAG_NAME: (i32, &str) = (65_025, "snapunix");

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
pub const SOURCEURL_TAG_NAME: (i32, &str) = (65_026, "sourceurl");

/// The tag and name counting the residual entries of one FIX row.
///
/// The counter of the `fixentries` group, and a counter in the ordinary FIX
/// sense: `NoPartyIDs` counts `Parties`, and this counts entries not fully
/// represented by the row's projected columns. A reader prunes on it without
/// opening the list.
///
/// The group it counts is not a registry definition - a `fixentry` contains
/// `fixentries`, and a definition referencing itself would be a cycle - so
/// the counter is a crate field here while the group is built beside the
/// fixed row it closes.
pub const NOFIXENTRIES_TAG_NAME: (i32, &str) = (65_027, "nofixentries");

/// The tag and name carrying the session instance a bridge handled a line on.
///
/// Not `sessionid`: that spelling is a bridge row's own `SESSIONID` key,
/// which names the counterparty session a message states it is on. This is
/// the bridge's own connection instance, and the two are separate facts -
/// one message states its session once, while two connections to one
/// counterparty are two instances.
pub const MSGSESSIONID_TAG_NAME: (i32, &str) = (65_032, "msgsessionid");

/// The tag and name carrying the message's identity: UUIDv7 ordered by its
/// millisecond and sequence, with a content payload seeded by its cross hash.
pub const CURRUUID_TAG_NAME: (i32, &str) = (65_039, "curruuid");

/// The tag and name carrying the identity every message of one lifecycle
/// shares, where the message names one.
pub const CROSSUUID_TAG_NAME: (i32, &str) = (65_040, "crossuuid");

/// The tag and name carrying the message's place in its chain: how many
/// came before it.
pub const SEQNUM_TAG_NAME: (i32, &str) = (65_042, "seqnum");

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

/// The tag and name carrying the identities of the elements the message
/// was read from: its provenance, never its lineage.
///
/// A message parsed from a text line has that line's identity as its one
/// source, and a message parsed from raw bytes has none. Sources travel along
/// no chain - following and restating leave them as they are - and never
/// feed the code the message digests to, because where a message was read
/// from is not what it states.
pub const SRCUUIDS_TAG_NAME: (i32, &str) = (65_051, "srcuuids");

/// The state the event reached, ranked so the column sorts by lifecycle.
///
/// Read off `OrdStatus(39)`, else `ExecType(150)`, as a message is built,
/// `00UNKNOWN` where neither states one; the furthest its chain knows once
/// the lifecycle followed it, and a row stating one is the row's word. A
/// column, because a monitor asking which orders are still live reads a
/// ranked column rather than two code sets.
pub const STATE_TAG_NAME: (i32, &str) = (65_052, "state");

/// When the message stops being good, where it does.
///
/// `ExpireTime(126)`, else `ValidUntilTime(62)`, `ExpireDate(432)` or
/// `MaturityDate(541)`, the first stated, as a message is built; the
/// previous deadline when the next event states none; a newer explicit
/// deadline replaces it, including when it shortens the lifetime.
pub const EXPRTIME_TAG_NAME: (i32, &str) = (65_053, "exprtime");

/// The tag and name carrying the fixed business category of the message type.
pub const MSGCAT_TAG_NAME: (i32, &str) = (65_054, "msgcat");
/// The crate-owned vocabulary registered for the MsgCat column to read by.
/// Its numeric graph identifiers are intrinsic and immutable in a registry.
pub(super) const MSGCAT_CODESET_NAME: &str = "msgcatcodeset";

/// The canonical MsgCat document every registry shares.
static MSGCAT_CODESET: LazyLock<Option<Arc<str>>> = LazyLock::new(|| {
    let codes = super::constants::MSGCATEGORY_CODES
        .iter()
        .map(|(name, _, value)| super::FixCode::new(*name, *value))
        .collect::<Vec<_>>();
    match super::FixCodes::render(&codes) {
        Ok(document) => Some(Arc::from(document)),
        Err(error) => {
            log::warn!("building FIX MsgCat code set: {error}");
            None
        }
    }
});

pub(super) fn msgcat_codeset() -> Option<Arc<str>> {
    MSGCAT_CODESET.as_ref().map(Arc::clone)
}
/// The tag and name carrying the normalized ISIN the message identifies.
pub const ISINCODE_TAG_NAME: (i32, &str) = (65_055, "isincode");
/// The tag and name carrying the normalized CUSIP the message identifies.
pub const CUSIPCODE_TAG_NAME: (i32, &str) = (65_057, "cusipcode");
/// The tag and name carrying the normalized SEDOL the message identifies.
pub const SEDOLCODE_TAG_NAME: (i32, &str) = (65_058, "sedolcode");
/// The tag and name carrying the normalized Bloomberg identifier the message identifies.
pub const BLOOMBERGCODE_TAG_NAME: (i32, &str) = (65_059, "bloombergcode");
/// The tag and name carrying the normalized market MIC the message identifies.
pub const MICCODE_TAG_NAME: (i32, &str) = (65_060, "miccode");
/// The tag and name carrying the normalized FIGI the message identifies.
pub const FIGICODE_TAG_NAME: (i32, &str) = (65_061, "figicode");

/// The tag and name carrying when the message's execution happened, where
/// one of its FIX facts states it, else - on an execution report stating
/// none - when the message happened.
pub const EXECUNIX_TAG_NAME: (i32, &str) = (65_062, "execunix");

/// The tag and name carrying when the message was recorded by its carrier,
/// where a carrier states one.
pub const RECDUNIX_TAG_NAME: (i32, &str) = (65_063, "recdunix");

/// The tag and name carrying the session event a bridge delivered the
/// message as: its `MsgType(35)`, session instance, message context and
/// `MsgSeqNum(34)` joined by `:`, where all four are stated.
pub const MSGSESSEVENTID_TAG_NAME: (i32, &str) = (65_065, "msgsesseventid");

/// The graph event column one crate tag is, for the seventeen that are one.
///
/// The event facts a row states are read and written through the column,
/// [`EventColumn::fact`] and [`EventColumn::record`], so a FIX row and a
/// text line's batch answer one cell for one fact. [`CRATED`] is where the
/// pairing is stated, so a column and its tag are never written twice.
#[must_use]
pub fn event_column_of(tag: i32) -> Option<EventColumn> {
    CRATED.iter().find_map(|held| match held.holds {
        Holds::Event(column) if held.tag_name.0 == tag => Some(column),
        _ => None,
    })
}

/// The graph market column one crate tag is, where the FIX field and the
/// generic market schema are two views of the same fact.
#[must_use]
pub fn market_column_of(tag: i32) -> Option<MarketColumn> {
    CRATED.iter().find_map(|held| match held.holds {
        Holds::Market { column, .. } if held.tag_name.0 == tag => Some(column),
        _ => None,
    })
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
/// the place in the chain and the element it follows, because a walk states
/// them per message; the state it reached and when it expires, because a
/// walk folds them per message; the session event it was delivered as,
/// because it joins this message's own type and sequence; `nofixentries`,
/// `sourceurl` and `srcuuids`, because they are facts about the line this
/// row was read from.
///
/// Everything else the crate owns is about the session or the chain the
/// message stands in - the identifiers it resolved, the keys a bridge
/// stated, the plugin, the context and the session instance - and carries.
const SETTLED_TO_ONE_MESSAGE: [i32; 19] = [
    CURRUNIX_TAG_NAME.0,
    EXECUNIX_TAG_NAME.0,
    RECDUNIX_TAG_NAME.0,
    MSGSESSEVENTID_TAG_NAME.0,
    CREAUNIX_TAG_NAME.0,
    SNAPUNIX_TAG_NAME.0,
    PREVUNIX_TAG_NAME.0,
    EXPRTIME_TAG_NAME.0,
    STATE_TAG_NAME.0,
    PREVUUID_TAG_NAME.0,
    CURRHASHCODE_TAG_NAME.0,
    CROSSHASHCODE_TAG_NAME.0,
    CURRUUID_TAG_NAME.0,
    CROSSUUID_TAG_NAME.0,
    CROSSCODE_TAG_NAME.0,
    SEQNUM_TAG_NAME.0,
    NOFIXENTRIES_TAG_NAME.0,
    SOURCEURL_TAG_NAME.0,
    SRCUUIDS_TAG_NAME.0,
];

/// The crate's own columns every message states: the instants the identity
/// is settled against, the codes and the identity it settles to.
///
/// The state a message reached is stated on every row a message writes -
/// `00UNKNOWN` where nothing states one - but the column admits a null,
/// because a state has no neutral member for an empty cell to read as, and
/// a column no default can fill is not one a row can be required to state.
const ALWAYS_STATED: [i32; 6] = [
    CURRUNIX_TAG_NAME.0,
    CREAUNIX_TAG_NAME.0,
    CURRHASHCODE_TAG_NAME.0,
    CROSSHASHCODE_TAG_NAME.0,
    CURRUUID_TAG_NAME.0,
    CROSSUUID_TAG_NAME.0,
];

/// Where one definition's datatype, display and wording come from.
enum Holds {
    /// One of the seventeen [`EventColumn`]s, which owns all three: a text
    /// line's batch, a FIX row and a chained message then carry one column
    /// under one name, one datatype and one sentence, and join on it.
    Event(EventColumn),
    /// One of the generic market facts shared with graph operations.
    Market {
        column: MarketColumn,
        display: &'static str,
    },
    /// A fact no graph event states - what a bridge's row header said and
    /// the session event it joins to, where the line was read from, the
    /// residual counter and the six normalized identifiers - which therefore
    /// spells its own.
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
}

impl Crated {
    /// One graph event column under the crate's own tag.
    const fn event(tag_name: (i32, &'static str), column: EventColumn) -> Self {
        Self {
            tag_name,
            holds: Holds::Event(column),
            fix_wording: None,
            names: &[],
            codeset: None,
        }
    }

    /// One graph market column under the crate's own tag.
    const fn market(
        tag_name: (i32, &'static str),
        column: MarketColumn,
        display: &'static str,
    ) -> Self {
        Self {
            tag_name,
            holds: Holds::Market { column, display },
            fix_wording: None,
            names: &[],
            codeset: None,
        }
    }

    /// One fact no graph event states, spelling its own datatype, display
    /// and wording.
    const fn own(
        tag_name: (i32, &'static str),
        datatype: fn() -> Result<DataType>,
        display: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            tag_name,
            holds: Holds::Own {
                datatype,
                display,
                description,
            },
            fix_wording: None,
            names: &[],
            codeset: None,
        }
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

    /// This definition as the field every registry holds.
    ///
    /// Display and description use generic field metadata rather than the
    /// `FIX:` scheme because every catalog the crate writes to understands
    /// them.
    fn field(&self) -> Result<Field> {
        let (tag, name) = self.tag_name;
        let (dtype, display, description) = match self.holds {
            Holds::Event(column) => (column.datatype()?, column.display(), column.description()),
            Holds::Market { column, display } => (
                column.datatype(),
                display,
                "The stable integer category of the market operation.",
            ),
            Holds::Own {
                datatype,
                display,
                description,
            } => (datatype()?, display, description),
        };
        // A crate Map is a group whose occurrence is its own entries Struct,
        // so the tag counting it is the tag it is: there is no second tag to
        // state, and no row that could state a different one.
        let counts_itself = matches!(dtype, DataType::Map(_) | DataType::SortedMap(_));
        let mut field = Field::new(name, dtype, !ALWAYS_STATED.contains(&tag));
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
/// and [`fix_crate_fields`] all walk them in. A row that only names a tag
/// and a column is a column this crate adds nothing to but the tag.
const CRATED: [Crated; 31] = [
    Crated::event(CURRUNIX_TAG_NAME, EventColumn::CurrUnix),
    Crated::own(
        MSGCTXID_TAG_NAME,
        || Ok(DataType::utf8()),
        "MsgCtxId",
        "The message context a bridge handled the message in, as its own \
         log names it.",
    ),
    Crated::own(
        MSGPLUGINID_TAG_NAME,
        || Ok(DataType::utf8()),
        "MsgPluginId",
        "The plugin that logged the line inside a bridge, as the bridge \
         names it: the row's own msgpluginid column, never derived.",
    ),
    Crated::event(CURRHASHCODE_TAG_NAME, EventColumn::CurrHashCode).saying(
        "The XXH3-64 of what the event states and the named FIX content \
         behind it.",
    ),
    Crated::event(CROSSHASHCODE_TAG_NAME, EventColumn::CrossHashCode),
    Crated::event(IDENTIFIERS_TAG_NAME, EventColumn::Identifiers).saying(
        "The names this message goes by, each under the canonical name of the \
         field that stated it, in sorted order; repeating-group members are \
         not flattened.",
    ),
    Crated::event(PREVUNIX_TAG_NAME, EventColumn::PrevUnix),
    Crated::event(PREVUUID_TAG_NAME, EventColumn::PrevUuid),
    Crated::event(CREAUNIX_TAG_NAME, EventColumn::CreaUnix)
        .saying(
            "When the message was created: what it states, else when the \
             original was sent, else when it happened; the earliest its \
             chain knows once followed.",
        )
        .also_called(&["CreationTime"]),
    Crated::event(SNAPUNIX_TAG_NAME, EventColumn::SnapUnix),
    Crated::own(
        SOURCEURL_TAG_NAME,
        || Ok(DataType::url()),
        "SourceUrl",
        "The object this message's line was read from; the capture's own \
         column, carried beside the row and never one of its own.",
    ),
    Crated::own(
        NOFIXENTRIES_TAG_NAME,
        || Ok(DataType::Int32),
        "NoFixEntries",
        "How many residual entries remain outside the row's projected columns.",
    ),
    Crated::own(
        MSGSESSIONID_TAG_NAME,
        || Ok(DataType::utf8()),
        "MsgSessionId",
        "The session instance a bridge handled a line on, as its own row \
         header brackets it - never what the message states about itself.",
    ),
    Crated::event(CURRUUID_TAG_NAME, EventColumn::CurrUuid),
    Crated::event(CROSSUUID_TAG_NAME, EventColumn::CrossUuid),
    Crated::event(SEQNUM_TAG_NAME, EventColumn::SeqNum),
    Crated::event(CROSSCODE_TAG_NAME, EventColumn::CrossCode).saying(
        "The identifier every message of one lifecycle shares: OrderID, \
         else ClOrdID, OrigClOrdID, QuoteID, QuoteReqID or MDReqID, the \
         first stated.",
    ),
    Crated::own(
        METADATA_TAG_NAME,
        || DataType::map_of(DataType::utf8(), DataType::utf8(), true),
        "Metadata",
        "What a bridge stated under its own namespaces - a `TECH.` or an \
         `AMON.` key - each under the key as the bridge spelled it, folded, \
         in sorted order.",
    ),
    Crated::event(SRCUUIDS_TAG_NAME, EventColumn::SrcUuids).saying(
        "The identities of the elements this message was read from: the \
         text line it was parsed out of, and none for one parsed from \
         raw bytes. Provenance, never lineage: no walk moves it.",
    ),
    Crated::event(STATE_TAG_NAME, EventColumn::State).saying(
        "The state the message reached, ranked so the column sorts by \
         lifecycle: OrdStatus, else ExecType, 00UNKNOWN where neither \
         states one; the furthest its chain knows once followed.",
    ),
    Crated::event(EXPRTIME_TAG_NAME, EventColumn::ExprTime).saying(
        "When the message stops being good: ExpireTime, else \
         ValidUntilTime, ExpireDate or MaturityDate; a newer explicit \
         deadline replaces the one its chain carried.",
    ),
    Crated::market(MSGCAT_TAG_NAME, MarketColumn::MarketOperationId, "MsgCat")
        .saying("The stable integer code for the message type's business category.")
        .reading(MSGCAT_CODESET_NAME),
    Crated::own(
        ISINCODE_TAG_NAME,
        || Ok(DataType::isin()),
        "IsinCode",
        "The normalized ISIN the message identifies.",
    ),
    Crated::own(
        CUSIPCODE_TAG_NAME,
        || Ok(DataType::cusip()),
        "CusipCode",
        "The normalized CUSIP the message identifies.",
    ),
    Crated::own(
        SEDOLCODE_TAG_NAME,
        || Ok(DataType::sedol()),
        "SedolCode",
        "The normalized SEDOL the message identifies.",
    ),
    Crated::own(
        BLOOMBERGCODE_TAG_NAME,
        || Ok(DataType::BloombergCode),
        "BloombergCode",
        "The normalized Bloomberg identifier the message identifies.",
    ),
    Crated::own(
        MICCODE_TAG_NAME,
        || Ok(DataType::mic()),
        "MicCode",
        "The normalized market MIC the message identifies.",
    ),
    Crated::own(
        FIGICODE_TAG_NAME,
        || Ok(DataType::figi()),
        "FIGICode",
        "The normalized FIGI the message identifies.",
    ),
    Crated::event(EXECUNIX_TAG_NAME, EventColumn::ExecUnix).saying(
        "When the message's execution happened: ExecutionTimestamp, an \
         execution TrdRegTimestamp, a proprietary EventTimestamp, or a \
         trade's TransactTime, the first one stated; an execution report \
         stating none executed at its currunix; the latest its chain \
         reached once followed.",
    ),
    Crated::event(RECDUNIX_TAG_NAME, EventColumn::RecdUnix).saying(
        "When the message was recorded by its carrier, where the carrier \
         states one; the earliest its statements know.",
    ),
    Crated::own(
        MSGSESSEVENTID_TAG_NAME,
        || Ok(DataType::utf8()),
        "MsgSessEventId",
        "The session event a bridge delivered the message as: MsgType, the \
         session instance, the message context and MsgSeqNum joined by `:`, \
         where all four are stated. Derived and never content: the key two \
         observations of one delivery merge on.",
    ),
];

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
/// assert_eq!(held.len(), 31);
/// assert_eq!(held[0].name(), "currunix");
/// assert_eq!(held[0].display(), Some("CurrUnix"));
/// // No partition column: how a layout is cut is the target's to decide -
/// // an Iceberg table takes an `hour` transform over `currunix` - and a
/// // materialized copy of that instant was a second owner of it.
/// assert!(held.iter().all(|field| !field.is_partition()));
/// assert!(held.iter().all(|field| field.name() != "timepartition"));
/// // The six normalized instrument and market codes have their own columns;
/// // other graph facts remain answers off the FIX fields the message lifted,
/// // so nothing here declares a `FIX:derivation`.
/// assert!(held.iter().all(|field| !field.has_metadata("FIX:derivation")));
/// // Above every tag FIX or a venue publishes, and its tag and name are
/// // its identity.
/// let (tag, name) = yggdryl::CURRUNIX_TAG_NAME;
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
            if let Some(taken) = spelled(spelling) {
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
