//! A FIX message: its typed facts, its row, and the registry that types it.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use super::build::stated;
use super::entry::{FixEntry, emit_bytes, emit_text, wire_text, wire_text_under};
use super::identity::{self, FixCapture, FixHeader, FixLifted, Typed};
use super::registry::FixMap;
use super::{FixId, FixIdMapKind, FixKey, FixRegistry};
use crate::graph::facts::OperationEventFacts;
use crate::graph::{Element, Event, FxRates, Market, Metadata, Operation};
use crate::xxhash;
use crate::{
    Ccy, Cfi, Decimal, Forex, IdSource, IdType, Identifier, Identifiers, MarketDataKind, Mic, Side,
    State, StructType, TimeInForce, Unit, Uuid,
};
use crate::{DataType, Error, Field, FieldPath, FieldSegment, Result, Scalar, Serie};

/// The nanoseconds in one day: what a transaction time at midnight to the
/// nanosecond is a multiple of, and what a day-only `TransactTime(60)` is
/// restated as.
const NANOS_PER_DAY: i64 = 86_400 * 1_000_000_000;

/// The row stated this normalized market code, rather than a raw FIX pair
/// from which the stated identifiers could replace it.
const ROW_STATED_ISIN: u64 = 1 << 2;
const ROW_STATED_BLOOMBERG: u64 = 1 << 5;
const ROW_STATED_FIGI: u64 = 1 << 7;
/// The row or a caller stated when the message executed, so a settle of the
/// clock alone leaves it.
const ROW_STATED_EXECUTION: u64 = 1 << 8;
const ROW_STATED_RECORDING: u64 = 1 << 9;
/// The row stated the currency pair, so detection off `Symbol(55)` leaves
/// it: set by a row's `forexcode` cell, a `record` of it and a stated
/// `FOREX` identifier, never by detection.
const ROW_STATED_FOREX: u64 = 1 << 11;

/// The market facts a message states off its FIX fields, one bit each: what
/// a message fills as it is built, what a write reaches ([`facts_of_tag`])
/// and a settle states again ([`FixMsg::state_market`]).
mod fact {
    pub(super) const SECURITYIDS: u32 = 1;
    pub(super) const STATE: u32 = 1 << 1;
    pub(super) const KIND: u32 = 1 << 2;
    pub(super) const MDTYPE: u32 = 1 << 3;
    pub(super) const EXPIRY: u32 = 1 << 4;
    pub(super) const EXECUTION: u32 = 1 << 5;
    pub(super) const TIF: u32 = 1 << 6;
    pub(super) const TICKER: u32 = 1 << 7;
    pub(super) const CFI: u32 = 1 << 8;
    pub(super) const MIC: u32 = 1 << 9;
    pub(super) const TRADABLE: u32 = 1 << 10;
    pub(super) const UNIT: u32 = 1 << 11;
    pub(super) const BIDASK: u32 = 1 << 12;
    pub(super) const PRICE: u32 = 1 << 13;
    pub(super) const STOPPX: u32 = 1 << 14;
    pub(super) const QUANTITY: u32 = 1 << 15;
    pub(super) const DISPLAYQTY: u32 = 1 << 16;
    pub(super) const HIDDENQTY: u32 = 1 << 17;
    pub(super) const CXLQTY: u32 = 1 << 18;
    pub(super) const PREVPX: u32 = 1 << 19;
    pub(super) const FILLS: u32 = 1 << 20;
    pub(super) const LASTPX: u32 = 1 << 21;
    pub(super) const CURRENCY: u32 = 1 << 22;
    pub(super) const SIDE: u32 = 1 << 23;
    pub(super) const ORDQTY: u32 = 1 << 24;
    pub(super) const IDENTIFIERS: u32 = 1 << 25;
    pub(super) const PARTYIDS: u32 = 1 << 26;
    /// Every fact.
    pub(super) const ALL: u32 = (1 << 27) - 1;
    /// An order's quantities - what it ordered, what traded, what is left,
    /// what was canceled - which fill one another against its state and are
    /// stated again together.
    pub(super) const ORDERED: u32 = ORDQTY | FILLS | CXLQTY;
    /// The facts that fill one another through their setters - a side's
    /// bid or ask off the price and quantity and back, their currency, the
    /// hidden part of an iceberg - which are stated again together.
    pub(super) const QUOTED: u32 =
        BIDASK | PRICE | QUANTITY | SIDE | CURRENCY | DISPLAYQTY | HIDDENQTY;
    /// The facts stated again from scratch: every one but those read with a
    /// rule of their own - the identifiers, the state, the category, the
    /// expiry and the execution clock.
    pub(super) const CLEARED: u32 =
        ALL & !(SECURITYIDS | STATE | KIND | EXPIRY | EXECUTION | IDENTIFIERS | PARTYIDS);
    /// What the message's status reaches: the state, the category read
    /// against it, the execution it dates, and what the category decides -
    /// the type and a sided quote's FX parts.
    pub(super) const STATUS: u32 = STATE | KIND | EXECUTION | MDTYPE | LASTPX | ORDERED;
    /// What a field no tag maps reaches: the facts a bridge's own spelling
    /// can state - an identifier source, a detailed classification, a quote
    /// side's currency, an event timestamp - and what the instrument key an
    /// identifier names fills.
    pub(super) const NAMED: u32 = SECURITYIDS | CFI | MIC | CURRENCY | BIDASK | EXECUTION;
}

/// The facts a write of `tag` reaches: the ones [`FixMsg::state_market`]
/// reads the tag for, and the ones those fill in turn. A tag no fact reads -
/// an order identifier, a free text - reaches none, so what a lifecycle
/// walk set on the message stands through it.
fn facts_of_tag(registry: &FixRegistry, tag: i32) -> u32 {
    match tag {
        44 => fact::PRICE | fact::BIDASK,
        99 => fact::STOPPX,
        38 => fact::ORDERED | fact::QUANTITY,
        53 => fact::QUANTITY | fact::HIDDENQTY | fact::BIDASK,
        1138 | 111 => fact::DISPLAYQTY | fact::HIDDENQTY,
        84 => fact::ORDERED,
        996 => fact::UNIT,
        54 => fact::SIDE | fact::BIDASK | fact::PRICE | fact::QUANTITY | fact::LASTPX,
        59 => fact::TIF,
        55 => fact::TICKER,
        326 | 340 | 965 => fact::TRADABLE,
        126 | 62 | 432 | 541 => fact::EXPIRY,
        140 => fact::PREVPX,
        BIDPX_TAG | OFFERPX_TAG | BIDSIZE_TAG | OFFERSIZE_TAG => {
            fact::BIDASK | fact::PRICE | fact::QUANTITY
        }
        31 | 194 | 195 | 188 | 189 | 190 | 191 => fact::LASTPX | fact::FILLS,
        32 | 6 | 14 | 151 => fact::ORDERED,
        15 | 120 => fact::CURRENCY | fact::BIDASK,
        30 | 100 | 207 => fact::MIC,
        461 | 201 => fact::CFI,
        22 | 48 | 454 | 455 | 456 => fact::SECURITYIDS | fact::MIC | fact::CURRENCY,
        2749 | 60 | 768 | 769 | 770 | 1750..=1770 => fact::EXECUTION,
        35 | 117 => fact::STATUS,
        tag if State::FIX_STATUS_TAGS.contains(&tag) || tag == 150 => fact::STATUS,
        tag if crate::MARKETDATATYPE_FIX_TAGS.contains(&tag)
            || registry
                .marketdatatype_sources()
                .iter()
                .any(|(held, _, _)| *held == tag) =>
        {
            fact::MDTYPE
        }
        tag if registry
            .timeinforce_sources()
            .iter()
            .any(|(held, _, _)| *held == tag) =>
        {
            fact::TIF
        }
        _ => 0,
    }
}

/// The fact a crate column states outright, which a row or a caller
/// recording it states as its own word: a pending restatement of it is
/// dropped, and a null hands it back to the fields.
fn fact_of_crate_tag(tag: i32) -> u32 {
    use super::crated as c;
    match tag {
        t if t == c::STATE_TAG_NAME.0 => fact::STATE,
        t if t == c::MARKETDATAKIND_TAG_NAME.0 => fact::KIND,
        t if t == c::MARKETDATATYPE_TAG_NAME.0 => fact::MDTYPE,
        t if t == c::EXPRUNIX_TAG_NAME.0 => fact::EXPIRY,
        t if t == c::EXECUNIX_TAG_NAME.0 => fact::EXECUTION,
        t if t == c::MICCODE_TAG_NAME.0 => fact::MIC,
        t if t == c::HIDDENQTY_TAG_NAME.0 => fact::HIDDENQTY,
        t if t == c::UNIT_TAG_NAME.0 => fact::UNIT,
        t if t == c::SECURITYIDS_TAG_NAME.0 => fact::SECURITYIDS,
        t if t == c::IDENTIFIERS_TAG_NAME.0 => fact::IDENTIFIERS,
        t if t == c::PARTYIDS_TAG_NAME.0 => fact::PARTYIDS,
        t if t == c::PREVPX_TAG_NAME.0 => fact::PREVPX,
        t if t == c::TICKER_TAG_NAME.0 => fact::TICKER,
        t if t == c::ORDQTY_TAG_NAME.0 => fact::ORDQTY,
        t if t == c::TRADABLE_TAG_NAME.0 => fact::TRADABLE,
        t if t == c::SPOTRATE_TAG_NAME.0 || t == c::FORWARDPOINTS_TAG_NAME.0 => fact::LASTPX,
        t if [
            c::BIDQTY_TAG_NAME.0,
            c::BIDCCY_TAG_NAME.0,
            c::ASKPX_TAG_NAME.0,
            c::ASKQTY_TAG_NAME.0,
            c::ASKCCY_TAG_NAME.0,
        ]
        .contains(&t) =>
        {
            fact::BIDASK
        }
        _ => 0,
    }
}

/// One field's text, trimmed; `None` where it is null or empty.
fn stated_text(value: Option<Scalar>) -> Option<SmolStr> {
    value
        .and_then(|held| held.as_str().map(str::trim).map(SmolStr::new))
        .filter(|held| !held.is_empty())
}

/// FIX's own `StrikePrice(202)`: what [`FixMsg::strikeprice`] reads.
const STRIKEPRICE_TAG: i32 = 202;

/// The group an identifier map reads its role sources out of, and the two
/// members of each occurrence it reads: the `PartyID(448)` stated under the
/// occurrence's `PartyRole(452)`.
const PARTIES: &str = "parties";
const PARTYROLE: &str = "partyrole";
const PARTYID: &str = "partyid";

/// `NoSides(552)`: a trade's or a cross's sides, each stating its own
/// parties and regulatory trade identifiers - an execution a trade's parse
/// split off holds its one side here.
const SIDES: i32 = 552;
/// `PartyRole(452)`, whose code set names every role a party is typed by.
const PARTY_ROLE: i32 = 452;
/// `PartyIDSource(447)`, whose code set names the source of a party.
const PARTY_ID_SOURCE: i32 = 447;
/// The groups naming a message's parties - `NoPartyIDs(453)` and
/// `NoRootPartyIDs(1116)` - each with the tags of its occurrence's
/// identifier, role and source.
const PARTY_GROUPS: [(i32, i32, i32, i32); 2] = [
    (453, 448, PARTY_ROLE, PARTY_ID_SOURCE),
    (1116, 1117, 1119, 1118),
];
/// `Account(1)`: the account an order is booked to, one of a message's
/// parties typed [`IdType::Account`].
const ACCOUNT: i32 = 1;
/// `AcctIDSource(660)`: the source of `Account(1)`.
const ACCOUNT_SOURCE: i32 = 660;
/// The groups naming a message's regulatory trade identifiers -
/// `NoRegulatoryTradeIDs(1907)` and a side's `NoSideRegulatoryTradeIDs(1971)`
/// - each with the tags of its occurrence's identifier and type.
const REGULATORY_GROUPS: [(i32, i32, i32); 2] = [(1907, 1903, 1906), (1971, 1972, 1975)];

/// The members of one occurrence a rebuild reads, by position.
type Occurrence<const N: usize> = [Option<SmolStr>; N];

/// One party group of a level: its value, beside where its occurrence
/// states the identifier, the role and the source.
type PartyGroup<'level> = Option<(&'level Scalar, [Option<usize>; 3])>;

/// Every occurrence of the typed group `value`, each read at the member
/// `positions` of its item as the text it spells: a member the item does
/// not declare, or an occurrence leaves null or empty, is `None`.
fn read_occurrences<const N: usize>(
    value: &Scalar,
    positions: [Option<usize>; N],
    into: &mut SmallVec<[Occurrence<N>; 8]>,
) {
    let Some(rows) = value.as_serie() else {
        return;
    };
    for occurrence in rows.iter() {
        if let Some(held) = occurrence.as_sequence() {
            into.push(positions.map(|at| at.and_then(|at| held.get(at)).and_then(scalar_text)));
        }
    }
}

/// Where the fields of one level - a root, a component, an occurrence -
/// hold the child stating `wanted`: as the group it counts when `counts`,
/// else as its own tag.
fn child_by_tag(
    registry: &FixRegistry,
    fields: &[Field],
    counts: bool,
    wanted: i32,
) -> Option<usize> {
    fields.iter().position(|child| {
        let (tag, counter) = super::schema::tag_and_counter(registry, child);
        (if counts { counter } else { tag }) == Some(wanted)
    })
}

/// The code sets a party is typed and sourced by: `PartyRole(452)`'s,
/// `PartyIDSource(447)`'s and `AcctIDSource(660)`'s, each where the
/// dictionary states one.
#[derive(Clone, Copy)]
pub(super) struct PartyCodes<'registry> {
    roles: Option<super::FixCodeSet<'registry>>,
    sources: Option<super::FixCodeSet<'registry>>,
    accounts: Option<super::FixCodeSet<'registry>>,
}

impl<'registry> PartyCodes<'registry> {
    /// The party code sets `registry` states.
    pub(super) fn new(registry: &'registry FixRegistry) -> Self {
        let of = |tag: i32| {
            registry
                .get_field_by_tag(tag)
                .and_then(|field| registry.codeset_of(field))
        };
        Self {
            roles: of(PARTY_ROLE),
            sources: of(PARTY_ID_SOURCE),
            accounts: of(ACCOUNT_SOURCE),
        }
    }

    /// The party one occurrence states: its `PartyID` typed by its role's
    /// name - `ExecutingTrader` is `executingtrader` - and sourced by its
    /// source's, each read through [`code_word`]; [`IdType::Party`] for no
    /// role and [`IdSource::Base`] for no source.
    pub(super) fn party(
        &self,
        value: &str,
        role: Option<&str>,
        source: Option<&str>,
    ) -> Option<Identifier> {
        let kind = match code_word(self.roles, role, "partyrole") {
            Some(word) => word.parse().ok()?,
            None => IdType::Party,
        };
        let src = match code_word(self.sources, source, "partyidsource") {
            Some(word) => word.parse().ok()?,
            None => IdSource::Base,
        };
        Identifier::new(src, kind, value).ok()
    }

    /// The party `Account(1)` states: an [`IdType::Account`] sourced by its
    /// `AcctIDSource(660)`'s name, read through [`code_word`], where stated.
    fn account(&self, value: &str, source: Option<&str>) -> Option<Identifier> {
        let src = match code_word(self.accounts, source, "acctidsource") {
            Some(word) => word.parse().ok()?,
            None => IdSource::Base,
        };
        Identifier::new(src, IdType::Account, value).ok()
    }
}

/// The word a party's role or source `text` is in its code set `codes`: the
/// name of the code it is - stated by its wire value, the hot path, or by
/// any spelling the set resolves - else its own spelling where the set
/// resolves none, `generallyacceptedmarketparticipantidentifier` as
/// itself; a bare wire code the set names nothing for - digits, or one
/// character - is `{prefix}{code}`, `partyrole99`, so it never reads as a
/// word of its own. None where the answer is no identifier word.
fn code_word<'registry, 'text>(
    codes: Option<super::FixCodeSet<'registry>>,
    text: Option<&'text str>,
    prefix: &str,
) -> Option<Cow<'text, str>>
where
    'registry: 'text,
{
    let text = text?.trim();
    let (code, name) = match codes {
        Some(codes) => match codes.code_name(text) {
            Some(name) => (text, Some(name)),
            None => codes
                .code_value(text)
                .map_or((text, None), |wire| (wire, codes.code_name(wire))),
        },
        None => (text, None),
    };
    if let Some(name) = name.filter(|name| crate::identifier::is_word(name)) {
        return Some(Cow::Borrowed(name));
    }
    let bare = code.len() == 1 || code.bytes().all(|byte| byte.is_ascii_digit());
    if !bare {
        return crate::identifier::is_word(code).then_some(Cow::Borrowed(code));
    }
    // Measured before it is spelled: a spelling no word is costs nothing.
    crate::identifier::folded_len(code)
        .is_some_and(|len| len > 0 && prefix.len() + len <= crate::identifier::IDENTIFIER_KEY_WIDTH)
        .then(|| Cow::Owned(format!("{prefix}{code}")))
}

/// Reads the parties one level of a message states into `into`: each
/// party of its `Parties(453)` and `RootParties(1116)` groups, then its
/// `Account(1)`. The first value of a role and source stands; a second
/// party of one is ordinary - two contra firms - and stays where the wire
/// states it, an anomaly of nothing.
fn read_parties(
    codes: PartyCodes<'_>,
    groups: [PartyGroup<'_>; 2],
    account: [Option<&Scalar>; 2],
    into: &mut Identifiers,
) {
    for (value, positions) in groups.into_iter().flatten() {
        let mut occurrences = SmallVec::<[Occurrence<3>; 8]>::new();
        read_occurrences(value, positions, &mut occurrences);
        for [value, role, source] in occurrences {
            if let Some(party) =
                value.and_then(|value| codes.party(&value, role.as_deref(), source.as_deref()))
            {
                into.insert(party);
            }
        }
    }
    if let Some(party) = account[0]
        .and_then(scalar_text)
        .and_then(|value| codes.account(&value, account[1].and_then(scalar_text).as_deref()))
    {
        into.insert(party);
    }
}

/// Where the occurrence of a party group states its identifier, its role
/// and its source, found by tag in the group's `field`.
fn party_positions(
    registry: &FixRegistry,
    field: &Field,
    counter: i32,
) -> Option<[Option<usize>; 3]> {
    let (_, id, role, source) = PARTY_GROUPS.iter().find(|(held, ..)| *held == counter)?;
    let serie = field.dtype().as_serie_type()?;
    let fields = serie.item().fields();
    Some([*id, *role, *source].map(|tag| child_by_tag(registry, fields, false, tag)))
}

/// Where one level of a message - a trade side, a book entry - states its
/// parties, planned once from its fields and read off each occurrence's
/// cells: each party group and, inside it, the identifier, the role and the
/// source, and the `Account(1)` with its source.
pub(super) struct AccountsAt {
    parties: [Option<(usize, [Option<usize>; 3])>; 2],
    account: [Option<usize>; 2],
}

impl AccountsAt {
    /// Where `fields` state their parties.
    pub(super) fn new(registry: &FixRegistry, fields: &[Field]) -> Self {
        Self {
            parties: PARTY_GROUPS.map(|(counter, ..)| {
                let at = child_by_tag(registry, fields, true, counter)?;
                Some((at, party_positions(registry, fields.get(at)?, counter)?))
            }),
            account: [ACCOUNT, ACCOUNT_SOURCE]
                .map(|tag| child_by_tag(registry, fields, false, tag)),
        }
    }

    /// Whether the level states no party at all.
    fn is_empty(&self) -> bool {
        self.parties.iter().all(Option::is_none) && self.account[0].is_none()
    }

    /// Reads the parties one occurrence's `cells` state into `into`, as
    /// [`read_parties`] does.
    pub(super) fn read(&self, codes: PartyCodes<'_>, cells: &[Scalar], into: &mut Identifiers) {
        if self.is_empty() {
            return;
        }
        let groups = self
            .parties
            .map(|group| group.and_then(|(at, positions)| Some((cells.get(at)?, positions))));
        let account = self.account.map(|at| at.and_then(|at| cells.get(at)));
        read_parties(codes, groups, account, into);
    }
}

/// The type a regulatory trade identifier of one
/// `RegulatoryTradeIDType(1906)` goes under in `identifiers`: `regtradeid`
/// for the current one or where no type is stated, the previous, block,
/// related and cleared-block ones prefixed, a trading venue's transaction
/// identifier `tvtic` and a report's tracking number
/// `reporttrackingnumber`; any other type `regtradeid{type}`, where that is
/// a word.
fn regulatory_kind(kind: Option<&str>) -> crate::Result<IdType> {
    Ok(match kind.unwrap_or("0") {
        "0" => IdType::RegTradeId,
        "1" => IdType::PrevRegTradeId,
        "2" => IdType::BlockRegTradeId,
        "3" => IdType::RelatedRegTradeId,
        "4" => IdType::ClearedRegTradeId,
        "5" => IdType::Tvtic,
        "6" => IdType::ReportTrackingNumber,
        other => return format!("regtradeid{other}").parse(),
    })
}

/// States `value` as an identifier of `kind` from `src` in `ids`: the first
/// value one is stated with fills it, and a later different one - or one no
/// identifier holds - states nothing and is kept as an anomaly of `field`.
fn admit_identifier(
    ids: &mut Identifiers,
    src: IdSource,
    kind: crate::Result<IdType>,
    value: &str,
    field: &str,
    dropped: &mut Vec<super::FixAnomaly>,
) {
    if crate::code::is_null_like(value) {
        return;
    }
    let id = match kind.and_then(|kind| Identifier::new(src, kind, value)) {
        Ok(id) => id,
        Err(error) => {
            dropped.push(super::FixAnomaly::new(
                field,
                format!("states {value:?}, which no identifier holds: {error}"),
            ));
            return;
        }
    };
    match ids.get_from(id.src(), id.kind()) {
        Some(held) if held != id.value() => dropped.push(super::FixAnomaly::new(
            field,
            format!(
                "states {id} where {}:{}={held} is already stated",
                id.src(),
                id.kind()
            ),
        )),
        Some(_) => {}
        None => {
            ids.insert(id);
        }
    }
}

/// The bridge key an execution clock is read from where no FIX field
/// states one.
const EVENT_TIMESTAMP: &str = "eventtimestamp";

/// The names a bid's currency is read by, ahead of the message's own: a
/// bridge or extension field, since FIX names none.
const BID_CURRENCY: [&str; 1] = ["bidcurrency"];

/// The names an ask's currency is read by, ahead of the message's own: an
/// ask is FIX's offer, so either spelling states it.
const ASK_CURRENCY: [&str; 2] = ["askcurrency", "offercurrency"];

/// FIX's `BidPx(132)`, `OfferPx(133)`, `BidSize(134)` and `OfferSize(135)`:
/// what a quote states for each side, and what `bidpx`, `askpx`, `bidqty`
/// and `askqty` read.
const BIDPX_TAG: i32 = 132;
const OFFERPX_TAG: i32 = 133;
const BIDSIZE_TAG: i32 = 134;
const OFFERSIZE_TAG: i32 = 135;

/// The row-owned facts whose non-null value must survive the fields.
fn row_stated_bit(tag: i32) -> Option<u64> {
    if tag == super::ISINCODE_TAG_NAME.0 {
        Some(ROW_STATED_ISIN)
    } else if tag == super::BLOOMBERGCODE_TAG_NAME.0 {
        Some(ROW_STATED_BLOOMBERG)
    } else if tag == super::FIGICODE_TAG_NAME.0 {
        Some(ROW_STATED_FIGI)
    } else if tag == super::EXECUNIX_TAG_NAME.0 {
        Some(ROW_STATED_EXECUTION)
    } else if tag == super::RECDUNIX_TAG_NAME.0 {
        Some(ROW_STATED_RECORDING)
    } else if tag == super::FOREXCODE_TAG_NAME.0 {
        Some(ROW_STATED_FOREX)
    } else {
        None
    }
}

/// The category the registry files `msgtype` under, as it files it.
///
/// A custom registry may file its own message type under any category the
/// crate's [`MarketDataKind`] names; a type the registry does not file falls
/// to the upstream type-to-category table, and a type neither files is
/// [`MarketDataKind::Unknown`].
fn filed_msgcat(registry: &FixRegistry, msgtype: &str) -> MarketDataKind {
    registry
        .get_msgtype(msgtype)
        .and_then(super::MsgType::msgcat)
        .or_else(|| super::constants::msgcat_of(msgtype).and_then(MarketDataKind::from_name))
        .unwrap_or(MarketDataKind::Unknown)
}

/// The business category one FIX message type files a message under: a
/// type filed under `EXEC` - an execution report - that `reports` no
/// execution filed as its order's report, [`MarketDataKind::Order`], or its
/// quote's, [`MarketDataKind::Quotation`], where it names a `QuoteID(117)`
/// (`quoted`): a lifecycle chains within one category, so an
/// acknowledgement and a cancel follow their order, and a report of a fill
/// stays the execution it states until a stream door splits it into its
/// order's report and that execution. Every other type is as filed
/// ([`filed_msgcat`]).
fn derived_msgcat(
    registry: &FixRegistry,
    msgtype: &str,
    quoted: bool,
    reports: bool,
) -> MarketDataKind {
    match filed_msgcat(registry, msgtype) {
        MarketDataKind::Execution if reports => MarketDataKind::Execution,
        MarketDataKind::Execution if quoted => MarketDataKind::Quotation,
        MarketDataKind::Execution => MarketDataKind::Order,
        other => other,
    }
}

/// The identity tags a row's reading records last, in this order: the
/// cross code once the category it is stored under stands, then the hash
/// and the elements it derives, which a row stating them states over the
/// derived ones.
const SETTLED_LAST: [i32; 4] = [
    super::CROSSCODE_TAG_NAME.0,
    super::CROSSHASHCODE_TAG_NAME.0,
    super::CROSSUUID_TAG_NAME.0,
    super::CURRUUID_TAG_NAME.0,
];

/// The category one `msgcat` cell states: the member, its code or any
/// spelling of it; `None` for a null or a value naming none.
fn msgcat_of(value: &Scalar) -> Option<MarketDataKind> {
    <MarketDataKind as crate::EnumValue>::from_scalar_value(value)
}

/// A FIX message: a market event with a FIX body around it.
///
/// Three typed holders and one row. The event holder is the event
/// the message is - its identity, when it happened, where it stands and the
/// market's facts - and the message answers [`Element`], [`Event`] and
/// [`Market`] through it, so a walk over messages reads them as it
/// reads any event. The [`FixHeader`] is the standard header, typed: the
/// version, the type, who sent it to whom, the sequence number and the
/// sending time. The [`FixCapture`] is what the capture said about the
/// line. The row is everything else the message states - the body's
/// fields, its repeating groups, the keys no dictionary explains - as a
/// core Struct [`Field`] and its value, each child typed by the registry's
/// field for it, and none of the typed facts is in it: every fact is held
/// once. The registry link is an [`Arc`], cloned from
/// [`FixRegistry::from_env`] when the caller names none, so a message carries
/// the dictionary it was resolved against.
///
/// A value is reached by tag, by identifier, by name, or by path, each
/// answering the value it finds - a typed fact for a tag the holders own,
/// a row child otherwise - with a failing twin; resolution goes through the
/// linked registry, never through a private copy of its rules. An unknown
/// tag is retained rather than dropped: it is looked for under its rendered
/// decimal name, which is where a transcriber keeps a tag no dictionary
/// explains. A repeating-group counter remains an int32 value reached by its
/// tag; the separate collection is reached by name, such as `Parties`.
///
/// The message's identity is settled from what it states: the code is the
/// XXH3-64 of the event's facts, the text, the metadata, the FIX fields it
/// lifted and the named content of the row - everything but the standard
/// header and trailer, less `MsgType`, and never the chain it is in - the
/// UUIDv7 identity ordered by millisecond and sequence with a content payload
/// seeded by the cross hash, and the cross identity the UUIDv8 the cross code's
/// digest derives - the first chain
/// identifier the message spells, `OrderID` before `ClOrdID`. A bridge's
/// bracketed session and context, joined with the message's type and
/// sequence, are the session event it was delivered as,
/// [`FixCapture::msgsesseventid`]: capture provenance that never replaces
/// that chain code. Every write settles it again, so a written code or identity
/// is overwritten by the settled one. [`Self::entries`] is the row read as a tree, for a
/// consumer that walks one shape, and [`Self::into_bytes`] re-emits the
/// message from it.
///
/// Serialization is inherited, not written: `field.clone().into_json()`
/// renders the row's schema, [`into_json_scalar`](crate::into_json_scalar)
/// its value, and [`from_json_scalar_with_field`](crate::from_json_scalar_with_field)
/// reads a value back typed, ordered and canonicalized against that field;
/// [`Self::into_row`] is the whole message as one fixed row.
///
/// ```
/// use std::sync::Arc;
///
/// use yggdryl::graph::{Element, Event};
/// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut symbol = DataType::utf8().required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_names(["Ticker"])?;
/// let mut qty = DataType::Int64.required_field("OrderQty");
/// qty.as_fix_mut().set_tag(38)?;
/// let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone()])?);
///
/// let root = DataType::from(StructType::from_fields([symbol, qty, DataType::utf8().nullable_field("9999")])?)
///     .required_field("NewOrderSingle");
/// let value = Scalar::from_struct([
///     ("Symbol", Scalar::from("AAPL")),
///     ("OrderQty", Scalar::from(100)),
///     ("9999", Scalar::from("custom")),
/// ])?;
/// let msg = FixMsg::with_registry(registry, root.clone(), value)?;
///
/// assert_eq!(msg.by_tag(55)?, Scalar::from("AAPL"));
/// assert_eq!(msg.by_name("ticker")?, Scalar::from("AAPL"));
/// assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"), "an unknown tag is kept");
/// // The identity is settled from what the message states.
/// assert_ne!(msg.get_currhashcode(), 0);
/// assert_eq!(msg.get_curruuid(), msg.time_uuid()?);
/// assert_eq!(msg.get_currunix(), msg.header().sendingtime());
///
/// // The row serializes through the paths every field and value share.
/// let root = msg.as_field();
/// let schema = root.clone().into_json()?;
/// assert!(schema.contains("FIX:tag"));
/// # Ok(())
/// # }
/// ```
pub struct FixMsg {
    registry: Arc<FixRegistry>,
    /// The event the message is: what the three graph traits answer.
    ///
    /// Boxed, because the event is forty facts and a message is moved
    /// through every stream by value. Its stamped kind is the message's
    /// `msgcat` - the business category the message's type files under, the
    /// dictionary's `FIX:msgcat`, or the one the row it was read from stated
    /// - which decides whether its cross code carries its side.
    event: Box<OperationEventFacts>,
    /// The standard header, typed.
    header: Box<FixHeader>,
    /// What the line said about the capture it was written for, typed: a
    /// bridge's own row header. What the *reader* said about the line is
    /// [`Self::carried`].
    capture: Box<FixCapture>,
    /// The FIX fields the message lifted out of its row: the prices, the
    /// quantities and the identifiers, each exactly as the message stated
    /// it. What the event *derives* from them is the event's.
    lifted: Box<FixLifted>,
    /// The market facts a write reached and the next settle states again
    /// off the fields, overwriting: one [`fact`] bit each.
    stale: u32,
    /// The market facts the row stated under a crate column, or a caller -
    /// a lifecycle walk - set through the graph traits: their word, which
    /// no write restates until a null recorded hands the fact back to the
    /// fields.
    stated: u32,
    /// The normalized market facts the row stated. The fields leave them as
    /// the row's word, so reconstruction keeps an explicit value rather than
    /// replacing it from a raw FIX field.
    row_stated: u64,
    /// `Text(58)`, where the message carries one.
    text: Option<SmolStr>,
    /// What a bridge stated under its own namespaces - `TECH.CLIENTID`,
    /// `AMON.…` - each under the key as the bridge spelled it, folded, in
    /// sorted order.
    metadata: BTreeMap<SmolStr, SmolStr>,
    /// Each root child's tag beside its position, in tag order.
    ///
    /// Resolved once, because reading a tag out of a child's metadata is a
    /// formatted key and a map lookup, and scanning the children per lookup
    /// pays it once per child.
    tags: Vec<(i32, usize)>,
    /// The first child of each name, built on the first tag no child
    /// declares and kept.
    ///
    /// [`Self::get_by_tag`] has two fallbacks past the index above - the
    /// child named as the dictionary names the tag, and the child named by
    /// the tag's decimal spelling - and both end in a child found by name.
    /// Derived from the field alone, and derived lazily, so a message nobody
    /// projects pays nothing for it; shared, so a clone - which holds the
    /// same field - keeps it for a reference count.
    named: OnceLock<Arc<FixMap<SmolStr, usize>>>,
    /// Group positions keyed by their `FIX:counter`, separate from tag values.
    groups: Vec<(i32, usize)>,
    field: Field,
    value: Scalar,
    /// The row read as a tree, derived on the first ask and dropped by
    /// every write. Shared, so a clone - which states the same row - keeps
    /// the tree for the cost of a reference count rather than deriving it
    /// again.
    entries: OnceLock<Arc<[FixEntry]>>,
    /// The capture's own cells: what the row this message was read from
    /// said for itself, each under the column's name. Provenance and never
    /// content - outside the code, the entries and the wire - stated back at
    /// the column of its name by [`Self::into_row`].
    carried: Vec<(SmolStr, Scalar)>,
    /// The security identifiers the message implies rather than states: the
    /// national code an ISIN carries, what a lifecycle's registry learned. The
    /// derived overlay - never on the wire, never in the arrival record - kept
    /// across settles and filling only a type the stated set leaves absent,
    /// each under the [`DERIVED`] source.
    derived: Identifiers,
    /// Which cells [FX detection](super::forex) wrote, one bit per cell:
    /// a later detection may rewrite or take back those and no other. A
    /// row read back, and a write of the cell, make it the row's word.
    detected_fx: u8,
    /// What the message states that its reading could not take as it
    /// stands: the parse's refusals first - a value that would not type, a
    /// counter disagreeing with its group - then what a settle dropped,
    /// rebuilt by every settle behind the parse's and folded as a union when
    /// messages merge. Never a column, never a digest input.
    anomalies: Vec<super::FixAnomaly>,
    /// How many of `anomalies` the parse recorded, which every settle keeps.
    arrival_anomalies: usize,
}

/// Whether `held` is the four parts of a session event joined by `:`,
/// compared in place rather than joined again.
fn is_session_event(
    held: &str,
    msgtype: &str,
    session: &str,
    context: &str,
    sequence: u64,
) -> bool {
    let mut rest = held;
    for part in [msgtype, session, context] {
        let Some(after) = rest
            .strip_prefix(part)
            .and_then(|after| after.strip_prefix(':'))
        else {
            return false;
        };
        rest = after;
    }
    rest.parse::<u64>().is_ok_and(|held| held == sequence)
        && rest.bytes().all(|byte| byte.is_ascii_digit())
        && (rest.len() == 1 || !rest.starts_with('0'))
}

/// One child a write lands: replaced at `at`, appended when there is none,
/// its field and value already resolved and typed.
pub(super) struct Write {
    pub(super) at: Option<usize>,
    pub(super) field: Field,
    pub(super) value: Scalar,
}

/// One write a key resolved to: a typed fact the holders own, or a row
/// child.
enum Staged {
    Typed(i32, Scalar),
    Row(Write),
}

/// The refusal a write to one of the capture's own columns earns, named by
/// the column it reached.
///
/// Loud rather than silent: a caller writing `sourceurl` on a message means
/// to state where a line came from, and answering nothing would leave it
/// believing the message says so.
fn refused_capture(name: &str, tag: i32) -> Error {
    identity::refused(
        name,
        "a field a message states",
        format_smolstr!(
            "the capture's own column {name} ({tag}), which whoever read the line states on the row"
        ),
    )
}

/// Adds one staged write to the batch, the later of two writes to one child
/// standing, whether both reached it or both would append it.
fn stage(writes: &mut Vec<Write>, write: Write) {
    let pending = writes.iter().position(|held| match (held.at, write.at) {
        (Some(held), Some(at)) => held == at,
        (None, None) => held.field.name() == write.field.name(),
        _ => false,
    });
    match pending {
        Some(pending) => writes[pending] = write,
        None => writes.push(write),
    }
}

/// What one landed write does to the tag and group indexes.
pub(super) struct Indexed {
    retired_tag: Option<i32>,
    retired_counter: Option<i32>,
    tag: Option<i32>,
    counter: Option<i32>,
    index: usize,
    /// Whether the write put a differently named child in a child's place.
    pub(super) renamed: bool,
    /// Whether the write added a child rather than replacing one.
    pub(super) appended: bool,
}

/// Stage proven writes without publishing a message or inventing a second row.
pub(super) fn stage_writes(
    root: &Field,
    row: &Scalar,
    writes: Vec<Write>,
) -> Result<(Field, Vec<Scalar>, Vec<Indexed>)> {
    let mut members = Vec::with_capacity(root.fields().len() + writes.len());
    members.extend_from_slice(root.fields());
    let held = row.as_sequence().unwrap_or_default();
    let mut values = Vec::with_capacity(held.len() + writes.len());
    values.extend_from_slice(held);
    let mut indexed: Vec<Indexed> = Vec::with_capacity(writes.len());
    for Write { at, field, value } in writes {
        let (index, replaced) = match at {
            Some(at) if at < members.len() && at < values.len() => {
                let replaced = std::mem::replace(&mut members[at], field);
                values[at] = value;
                (at, Some(replaced))
            }
            _ => {
                members.push(field);
                values.push(value);
                (members.len() - 1, None)
            }
        };
        let renamed = replaced
            .as_ref()
            .is_some_and(|replaced| replaced.name() != members[index].name());
        let appended = replaced.is_none();
        let held = replaced.as_ref().map(Field::as_fix);
        let written = members[index].as_fix();
        // A tag is resolved here as the column plan resolves one, so the
        // index a write leaves is the index a construction would have built:
        // a child named after a tag the dictionary does not explain answers
        // for that tag, and a reader that walks the index rather than the
        // names finds it.
        let resolved = |field: &Field| {
            field
                .as_fix()
                .tag()
                .ok()
                .flatten()
                .or_else(|| super::field::parse_tag(field.name()))
        };
        indexed.push(Indexed {
            retired_tag: replaced.as_ref().and_then(resolved),
            retired_counter: held.as_ref().and_then(|held| held.counter().ok().flatten()),
            tag: resolved(&members[index]),
            counter: written.counter().ok().flatten(),
            index,
            renamed,
            appended,
        });
    }
    // A write replaces the child it reached or appends a field no child is
    // named as, so the members stay named once.
    let dtype = DataType::from(StructType::from_unique_fields(members));
    let field = Field::new_with_metadata(
        root.name(),
        dtype,
        root.is_nullable(),
        root.as_metadata().clone(),
    );
    Ok((field, values, indexed))
}

impl Indexed {
    pub(super) fn apply(
        &self,
        tags: &mut Vec<(i32, usize)>,
        groups: Option<&mut Vec<(i32, usize)>>,
    ) {
        retire(tags, self.retired_tag, self.index);
        admit(tags, self.tag, self.index);
        if let Some(groups) = groups {
            retire(groups, self.retired_counter, self.index);
            admit(groups, self.counter, self.index);
        }
    }
}

/// Each child's resolved tag beside its position, sorted for a binary search.
fn tag_positions(columns: &[super::schema::Column]) -> Vec<(i32, usize)> {
    let mut held = Vec::with_capacity(columns.len());
    held.extend(
        columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| Some((column.tag?, index))),
    );
    held.sort_unstable();
    held
}

/// Two instants standing no further apart than `delay`, in either
/// direction: what makes an official clock and a sending clock the one
/// event said twice. A nonpositive delay admits only equality.
fn within(unix: i64, reference: i64, delay: i64) -> bool {
    unix.abs_diff(reference) <= delay.unsigned_abs()
}

/// The rank `TransactTime(60)` takes among the official clocks: what the
/// message itself says about when its transaction happened, which no stamp
/// of another party outranks.
const TRANSACT_RANK: u8 = 0;

/// The `TrdRegTimestampType(770)` codes that stamp the event this message
/// reports - when it executed, when it entered the book, when it took its
/// priority, when it was submitted, cancelled or modified. These are the
/// venue's own answer to the question `currunix` asks.
const EVENT_TRDREG_TYPES: [i64; 9] = [
    1,  // ExecutionTime
    5,  // BrokerExecution
    8,  // TimePriority
    9,  // OrderbookEntryTime
    10, // OrderSubmissionTime
    29, // OrderCancellationTime
    30, // OrderModificationTime
    32, // TradeCancellationTime
    33, // TradeModificationTime
];

/// The `TrdRegTimestampType(770)` codes that stamp a hop the message
/// crossed on its way here rather than the event itself. Nearer the event
/// than the sending clock and further from it than the stamps above, which
/// is exactly the rank they take.
const HOP_TRDREG_TYPES: [i64; 5] = [
    2,  // TimeIn
    3,  // TimeOut
    4,  // BrokerReceipt
    6,  // DeskReceipt
    31, // OrderRoutingTime
];

/// How well one `TrdRegTimestampType(770)` answers when the event this
/// message reports happened; `None` where it answers something else.
///
/// The code set names thirty-six stamps and twenty-two of them answer
/// something else. Nineteen are the trade's afterlife rather than the trade:
/// submission to clearing, public and non-public reporting and their updates,
/// confirmation, clearing, allocation, submission to a repository,
/// continuation events, valuation, an identifier's assignment, affirmation
/// and a bare update time all happen after the event and say nothing about
/// when it happened. Three are about something other than this message: a
/// previous time priority and a previous identifier describe the state it
/// replaced, and a reference time for the BBO describes the market it was
/// measured against. A code no set names is one of these until someone says
/// otherwise - an unranked stamp is silence, never a clock - so only the two
/// lists above date a message.
fn trdregtimestamp_rank(kind: i64) -> Option<u8> {
    if EVENT_TRDREG_TYPES.contains(&kind) {
        return Some(TRANSACT_RANK + 1);
    }
    if HOP_TRDREG_TYPES.contains(&kind) {
        return Some(TRANSACT_RANK + 2);
    }
    None
}

/// One typed clock as the nanoseconds since the epoch it counts; nothing
/// where the value is not an instant, which is what a clock the dictionary
/// does not type as one, or one that would not read, leaves in the row.
fn instant_of(value: &Scalar) -> Option<i64> {
    let held @ Scalar::DateTime64(_) = value else {
        return None;
    };
    held.temporal_count_at(crate::TimeUnit::Nanosecond)
}

/// One code of a set as the number it is, however the dictionary typed the
/// tag: the integer a code set answers with, else the digits a dictionary
/// that left the tag as text carries.
fn code_of(value: &Scalar) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// The position of the child carrying one tag among `fields`, by the
/// dictionary's own reading of each child. A group occurrence is positional,
/// the names living on the field and never in the value, so this is found once
/// on the occurrence's field and every occurrence is read by it.
fn position_of_tag(registry: &FixRegistry, fields: &[Field], tag: i32) -> Option<usize> {
    fields
        .iter()
        .position(|child| super::schema::tag_and_counter(registry, child).0 == Some(tag))
}

fn group_positions(field: &Field, registry: &FixRegistry) -> Vec<(i32, usize)> {
    let mut held: Vec<_> = field
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            Some((super::schema::tag_and_counter(registry, child).1?, index))
        })
        .collect();
    held.sort_unstable();
    held
}

impl FixMsg {
    /// The deterministic hash of this message's schema and row.
    /// Uses one allocation for the shared XXH3 state, independent of message size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    /// Builds a message against the process-wide registry.
    ///
    /// # Errors
    ///
    /// Returns the default registry's load failure, or the refusal
    /// [`Self::with_registry`] raises.
    pub fn new(field: Field, value: Scalar) -> Result<Self> {
        Self::with_registry(Arc::clone(FixRegistry::from_env()?), field, value)
    }

    /// Builds a message against an explicit registry.
    ///
    /// The value is validated through [`Field::validate_value`] and stored
    /// as [`Field::canonicalize_value`] rewrites it, so a record becomes the
    /// ordered sequence the root declares. A child stating a typed fact -
    /// a header tag, a crate column, the event's own tags - fills the
    /// holder that owns it and leaves the row.
    ///
    /// # Errors
    ///
    /// Returns an error when the root is not a Struct field, or when the
    /// value violates it, naming the path of the first value that does not
    /// fit.
    pub fn with_registry(registry: Arc<FixRegistry>, field: Field, value: Scalar) -> Result<Self> {
        let value = field.canonicalize_value(value)?;
        Self::assemble(
            registry,
            field,
            value,
            None,
            None,
            super::FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_NS,
            true,
        )
    }

    /// Builds the message the one builder finished, checking nothing twice.
    ///
    /// Every value in the row went through the contract of the field it
    /// lands under, so the row is canonical by construction. The sending
    /// clock is what the message stated, else `fallback_sending_time` - its
    /// carrier's, else the codec's default - else now: initial intake
    /// settles clocks, and replay never reads now.
    /// `official_time_delay_ns` is how far from that clock an official
    /// transaction may stand and still date the message, which the codec
    /// states for the whole run. The identity is not settled here: the
    /// [enriching pass](super::enrich) that takes every built message
    /// restates and fills it first, and settles it once at its end, so
    /// nothing is digested that a later write of the same pass rewrites.
    pub(super) fn from_built(
        registry: Arc<FixRegistry>,
        built: super::build::Built,
        fallback_sending_time: Option<&Scalar>,
        source: Option<Uuid>,
        official_time_delay_ns: i64,
    ) -> Result<Self> {
        let super::build::Built {
            field,
            value,
            anomalies,
            ..
        } = built;
        let mut message = Self::assemble(
            registry,
            field,
            value,
            fallback_sending_time,
            source,
            official_time_delay_ns,
            false,
        )?;
        // What arrived leads what the fields' reading recorded.
        message.arrival_anomalies = anomalies.len();
        message.anomalies.splice(0..0, anomalies);
        Ok(message)
    }

    /// Builds a message from the content reconstructed out of a semantic row.
    /// The rebuilt nested tree is canonicalized once against its newly built
    /// root. A row carrying its complete event identity keeps that recorded
    /// identity; a narrower row is settled from the facts it does carry.
    pub(super) fn from_rebuilt_row(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        retains_identity: bool,
    ) -> Result<Self> {
        let value = field.canonicalize_value(value)?;
        let mut message = Self::assemble(
            registry,
            field,
            value,
            None,
            None,
            super::FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_NS,
            false,
        )?;
        message.sync_session_event_identifier();
        if retains_identity {
            message.rebuild_idmaps();
            message.event.fill_market();
            message.fill_parents();
        } else {
            message.settle();
        }
        Ok(message)
    }

    /// One message from a root and its canonical row: the typed facts are
    /// lifted out of the children that state them, the rest is the row,
    /// the clocks are settled and, where `settle` says so, the identity
    /// derived. `source` is the identity of the line the row was parsed out
    /// of, stated as the message's one source before it is settled; a row
    /// stating a `srcuuids` column of its own states those instead.
    /// `official_time_delay_ns` is how far from the sending clock an
    /// official transaction may stand and still date the message.
    fn assemble(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        fallback_sending_time: Option<&Scalar>,
        source: Option<Uuid>,
        official_time_delay_ns: i64,
        settle: bool,
    ) -> Result<Self> {
        let plan = super::schema::column_plan_of(&field, &registry)?;
        let held = value.as_sequence().ok_or_else(|| {
            identity::refused(field.name(), "a canonical Struct row", value.kind())
        })?;
        let mut event = Box::new(OperationEventFacts::default());
        if let Some(source) = source {
            event.set_srcuuids(vec![source]);
        }
        let mut header = Box::new(FixHeader::unknown());
        let mut capture = Box::new(FixCapture::default());
        let mut lifted = Box::new(FixLifted::default());
        let mut text = None;
        let mut metadata = BTreeMap::new();
        let mut members = Vec::with_capacity(field.fields().len());
        let mut values = Vec::with_capacity(held.len());
        let mut kept = Vec::with_capacity(plan.len());
        let mut stated_sending = false;
        let mut stated_unix = false;
        let mut stated_creation = false;
        // Whether the row has a place for the recording at all: a row that
        // does states it, a null included, and one that does not leaves it
        // to the sender's clock.
        let mut carries_recording = false;
        let mut row_stated = 0_u64;
        let mut stated = 0_u32;
        let mut msgcat = None;
        let mut settled_last = [None; SETTLED_LAST.len()];
        // A typed tag is lifted out of the row onto its holder, and a holder
        // keeps one fact per tag - so a row stating one tag twice is left
        // where it stands rather than collapsed into one slot. Two children
        // under `ClOrdID(11)`, a venue's own beside the client's, are two
        // facts and a reader addressing them by name must still find both.
        for ((child, column), value) in field.fields().iter().zip(plan.iter()).zip(held) {
            match column.tag {
                Some(tag) if identity::is_typed_tag(tag) && !column.shared => {
                    carries_recording |= tag == super::RECDUNIX_TAG_NAME.0;
                    if value.is_null() {
                        continue;
                    }
                    if tag == identity::TEXT_TAG {
                        text = value.as_str().map(SmolStr::new);
                    } else if tag == super::METADATA_TAG_NAME.0 {
                        metadata = metadata_of(value);
                    } else if tag == super::MARKETDATAKIND_TAG_NAME.0 {
                        msgcat = msgcat_of(value);
                    } else if let Some(at) = SETTLED_LAST.iter().position(|last| *last == tag) {
                        settled_last[at] = Some(value);
                    } else {
                        identity::record(
                            &mut event,
                            &mut header,
                            &mut capture,
                            &mut lifted,
                            tag,
                            value,
                        );
                    }
                    stated |= fact_of_crate_tag(tag);
                    stated_sending |= tag == 52;
                    stated_unix |= tag == super::CURRUNIX_TAG_NAME.0;
                    stated_creation |= tag == super::CREAUNIX_TAG_NAME.0;
                    if let Some(bit) = row_stated_bit(tag) {
                        row_stated |= bit;
                    }
                }
                // A key spelled under a namespace - `TECH.CLIENTID` - is a
                // bridge's own statement and goes to the metadata, under
                // the key as the bridge spelled it, folded; so does an alias
                // that lost to the field it names while stating another
                // value, which no field holds and the wire never re-emits.
                None if child.name().contains('.')
                    || child.get_metadata(super::field::ALIAS_OF).is_some() =>
                {
                    if let Some(held) = spelled(value).filter(|held| !held.is_empty()) {
                        metadata.insert(SmolStr::new(child.name()), held);
                    }
                }
                // The capture's own column, whoever built the root: the
                // object the line was read from, the instant it was
                // recorded. Read past, because a message states nothing
                // about the reading it arrived through - and never kept as
                // a child, which would make it content the wire re-emits.
                Some(tag) if identity::is_capture_tag(tag) => {}
                _ => {
                    members.push(child.clone());
                    values.push(value.clone());
                    kept.push(*column);
                }
            }
        }
        if !stated_sending {
            // A row read back states the instant it was dated by, and that
            // instant is its sending clock again: only an intake stating no
            // clock at all reads the wall clock, so a replay is a fixed point.
            let stated = if stated_unix {
                Some(event.get_currunix())
            } else if stated_creation {
                event.get_creaunix()
            } else {
                None
            };
            let sending = match (
                fallback_sending_time.filter(|value| !value.is_null()),
                stated,
            ) {
                (Some(value), _) => value.clone(),
                (None, Some(unix)) => {
                    Scalar::datetime64(unix, crate::TimeUnit::Nanosecond, crate::Timezone::UTC)?
                }
                (None, None) => identity::now()?,
            };
            identity::validate_value("sendingtime", &super::schema::CLOCK_DATATYPE, &sending)?;
            if let Some(unix) = sending.temporal_count_at(crate::TimeUnit::Nanosecond) {
                header.set_sendingtime(unix);
            }
        }
        // The category the row stated; one it states none of - or names
        // none by - the fields state below, against the state they reach.
        match msgcat {
            Some(msgcat) => event.set_marketdatakind(msgcat),
            None => stated &= !fact::KIND,
        }
        // The cross code is stored under the category, so it is recorded once
        // that stands; the hash and the elements the row states after it, so
        // what the row states stands over what the code derives.
        for (tag, value) in SETTLED_LAST.into_iter().zip(settled_last) {
            if let Some(value) = value {
                identity::record(
                    &mut event,
                    &mut header,
                    &mut capture,
                    &mut lifted,
                    tag,
                    value,
                );
            }
        }
        // The members are the planned root's children less the lifted
        // ones, named once as that root named them.
        let field = Field::new_with_metadata(
            field.name(),
            DataType::from(StructType::from_unique_fields(members)),
            field.is_nullable(),
            field.as_metadata().clone(),
        );
        // The row keeps the planned root's children less the lifted ones,
        // so its plan is those children's, read once above.
        let plan: super::schema::ColumnPlan = Arc::from(kept);
        let tags = tag_positions(&plan);
        let groups = group_positions(&field, &registry);
        let mut message = Self {
            registry,
            event,
            header,
            capture,
            lifted,
            stale: 0,
            stated,
            row_stated,
            text,
            metadata,
            tags,
            named: OnceLock::new(),
            groups,
            field,
            value: Scalar::from_sequence(values),
            entries: OnceLock::new(),
            carried: Vec::new(),
            derived: Identifiers::new(),
            detected_fx: 0,
            anomalies: Vec::new(),
            arrival_anomalies: 0,
        };
        // The instant the message happened: what it states, else when the
        // transaction it reports happened, else when it was sent - the one
        // clock every message carries. A parse structures what a line said
        // and dates it against the sending clock, taking the official
        // transaction over it only where the two stand within
        // `official_time_delay_ns` of each other and are therefore the one
        // event said twice; what a resend's `OrigSendingTime(122)` says, and
        // what a `TransactTime(60)` further off than that says, is the
        // lifecycle's to read off the structured message.
        if !stated_unix {
            message
                .event
                .set_currunix(message.official_unix(official_time_delay_ns));
        }
        if !stated_creation {
            message.event.set_creaunix(Some(message.get_currunix()));
        }
        if !carries_recording {
            message.record_at_sending();
        }
        // What the message implies about its market, read off the FIX
        // fields it stated; what the row stated stands as its word.
        message.state_market(fact::ALL);
        if settle {
            message.settle();
        }
        Ok(message)
    }

    /// The instant this message happened: the best official clock standing
    /// within `delay` of the sending clock, else the sending clock itself.
    ///
    /// The sending clock is the reference because every message carries one
    /// and no message carries two. An official clock is the more exact
    /// saying of when the event happened, and the distance between the two
    /// is the only evidence a parse has that they are saying the same thing:
    /// inside the delay they are one event and the official clock wins,
    /// outside it they are two and the parse keeps the clock it can trust.
    /// Rank decides between several that qualify - what the message says its
    /// transaction was before what a regulatory stamp says a hop was - and
    /// the nearer of two equal ranks decides after that, the earlier
    /// instant closing the last tie so that one row reads one way.
    fn official_unix(&self, delay: i64) -> i64 {
        let sending = self.header.sendingtime();
        self.official_clocks()
            .filter(|(_, unix)| within(*unix, sending, delay))
            .min_by_key(|(rank, unix)| (*rank, unix.abs_diff(sending), *unix))
            .map_or(sending, |(_, unix)| unix)
    }

    /// Every clock this message states about when its own event happened,
    /// each under the rank that says how well it answers that question.
    ///
    /// `TransactTime(60)` is the message's own statement and outranks
    /// everything; the `TrdRegTimestamps(768)` group is the venue's, and
    /// [`trdregtimestamp_rank`] reads each occurrence's
    /// `TrdRegTimestampType(770)` to say which of its stamps are about this
    /// event at all. Nothing here is ordered or bounded: the caller bounds
    /// them by the delay and picks one.
    fn official_clocks(&self) -> impl Iterator<Item = (u8, i64)> + use<'_> {
        self.transact_unix()
            .map(|unix| (TRANSACT_RANK, unix))
            .into_iter()
            .chain(self.trdregtimestamps())
    }

    /// The `TransactTime(60)` this message states, as an instant.
    ///
    /// By the index alone: a lookup that falls through to the name table is a
    /// read a dating has no use for. A transaction stating a day and no clock
    /// dates nothing - `60=20260814`, which the parse restates as that day's
    /// midnight - because what it says is the day, and midnight to the
    /// nanosecond is that statement and no other a venue makes.
    fn transact_unix(&self) -> Option<i64> {
        instant_of(&self.indexed_by_tag(60)?).filter(|unix| unix.rem_euclid(NANOS_PER_DAY) != 0)
    }

    /// Every `TrdRegTimestamp(769)` this message states that is about the
    /// event the message reports, under its rank.
    ///
    /// The group is read as a group and only as a group: `TrdRegTimestamp`
    /// says nothing on its own - the same tag carries the execution's
    /// instant, a desk's receipt and the moment a report reached a
    /// repository - and what tells them apart is the
    /// `TrdRegTimestampType(770)` standing beside it in the same
    /// occurrence. A dictionary that declares the group pairs them; one that
    /// does not leaves two flat children whose pairing is a guess, and a
    /// guess about which regulatory clock this is would date the message by
    /// a stamp that belongs to a different question.
    ///
    /// A group occurrence is positional - the names live on the field and
    /// never in the value - so the two members are found once on the
    /// occurrence's field and every occurrence is read by those positions.
    fn trdregtimestamps(&self) -> impl Iterator<Item = (u8, i64)> + use<'_> {
        self.trdregtimestamp_members()
            .into_iter()
            .flat_map(|(occurrences, stamp, kind)| {
                occurrences.iter().filter_map(move |occurrence| {
                    let held = occurrence.as_sequence()?;
                    let rank = trdregtimestamp_rank(code_of(held.get(kind)?)?)?;
                    Some((rank, instant_of(held.get(stamp)?)?))
                })
            })
    }

    /// The `TrdRegTimestamps(768)` occurrences beside the positions its
    /// `TrdRegTimestamp(769)` and `TrdRegTimestampType(770)` members hold in
    /// each of them; nothing where the dictionary declares no such group, or
    /// where the group it declares does not carry both members.
    fn trdregtimestamp_members(&self) -> Option<(&Serie, usize, usize)> {
        let at = self.index_of_group(768)?;
        let sequence = (self.field.fields().get(at)?.dtype()).as_serie_type()?;
        let members = sequence.item().fields();
        let stamp = position_of_tag(&self.registry, members, 769)?;
        let kind = position_of_tag(&self.registry, members, 770)?;
        let occurrences = self.value.as_sequence()?.get(at)?.as_serie()?;
        Some((occurrences, stamp, kind))
    }

    /// Replaces the row with another statement of the same content, as a
    /// restatement leaves it: the typed facts a restated child states fill
    /// their holders and the indexes are reread. The identity is not
    /// settled: the pass that restates settles once, after everything it
    /// writes.
    pub(super) fn replace_content(&mut self, field: Field, values: Vec<Scalar>) -> Result<()> {
        // Another statement of the whole content: every fact is stated
        // again off it at the settle that follows.
        self.stale |= fact::ALL;
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        let mut content = Vec::with_capacity(field.fields().len());
        let mut kept = Vec::with_capacity(values.len());
        for ((child, column), value) in field.fields().iter().zip(plan.iter()).zip(values) {
            let is_content = match column.tag {
                Some(tag) if identity::is_typed_tag(tag) => {
                    if !value.is_null() {
                        self.record(tag, &value);
                    }
                    false
                }
                // The capture's own column, which no restatement of the
                // content can make a fact of the message: read past, as
                // every other door reads it past.
                Some(tag) if identity::is_capture_tag(tag) => false,
                None if child.name().contains('.')
                    || child.get_metadata(super::field::ALIAS_OF).is_some() =>
                {
                    if let Some(held) = value.as_str().filter(|held| !held.is_empty()) {
                        self.metadata
                            .insert(SmolStr::new(child.name()), SmolStr::new(held));
                    }
                    false
                }
                _ => true,
            };
            content.push(is_content);
            if is_content {
                kept.push(value);
            }
        }
        // Where every child is content, the root handed in is the row's
        // own and its plan is the one just read.
        let plan = if kept.len() == field.fields().len() {
            self.field = field;
            plan
        } else {
            let members = field
                .fields()
                .iter()
                .zip(&content)
                .filter(|(_, is_content)| **is_content)
                .map(|(child, _)| child.clone())
                .collect();
            self.field = Field::new_with_metadata(
                field.name(),
                DataType::from(StructType::from_unique_fields(members)),
                field.is_nullable(),
                field.as_metadata().clone(),
            );
            super::schema::column_plan_of(&self.field, &self.registry)?
        };
        self.value = Scalar::from_sequence(kept);
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field, &self.registry);
        self.named = OnceLock::new();
        self.entries = OnceLock::new();
        Ok(())
    }

    /// Records one typed fact on the holder that owns it; a null clears it.
    /// A row-owned fact recorded is the row's word from then on, and a null
    /// recorded hands it back to derivation.
    pub(super) fn record(&mut self, tag: i32, value: &Scalar) -> bool {
        if tag == identity::TEXT_TAG {
            self.text = value.as_str().map(SmolStr::new);
            return true;
        }
        if tag == super::METADATA_TAG_NAME.0 {
            // A key no dictionary maps may name a security identifier, which
            // the next settle reads again.
            self.metadata = metadata_of(value);
            self.stale |= fact::SECURITYIDS;
            return true;
        }
        if tag == super::MARKETDATAKIND_TAG_NAME.0 {
            // A null, or a value naming no category, hands it back to the
            // fields, which the next settle reads it off again.
            match msgcat_of(value) {
                Some(msgcat) => {
                    self.event.set_marketdatakind(msgcat);
                    self.stated |= fact::KIND;
                    self.stale &= !fact::KIND;
                }
                None => {
                    self.stated &= !fact::KIND;
                    self.stale |= fact::KIND;
                }
            }
            return true;
        }
        let recorded = identity::record(
            &mut self.event,
            &mut self.header,
            &mut self.capture,
            &mut self.lifted,
            tag,
            value,
        );
        if recorded {
            if let Some(bit) = row_stated_bit(tag) {
                if value.is_null() {
                    self.row_stated &= !bit;
                } else {
                    self.row_stated |= bit;
                }
            }
            // A crate column recorded is its fact's word, which no pending
            // restatement replaces; a null hands the fact back to the
            // fields. A FIX field recorded - a lifted price - is a write of
            // what its facts read.
            match fact_of_crate_tag(tag) {
                0 => self.stale |= facts_of_tag(&self.registry, tag),
                fact if value.is_null() => {
                    self.stated &= !fact;
                    self.stale |= fact;
                }
                fact => {
                    self.stated |= fact;
                    self.stale &= !fact;
                }
            }
        }
        recorded
    }

    /// What the message states under one typed tag, as the tag's own field
    /// types it.
    ///
    /// A holder keeps a number at the one width this crate keeps a number
    /// at, and a dictionary may type the tag's column narrower, so the
    /// answer is narrowed to the column that names it: one tag answers one
    /// type whether it is read here, off the row, or out of an Arrow
    /// column.
    fn typed_fact(&self, tag: i32) -> Option<Scalar> {
        if tag == identity::TEXT_TAG {
            return self.text.as_deref().map(Scalar::from);
        }
        if tag == super::METADATA_TAG_NAME.0 {
            if self.metadata.is_empty() {
                return None;
            }
            return Scalar::from_mapping(
                self.metadata
                    .iter()
                    .map(|(key, value)| (Scalar::from(key.as_str()), Scalar::from(value.as_str()))),
            )
            .ok();
        }
        if tag == super::MARKETDATAKIND_TAG_NAME.0 {
            return Some(Scalar::MarketDataKind(self.msgcat()));
        }
        let fact = Typed {
            event: &self.event,
            header: &self.header,
            capture: &self.capture,
            lifted: &self.lifted,
        }
        .fact(tag)?;
        Some(match self.registry.get_field_by_tag(tag) {
            Some(field) => super::schema::narrowed(field, fact),
            None => fact,
        })
    }

    /// What the message states that its reading could not take as it
    /// stands, in arrival order: the parse's refusals - a value that would
    /// not type, a counter disagreeing with its group - then what the last
    /// settle dropped. Read off the message beside the row: never a column,
    /// never part of the code it digests to. Two statements of one message
    /// merge them as a union, the reference's first.
    #[must_use]
    pub fn anomalies(&self) -> &[super::FixAnomaly] {
        &self.anomalies
    }

    /// Keeps `anomaly` beside the message as one its parse recorded, once:
    /// what a split could not take as it stands.
    pub(super) fn note_anomaly(&mut self, anomaly: super::FixAnomaly) {
        if !self.anomalies[..self.arrival_anomalies].contains(&anomaly) {
            self.anomalies.insert(self.arrival_anomalies, anomaly);
            self.arrival_anomalies += 1;
        }
    }

    /// Takes the parse's anomalies of another statement of this message
    /// into this one's, each once, so a merge loses no refusal either side
    /// recorded; what a settle drops is this message's own to record again.
    fn fold_anomalies(&mut self, other: &Self) {
        for anomaly in &other.anomalies[..other.arrival_anomalies] {
            if !self.anomalies[..self.arrival_anomalies].contains(anomaly) {
                self.anomalies
                    .insert(self.arrival_anomalies, anomaly.clone());
                self.arrival_anomalies += 1;
            }
        }
    }

    /// Settles the identity from what the message states: the cross code
    /// where it names none yet, the cross codes in step with it, the code
    /// the content digests to, and the identity the instant and that code
    /// derive.
    ///
    /// The cross code names the chain and defaults to the first nonempty FIX
    /// identifier in [`identity::CROSS_TAGS`], `OrderID(37)` first. The
    /// message type, capture session/context and message sequence name where
    /// a bridge observed the message instead: when all four are present,
    /// `sync_session_event_identifier` states their joined values as the
    /// capture's `msgsesseventid` without making it content.
    pub(super) fn settle(&mut self) {
        self.sync_session_event_identifier();
        if self.event.get_crosscode().is_empty() {
            let code = identity::CROSS_TAGS.iter().find_map(|tag| {
                self.get_by_tag(*tag)
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .filter(|code| !code.is_empty())
            });
            if let Some(code) = code {
                self.event.set_crosscode(code);
            }
        }
        // What a write reached, stated again off the FIX fields it moved -
        // overwriting - before the code the message answers to covers
        // either.
        let stale = std::mem::take(&mut self.stale);
        self.state_market(stale);
        self.rebuild_idmaps();
        self.event.fill_market();
        self.fill_parents();
        self.event.sync_cross();
        let currhashcode = self.currhashcode();
        self.event.finalized(currhashcode);
    }

    /// Derives the complete FIX session event onto the capture: the four
    /// values joined by `:`, exactly as stated, and nothing where one is
    /// missing - a stale key never outlives the parts it was joined from.
    /// `:` is how a bridge's own row header brackets a session, its context
    /// and its sequence, so the key reads as the header does; values holding
    /// a `:` of their own can join to one key from two splits, and a bridge
    /// names none of its sessions or contexts that way.
    fn sync_session_event_identifier(&mut self) {
        let identity = match (
            self.header.msgtype(),
            self.capture.msgsessionid(),
            self.capture.msgctxid(),
            self.header.msgseqnum(),
        ) {
            (msgtype, Some(session), Some(context), Some(sequence))
                if !msgtype.is_empty() && !session.is_empty() && !context.is_empty() =>
            {
                Some((msgtype, session, context, sequence))
            }
            _ => None,
        };
        let joined = identity.map(|(msgtype, session, context, sequence)| {
            if self
                .capture
                .msgsesseventid()
                .is_some_and(|held| is_session_event(held, msgtype, session, context, sequence))
            {
                return None;
            }
            let mut joined =
                String::with_capacity(msgtype.len() + session.len() + context.len() + 3 + 20);
            write!(joined, "{msgtype}:{session}:{context}:{sequence}")
                .expect("writing into a String cannot fail");
            Some(SmolStr::from(joined))
        });
        match joined {
            // Already the key the four parts join to.
            Some(None) => {}
            Some(Some(joined)) => self.capture.set_msgsesseventid(Some(joined)),
            None => self.capture.set_msgsesseventid(None),
        }
    }

    /// The complete prebuilt session-event delivery identity, where present.
    pub(super) fn session_event_identifier(&self) -> Option<&str> {
        self.capture.msgsesseventid()
    }

    /// Whether two observations state one complete session event: one
    /// delivery, read as the same category of market data. The messages a
    /// parse splits one delivery into - an order's report beside its
    /// execution, a trade beside its sided executions, an unsided quote
    /// beside its sided quotes - share the delivery and are never another
    /// observation of one another: their categories differ, and where they
    /// do not their chains do, which is what keeps them apart before the
    /// walk (the lifecycle's own key adds the side and an execution's cross
    /// code) and inside it (each walks a chain of its own).
    pub(super) fn is_same_session_event(&self, other: &Self) -> bool {
        self.msgcat() == other.msgcat()
            && self
                .session_event_identifier()
                .is_some_and(|identity| other.session_event_identifier() == Some(identity))
    }

    /// Whether this is the graph's derived expiry rather than another raw FIX
    /// observation. It deliberately keeps the source session-event identity,
    /// but remains a later lifecycle event and must follow instead of merge.
    fn is_synthetic_expiry(&self) -> bool {
        *self.get_state() == State::Expired
            && self.get_recdunix().is_none()
            && self.get_exprunix() == Some(self.get_currunix())
    }

    /// Whether `with_previous` must treat the two values as observations of
    /// one FIX event rather than successive lifecycle events.
    pub(super) fn should_merge_session_event(&self, other: &Self) -> bool {
        self.is_same_session_event(other) && !self.is_synthetic_expiry()
    }

    /// Fully merge another observation of this session event, retaining the
    /// latest recording as the reference row and the earliest precise facts.
    pub(super) fn merge_session_event(self, other: &Self) -> Result<Self> {
        debug_assert!(self.is_same_session_event(other));
        let other_leads = crate::graph::element::right_is_reference(
            self.get_recdunix(),
            self.get_currunix(),
            other.get_recdunix(),
            other.get_currunix(),
        );
        if other_leads {
            return other.clone().fold_session_event(&self);
        }
        self.fold_session_event(other)
    }

    /// Fully merge another observation of this session event into this one,
    /// which stays the reference whatever the two recording clocks say: for
    /// a caller that already chose it, as a run of observations sorted
    /// latest recording first does, where re-deciding at every pair would
    /// let a later one lead against the earliest recording a fold keeps.
    pub(super) fn fold_session_event(mut self, other: &Self) -> Result<Self> {
        debug_assert!(self.is_same_session_event(other));
        super::latest::merge_content(&mut self, other)?;
        self.fold_session_event_facts(other);
        Ok(self)
    }

    /// [`Self::fold_session_event`] for an observation whose content an
    /// earlier fold already merged: the content merge fills only what the
    /// reference leaves missing, so a second merge of the same content moves
    /// nothing, and what is left to fold is the event, the anomalies and the
    /// provenance - in place, and infallibly.
    pub(super) fn fold_session_event_facts(&mut self, other: &Self) {
        debug_assert!(self.is_same_session_event(other));
        crate::graph::market::merge_operation_event_into_reference(self, other);
        self.fold_anomalies(other);
        self.fold_provenance(other);
    }

    /// Keeps the provenance the earlier observation of this session event
    /// states - its originator, its conversation - over the later one's,
    /// and a later one stating something else is kept as an anomaly beside
    /// the merged message, as a refusal the parse recorded is.
    fn fold_provenance(&mut self, other: &Self) {
        let other_earlier = crate::graph::element::right_is_reference(
            other.get_recdunix(),
            other.get_currunix(),
            self.get_recdunix(),
            self.get_currunix(),
        );
        for (tag, name) in [
            super::MSGORIGINATOR_TAG_NAME,
            super::CONVERSATIONID_TAG_NAME,
        ] {
            let (mine, theirs) = (self.capture.provenance(tag), other.capture.provenance(tag));
            let (earlier, later) = if other_earlier {
                (theirs, mine)
            } else {
                (mine, theirs)
            };
            let kept = earlier.or(later).map(SmolStr::new);
            if let (Some(earlier), Some(later)) = (earlier, later)
                && earlier != later
            {
                let anomaly = super::FixAnomaly::new(
                    name,
                    format!("states {later} where an earlier observation states {earlier}"),
                );
                if !self.anomalies[..self.arrival_anomalies].contains(&anomaly) {
                    self.anomalies.insert(self.arrival_anomalies, anomaly);
                    self.arrival_anomalies += 1;
                }
            }
            self.capture.set_provenance(tag, kept.as_deref());
        }
    }

    /// Whether an explicitly stated `ExecType(150)` reports an execution.
    /// Its raw FIX code is inspected, never the lifecycle state derived from
    /// `OrdStatus`: trade corrections, cancels and clearing transitions can
    /// carry a filled order state without being executions themselves.
    fn explicit_execution_type(&self) -> Option<bool> {
        self.get_by_tag(150)
            .map(|held| matches!(held.as_str(), Some("F" | "1" | "2")))
    }

    /// Whether this is an execution report of no fill: a type filed under
    /// `EXEC` that reports no execution, its order's or its quote's report.
    pub(super) fn reports_no_fill(&self) -> bool {
        filed_msgcat(&self.registry, self.header.msgtype()) == MarketDataKind::Execution
            && !self.reports_execution()
    }

    /// Whether this message reports an execution rather than merely carrying
    /// execution-shaped fields. A TradeCaptureReport may omit `ExecType(150)`;
    /// its initial `TradeReportTransType(487)` is then the execution signal.
    /// Requests and acknowledgements never become executions, and a cancel,
    /// replace, release or reverse report is a lifecycle action rather than a
    /// new precise execution.
    pub(super) fn reports_execution(&self) -> bool {
        let msgtype = self.header.msgtype();
        if msgtype == "AE" {
            let new_report = self.get_by_tag(487).is_none_or(|held| {
                held.is_null()
                    || held.as_i64() == Some(0)
                    || held.as_str().is_some_and(|value| {
                        matches!(value, "0" | "N") || crate::folds_equal(value, "New")
                    })
            });
            return new_report && self.explicit_execution_type() != Some(false);
        }
        if matches!(msgtype, "AD" | "AQ" | "AR") {
            return false;
        }
        self.explicit_execution_type()
            .unwrap_or_else(|| self.event.is_execution())
    }

    /// One FIX or proprietary timestamp read as the event clock the graph
    /// keeps. Dictionary timestamps are already typed; an unresolved bridge
    /// key is read once through the crate execution field's FIX spelling.
    fn execution_instant(&self, value: Option<Scalar>) -> Option<i64> {
        let value = value?;
        value
            .temporal_count_at(crate::TimeUnit::Nanosecond)
            .or_else(|| {
                let text = value.as_str()?;
                let field = self.registry.get_field_by_tag(super::EXECUNIX_TAG_NAME.0)?;
                super::build::typed_spelling(&self.registry, field, text)
                    .temporal_count_at(crate::TimeUnit::Nanosecond)
            })
    }

    /// The first regulatory timestamp explicitly classified as execution
    /// time. A timestamp of any other type says nothing about execution.
    fn trdreg_execution_instant(&self) -> Option<i64> {
        let at = self.index_of_group(768)?;
        let column = self.field.fields().get(at)?;
        let sequence = (column.dtype()).as_serie_type()?;
        let item = sequence.item();
        let (timestamp, kind) = (
            item.index_of("trdregtimestamp")?,
            item.index_of("trdregtimestamptype")?,
        );
        self.value
            .as_sequence()?
            .get(at)?
            .as_serie()?
            .iter()
            .find_map(|occurrence| {
                let held = occurrence.as_sequence()?;
                let kind = held.get(kind)?;
                let execution = kind.as_i64() == Some(1)
                    || kind.as_str().is_some_and(|kind| {
                        kind == "1" || crate::folds_equal(kind, "ExecutionTime")
                    });
                execution
                    .then(|| self.execution_instant(held.get(timestamp).cloned()))
                    .flatten()
            })
    }

    /// States the market facts `facts` names off the FIX fields, each
    /// through its [`Market`] setter with `overwrite`: the fields' reading
    /// replaces what the message held, a fact they no longer state cleared -
    /// except a fact the row or a caller stated under a crate column
    /// ([`FixMsg::record`]), which is their word: the fields only fill it.
    ///
    /// A message states every fact as it is built; a write marks the facts
    /// its tags feed ([`facts_of_tag`]) and the next settle states them
    /// again - so a write moves exactly what it reaches, and what a
    /// lifecycle walk set on the message stands until a field it was read
    /// from is written. What a setter implies past its own fact - the bid of
    /// a buyer's price, the part an iceberg hides, the third of an FX
    /// triple - it fills itself ([`Market`]), so the facts that fill one
    /// another are cleared and stated again together ([`fact::QUOTED`]),
    /// each overwriting what an earlier one filled, and the side last: a
    /// value the wire states always stands over a fill.
    ///
    /// The execution clock is the one the message's own fields state, and a
    /// raw observation reporting an execution - [`Event::is_execution`] as
    /// this message reads it - that states none executed when it happened,
    /// so its `execunix` is its own `currunix` from intake on rather than
    /// from the first walk. A message following another that states none
    /// keeps the execution its chain reached: that is the walk's to state,
    /// and its state may be one it inherited rather than one it reported.
    ///
    /// Nothing stated here reaches the wire, the arrival record or the code
    /// the message digests to: those read what the message *stated*, and a
    /// derived fact is not a statement.
    fn state_market(&mut self, facts: u32) {
        // The facts that fill one another move together.
        // The quantity is what an order still has open, so the quoting and
        // the order's quantities are one group.
        let group = fact::QUOTED | fact::ORDERED;
        let facts = if facts & group != 0 {
            facts | group
        } else {
            facts
        };
        let reached = |fact: u32| facts & fact != 0;
        // What the row or a caller stated under a crate column is its word:
        // the fields fill it where it is empty and never overwrite it.
        let stated = self.stated;
        let over = |fact: u32| stated & fact == 0;
        self.clear_market(facts & fact::CLEARED & !stated);
        if reached(fact::SECURITYIDS) {
            // The identifiers the fields state already rank a crated view the
            // row stated after the wire's, so they replace what the row
            // recorded - unless the row stated the whole set.
            self.state_securityids(over(fact::SECURITYIDS));
        }
        if reached(fact::STATE) {
            let state = State::FIX_STATUS_TAGS
                .into_iter()
                .find_map(|tag| {
                    self.stated_word(tag)
                        .and_then(|held| State::from_fix_status(tag, &held))
                })
                .or_else(|| State::from_fix_msgtype(self.header.msgtype()))
                .unwrap_or_else(State::unknown);
            if over(fact::STATE) || *self.event.get_state() == State::unknown() {
                self.event.set_state(state);
            }
        }
        if reached(fact::KIND) {
            // The category it files under, a report of no fill its order's or
            // its quote's report, read against the state it reached.
            let reports = self
                .explicit_execution_type()
                .unwrap_or_else(|| self.event.get_state().is_execution());
            let kind = derived_msgcat(
                &self.registry,
                self.header.msgtype(),
                self.lifted.quoteid().is_some(),
                reports,
            );
            if over(fact::KIND) || self.event.marketdatakind() == MarketDataKind::Unknown {
                self.event.set_marketdatakind(kind);
            }
        }
        if reached(fact::MDTYPE) {
            let mdtype = self.stated_marketdatatype().unwrap_or_default();
            self.event.set_marketdatatype(mdtype, over(fact::MDTYPE));
        }
        if reached(fact::EXPIRY) {
            let exprunix = [126, 62, 432, 541].into_iter().find_map(|tag| {
                self.stated_by_tag(tag)
                    .and_then(|held| held.temporal_count_at(crate::TimeUnit::Nanosecond))
            });
            if over(fact::EXPIRY) || self.event.get_exprunix().is_none() {
                self.event.set_exprunix(exprunix);
            }
        }
        if reached(fact::EXECUTION) {
            let execunix = self.execution_or_placed(self.stated_execution_instant());
            self.event.set_execunix(execunix, over(fact::EXECUTION));
        }
        if reached(fact::TIF) {
            let tif = self.stated_timeinforce();
            self.event.set_timeinforce(tif, over(fact::TIF));
        }
        if reached(fact::TICKER) {
            let ticker = self
                .stated_word(55)
                .filter(|held| held != "[N/A]" && held != "[N/A");
            self.event.set_ticker(ticker, over(fact::TICKER));
        }
        if reached(fact::CFI) {
            // The detailed classification the chain reaches, or none: a
            // coarse stated code is not a classification the market keeps.
            let cficode = self.classification().and_then(|held| Cfi::new(&held).ok());
            if cficode.is_none()
                && let Some(held) = self.stated_word(461).filter(|held| Cfi::new(held).is_err())
            {
                self.unread(
                    "FIX classification defaulted to none: the stated CFI code does not read",
                    461,
                    &held,
                );
            }
            debug_assert!(
                cficode
                    .as_ref()
                    .is_none_or(|held| Cfi::is_detailed(held.as_str())),
                "the classification chain answers a detailed code or none"
            );
            self.event.set_cficode(cficode, over(fact::CFI));
        }
        if reached(fact::MIC) {
            let miccode = self.stated_miccode();
            self.event.set_miccode(miccode, over(fact::MIC));
        }
        if reached(fact::TRADABLE) {
            let tradable = self.stated_tradable();
            self.event.set_tradable(tradable, over(fact::TRADABLE));
        }
        if reached(fact::UNIT) {
            let unit = self.stated_word(996).and_then(|held| {
                Unit::new(&held)
                    .inspect_err(|_| {
                        self.unread(
                            "FIX unit defaulted to none: the stated unit of measure does not read",
                            996,
                            &held,
                        );
                    })
                    .ok()
            });
            self.event
                .set_unit(unit.unwrap_or_else(Unit::none), over(fact::UNIT));
        }
        // The bid and the ask the message states, each in the currency a
        // field of its own names: the message's currency fills one that
        // names none, as the setters fill it.
        if reached(fact::BIDASK) {
            let (bidccy, askccy) = (
                self.stated_quote_ccy(&BID_CURRENCY),
                self.stated_quote_ccy(&ASK_CURRENCY),
            );
            let (bidpx, bidqty) = (
                self.stated_number(BIDPX_TAG),
                self.stated_number(BIDSIZE_TAG),
            );
            let (askpx, askqty) = (
                self.stated_number(OFFERPX_TAG),
                self.stated_number(OFFERSIZE_TAG),
            );
            let overwrite = over(fact::BIDASK);
            let event = &mut *self.event;
            event.set_bidccy(bidccy, overwrite);
            event.set_askccy(askccy, overwrite);
            event.set_bidpx(bidpx, overwrite);
            event.set_bidqty(bidqty, overwrite);
            event.set_askpx(askpx, overwrite);
            event.set_askqty(askqty, overwrite);
        }
        // The numbers, each from its own lifted slot.
        if reached(fact::PRICE) {
            let price = self.lifted.price();
            self.event.set_price(price, over(fact::PRICE));
        }
        if reached(fact::STOPPX) {
            let stoppx = self.stated_number(99);
            self.event.set_stoppx(stoppx, over(fact::STOPPX));
        }
        if reached(fact::QUANTITY) {
            // `Quantity(53)`; what an order still has open fills it from
            // `LeavesQty(151)` ([`Market`]), never what it ordered.
            let quantity = self.lifted.quantity();
            self.event.set_quantity(quantity, over(fact::QUANTITY));
        }
        if reached(fact::DISPLAYQTY) {
            // The peak it shows: `DisplayQty(1138)`, else the older
            // `MaxFloor(111)`.
            let displayqty = self.stated_number(1138).or_else(|| self.stated_number(111));
            self.event
                .set_displayqty(displayqty, over(fact::DISPLAYQTY));
        }
        if reached(fact::HIDDENQTY) {
            // What an iceberg keeps back: the quantity past the peak it
            // shows, where it shows less than all of it.
            let hiddenqty = self
                .event
                .get_quantity()
                .zip(self.event.get_displayqty())
                .and_then(|(total, shown)| total.checked_sub(shown))
                .filter(|held| held.is_positive());
            self.event.set_hiddenqty(hiddenqty, over(fact::HIDDENQTY));
        }
        if reached(fact::CXLQTY) {
            let cxlqty = self.stated_number(84);
            self.event.set_cxlqty(cxlqty, over(fact::CXLQTY));
        }
        if reached(fact::PREVPX) {
            let prevpx = self.stated_number(140);
            self.event.set_prevpx(prevpx, over(fact::PREVPX));
        }
        if reached(fact::FILLS) {
            let overwrite = over(fact::FILLS);
            let event = &mut *self.event;
            event.set_lastqty(self.lifted.lastqty(), overwrite);
            event.set_avgpx(self.lifted.avgpx(), overwrite);
            event.set_cumqty(self.lifted.cumqty(), overwrite);
            event.set_leavesqty(self.lifted.leavesqty(), overwrite);
        }
        if reached(fact::ORDQTY) {
            let ordqty = self.lifted.orderqty();
            self.event.set_ordqty(ordqty, over(fact::ORDQTY));
        }
        if reached(fact::LASTPX) {
            // The FX parts of the last price, each from its own lifted slot:
            // `LastSpotRate(194)` and `LastForwardPoints(195)`, the parts of
            // `LastPx(31)` - never of `Price(44)` - so one tag triple is one
            // statement, completed where two of its three are stated. A
            // sided quote states its side's where it states none of its own.
            let (mut lastpx, mut spotrate, mut forwardpoints) = (
                self.lifted.lastpx(),
                self.lifted.lastspotrate(),
                self.lifted.lastforwardpoints(),
            );
            fx_triple(&mut lastpx, &mut spotrate, &mut forwardpoints);
            if self.event.marketdatakind() == MarketDataKind::Quotation {
                match self.stated_side() {
                    Some(side) if side.is_bid() => {
                        spotrate = spotrate.or_else(|| self.lifted.bidspotrate());
                        forwardpoints = forwardpoints.or_else(|| self.lifted.bidforwardpoints());
                    }
                    Some(side) if side.is_ask() => {
                        spotrate = spotrate.or_else(|| self.lifted.offerspotrate());
                        forwardpoints = forwardpoints.or_else(|| self.lifted.offerforwardpoints());
                    }
                    _ => {}
                }
            }
            let overwrite = over(fact::LASTPX);
            let event = &mut *self.event;
            event.set_lastpx(lastpx, overwrite);
            event.set_spotrate(spotrate, overwrite);
            event.set_forwardpoints(forwardpoints, overwrite);
        }
        if reached(fact::CURRENCY) {
            let currency = self.stated_currency();
            self.event
                .set_currency(currency.unwrap_or_else(Ccy::none), over(fact::CURRENCY));
        }
        // The side last: what it quotes follows every value stated above.
        if reached(fact::SIDE) {
            let side = self.stated_side().unwrap_or(Side::Unknown);
            self.event.set_side(side, over(fact::SIDE));
        }
        // And what the order's quantities imply of one another, once every
        // one of them is stated: a none a field stated overwrote no fill.
        if reached(fact::ORDERED | fact::STATE) {
            self.event.settle_orders();
        }
    }

    /// Clears the facts `facts` names, each through its setter, overwriting
    /// with none: what [`Self::state_market`] states again from scratch, so
    /// a fill an earlier value made runs again off the new one.
    fn clear_market(&mut self, facts: u32) {
        let reached = |fact: u32| facts & fact != 0;
        let event = &mut *self.event;
        if reached(fact::MDTYPE) {
            event.set_marketdatatype(crate::MarketDataType::Unknown, true);
        }
        if reached(fact::TIF) {
            event.set_timeinforce(None, true);
        }
        if reached(fact::TICKER) {
            event.set_ticker(None, true);
        }
        if reached(fact::CFI) {
            event.set_cficode(None, true);
        }
        if reached(fact::MIC) {
            event.set_miccode(None, true);
        }
        if reached(fact::TRADABLE) {
            event.set_tradable(None, true);
        }
        if reached(fact::UNIT) {
            event.set_unit(Unit::none(), true);
        }
        if reached(fact::SIDE) {
            event.set_side(Side::Unknown, true);
        }
        if reached(fact::CURRENCY) {
            event.set_currency(Ccy::none(), true);
        }
        if reached(fact::BIDASK) {
            event.set_bidpx(None, true);
            event.set_bidqty(None, true);
            event.set_bidccy(None, true);
            event.set_askpx(None, true);
            event.set_askqty(None, true);
            event.set_askccy(None, true);
        }
        if reached(fact::PRICE) {
            event.set_price(None, true);
        }
        if reached(fact::STOPPX) {
            event.set_stoppx(None, true);
        }
        if reached(fact::QUANTITY) {
            event.set_quantity(None, true);
        }
        if reached(fact::DISPLAYQTY) {
            event.set_displayqty(None, true);
        }
        if reached(fact::HIDDENQTY) {
            event.set_hiddenqty(None, true);
        }
        if reached(fact::CXLQTY) {
            event.set_cxlqty(None, true);
        }
        if reached(fact::PREVPX) {
            event.set_prevpx(None, true);
        }
        if reached(fact::ORDQTY) {
            event.set_ordqty(None, true);
        }
        if reached(fact::FILLS) {
            event.set_lastqty(None, true);
            event.set_avgpx(None, true);
            event.set_cumqty(None, true);
            event.set_leavesqty(None, true);
        }
        if reached(fact::LASTPX) {
            event.set_lastpx(None, true);
            event.set_spotrate(None, true);
            event.set_forwardpoints(None, true);
        }
    }

    /// The security identifiers the message states, and the instrument a
    /// bridge's own key names filling what the fields left open, stated on
    /// the event with the derived overlay beside them.
    fn state_securityids(&mut self, overwrite: bool) {
        let (mut securityids, dropped) = self.stated_securityids(self.row_stated);
        // The instrument a bridge's own key names fills only what the fields
        // left open, and is never written back: a part its type refuses is
        // skipped.
        let keyed = instrument_key(&securityids)
            .and_then(|named| Some((named.isin?, named.src)))
            .filter(|_| !securityids.contains_kind(&IdType::Isin))
            .and_then(|(isin, src)| Identifier::new(src, IdType::Isin, isin.as_str()).ok());
        if let Some(isin) = keyed {
            securityids.insert(isin);
        }
        let _ = self.event.set_securityids(securityids, overwrite);
        // The derived overlay goes back as derived, so a stated identifier
        // still answers before it and removing the ISIN still takes it back.
        for id in self.derived.iter() {
            self.event.derive_securityid(id.kind(), id.value());
        }
        // What this reading dropped replaces what the last one did.
        self.anomalies.truncate(self.arrival_anomalies);
        self.anomalies.extend(dropped);
    }

    /// One field's value, where it states one that is not null.
    fn stated_by_tag(&self, tag: i32) -> Option<Scalar> {
        self.get_by_tag(tag).filter(|held| !held.is_null())
    }

    /// One field's text, trimmed; `None` where it is null or empty.
    fn stated_word(&self, tag: i32) -> Option<SmolStr> {
        stated_text(self.stated_by_tag(tag))
    }

    /// One field's exact decimal. A value stated but unreadable is none,
    /// beside a warning naming the field; an absent one is none silently.
    fn stated_number(&self, tag: i32) -> Option<Decimal> {
        self.stated_by_tag(tag).and_then(|held| {
            let read = Decimal::from_scalar(&held);
            if read.is_none() {
                self.unread(
                    "FIX price or quantity defaulted to none: the stated value is no exact decimal",
                    tag,
                    &held,
                );
            }
            read
        })
    }

    /// The side `Side(54)` states, where it states one that reads.
    fn stated_side(&self) -> Option<Side> {
        self.stated_word(54).and_then(|held| {
            Side::read(&held)
                .inspect_err(|_| {
                    self.unread(
                        "FIX side defaulted to UNKN: the stated side does not read",
                        54,
                        &held,
                    );
                })
                .ok()
        })
    }

    /// The currency one field states, where it names an ISO 4217 code other
    /// than none.
    fn stated_ccy(&self, tag: i32) -> Option<Ccy> {
        self.stated_word(tag)
            .and_then(|held| {
                Ccy::new(&held)
                    .inspect_err(|_| {
                        self.unread(
                            "FIX currency defaulted to none: the stated currency is no ISO 4217 code",
                            tag,
                            &held,
                        );
                    })
                    .ok()
            })
            .filter(|held| held.as_str() != Ccy::none().as_str())
    }

    /// The currency the message is priced in: `Currency(15)`, else
    /// `SettlCurrency(120)`, else the one the instrument a bridge's own key
    /// names.
    fn stated_currency(&self) -> Option<Ccy> {
        self.stated_ccy(15)
            .or_else(|| self.stated_ccy(120))
            .or_else(|| instrument_key(self.event.get_securityids()).and_then(|named| named.ccy))
    }

    /// The currency one of `names` states for a quote side, where one does.
    fn stated_quote_ccy(&self, names: &[&str]) -> Option<Ccy> {
        names.iter().find_map(|name| {
            let held = self
                .get_by_name(name)
                .or_else(|| self.named_value(name))
                .filter(|held| !held.is_null());
            stated_text(held)
                .and_then(|held| Ccy::new(&held).ok())
                .filter(|held| held.as_str() != Ccy::none().as_str())
        })
    }

    /// The market it last traded on, was routed to, the one the instrument
    /// key names, else the one it is listed on - each an ISO 10383 MIC or
    /// the Reuters mnemonic FIX 4.2 spelled it in, a code neither reading
    /// resolves naming none and passed over for the next - and ISO 10383's
    /// none for a currency pair, which trades on no one market.
    fn stated_miccode(&self) -> Option<Mic> {
        let market = |tag: i32| {
            self.stated_word(tag).and_then(|held| {
                let read = Mic::from_market(&held);
                if read.is_none() {
                    self.unread(
                        "FIX market passed over: the stated market names no MIC",
                        tag,
                        &held,
                    );
                }
                read
            })
        };
        let named = instrument_key(self.event.get_securityids());
        market(30)
            .or_else(|| market(100))
            .or_else(|| named.as_ref().and_then(|named| named.mic.clone()))
            .or_else(|| market(207))
            .or_else(|| {
                self.event
                    .get_securityids()
                    .get(&IdType::Forex)
                    .or_else(|| self.derived.get(&IdType::Forex))
                    .and_then(|code| Forex::new(code).ok())
                    .map(|_| Mic::none())
            })
    }

    /// Whether it could trade, from whichever status says so, in the codes
    /// FIX's own enumerations state. A status that is about something else -
    /// a code neither list names - says nothing either way, so the next
    /// status answers instead.
    fn stated_tradable(&self) -> Option<bool> {
        let status = |tag: i32, open: &[i64], shut: &[i64]| {
            let held = self.stated_by_tag(tag)?.as_i64()?;
            if open.contains(&held) {
                Some(true)
            } else if shut.contains(&held) {
                Some(false)
            } else {
                None
            }
        };
        status(326, &[3, 17], &[1, 2, 4, 18, 19, 21])
            .or_else(|| status(340, &[2], &[1, 3, 4, 5, 7]))
            .or_else(|| match self.stated_word(965)?.as_str() {
                "1" | "3" => Some(true),
                "2" | "4" | "5" | "6" | "9" | "11" => Some(false),
                _ => None,
            })
    }

    /// The type of its kind the message is: the first field its kind names
    /// that states a value - then any other field the registry maps - read
    /// through the registry ([`FixRegistry::marketdatatype_of`]).
    /// How long the message stands: `TimeInForce(59)` read through the
    /// dictionary's `FIX:timeinforce`, a value it maps to no member being
    /// `OTHER`, else the first other field the dictionary maps a stated
    /// value of.
    fn stated_timeinforce(&self) -> Option<TimeInForce> {
        let read = |tag: i32| {
            self.stated_by_tag(tag)
                .as_ref()
                .and_then(scalar_text)
                .and_then(|held| self.registry.timeinforce_of(tag, held.trim()))
        };
        read(identity::TIMEINFORCE_TAG).or_else(|| {
            self.registry
                .timeinforce_sources()
                .iter()
                .map(|(tag, _, _)| *tag)
                .filter(|tag| *tag != identity::TIMEINFORCE_TAG)
                .find_map(read)
        })
    }

    fn stated_marketdatatype(&self) -> Option<crate::MarketDataType> {
        // A typing field may be an `int` - `QuoteType(537)`, `TrdType(828)`
        // - so its value is read as the wire spells it.
        let read = |tag: i32| {
            self.stated_by_tag(tag)
                .as_ref()
                .and_then(scalar_text)
                .and_then(|held| self.registry.marketdatatype_of(tag, held.trim()))
        };
        let tags =
            crate::MarketDataType::fix_tags_of(self.header.msgtype(), self.event.marketdatakind());
        tags.iter().copied().find_map(read).or_else(|| {
            self.registry
                .marketdatatype_sources()
                .iter()
                .map(|(tag, _, _)| *tag)
                .filter(|tag| !tags.contains(tag))
                .find_map(read)
        })
    }

    /// Warns that `stated`, the value `tag` states, does not read as the
    /// market fact it fills: `what` names the fact and the default it took.
    /// The field is the key, so a stream warns once per field rather than
    /// once per message.
    fn unread(&self, what: &'static str, tag: i32, stated: &dyn fmt::Debug) {
        let field = self.registry.get_field_by_tag(tag).map_or_else(
            || format_smolstr!("{tag}"),
            |field| format_smolstr!("{}({tag})", field.name()),
        );
        crate::warning::warned!(
            what,
            &field,
            "{stated:?} on a {} message",
            self.header.msgtype()
        );
    }

    /// When the fields say the message executed. Execution time is not the
    /// message time: it is stated directly by the crate column, then by
    /// FIX's execution-specific timestamp, a regulatory execution member, a
    /// bridge's event timestamp, or the transaction time of an actual trade
    /// where it states a clock - a `TransactTime(60)` stating a day alone
    /// dates no execution, as it dates no event ([`Self::transact_unix`]).
    /// Corrections and cancels do not make their transaction clock an
    /// execution clock.
    fn stated_execution_instant(&self) -> Option<i64> {
        let by_tag = |tag: i32| self.get_by_tag(tag).filter(|held| !held.is_null());
        self.execution_instant(by_tag(2749))
            .or_else(|| self.trdreg_execution_instant())
            .or_else(|| {
                self.execution_instant(
                    self.get_by_name(EVENT_TIMESTAMP)
                        .or_else(|| self.named_value(EVENT_TIMESTAMP)),
                )
            })
            .or_else(|| {
                self.reports_execution()
                    .then(|| self.execution_instant(by_tag(60)))
                    .flatten()
                    .filter(|unix| unix.rem_euclid(NANOS_PER_DAY) != 0)
            })
    }

    /// The execution instant a message states, else the one its place gives
    /// it. A clock the fields state is the execution's. Where they state
    /// none, a raw execution report executed when it happened, so intake
    /// dates it rather than leaving that to a walk; a message a walk placed
    /// keeps the latest execution its chain reached, which is the walk's to
    /// state - a walk that carried the same instant states nothing new - and
    /// its state may be one it inherited, which read as its own report would
    /// date an execution it never made.
    fn execution_or_placed(&self, stated: Option<i64>) -> Option<i64> {
        stated.or_else(|| {
            if self.event.get_prevuuid().is_some() {
                self.event.get_execunix()
            } else {
                self.reports_execution().then(|| self.event.get_currunix())
            }
        })
    }

    /// What a settle moves when the clock alone moved: the execution instant
    /// of a report nothing else dates, which is its own instant, and the
    /// identity the instant derives. Everything else a settle reads - the
    /// fields, the row's word, what a walk set - stood still, so the market,
    /// the maps and the code it would derive again are the ones the message
    /// holds.
    pub(super) fn settle_clock(&mut self) {
        if self.row_stated & ROW_STATED_EXECUTION == 0 {
            self.state_market(fact::EXECUTION);
        }
        let code = self.event.get_currhashcode();
        self.event.finalized(code);
    }

    /// The security identifiers the message states, in rank order: the
    /// primary `SecurityID(48)` under its `SecurityIDSource(22)`, then each
    /// `secaltids` occurrence, each source read through
    /// [`IdType::from_security_source`] - a source it cannot read an
    /// anomaly, the field kept on the wire -
    /// then the crated `isincode`, `bloombergcode`, `figicode` and
    /// `forexcode` views the row stated (`row_stated`), then each unmapped
    /// field whose name names a source. Each code is validated and the first stated code under a
    /// key kept.
    fn stated_securityids(&self, row_stated: u64) -> (Identifiers, Vec<super::FixAnomaly>) {
        let mut ids = Identifiers::new();
        let mut anomalies = Vec::new();
        // Fill only: the same code twice under one type and source is one
        // entry, and a later different one is dropped with an anomaly,
        // staying on the wire as it arrived.
        let mut insert = |field: &str, made: crate::Result<Identifier>| match made {
            Ok(id) => match ids.get_from(id.src(), id.kind()) {
                Some(held) if held != id.value() => anomalies.push(super::FixAnomaly::new(
                    field,
                    format!(
                        "states {id} where {}:{}={held} is already stated",
                        id.src(),
                        id.kind()
                    ),
                )),
                Some(_) => {}
                None => {
                    ids.insert(id);
                }
            },
            Err(error) => anomalies.push(super::FixAnomaly::new(field, error.to_string())),
        };
        let mut state = |field: &str, source: Option<SmolStr>, code: Option<SmolStr>| {
            if let (Some(source), Some(code)) = (source, code) {
                insert(field, security_identifier(&source, IdSource::Fix, &code));
            }
        };
        state(
            "securityid",
            self.get_by_tag(22).as_ref().and_then(scalar_text),
            self.get_by_tag(48).as_ref().and_then(scalar_text),
        );
        for [source, code] in self.group_rows("secaltids", ["securityaltidsource", "securityaltid"])
        {
            state("secaltids", source, code);
        }
        // A crated column's row-stated entry is the crate's own statement,
        // ranked after the wire's and before a name a bridge happened to
        // spell.
        for (bit, kind, name) in [
            (ROW_STATED_ISIN, IdType::Isin, super::ISINCODE_TAG_NAME.1),
            (
                ROW_STATED_BLOOMBERG,
                IdType::Bloomberg,
                super::BLOOMBERGCODE_TAG_NAME.1,
            ),
            (ROW_STATED_FIGI, IdType::Figi, super::FIGICODE_TAG_NAME.1),
            (ROW_STATED_FOREX, IdType::Forex, super::FOREXCODE_TAG_NAME.1),
        ] {
            if row_stated & bit != 0
                && let Some(id) = self.event.get_securityids().get_identifier(&kind)
            {
                insert(name, Ok(id.clone()));
            }
        }
        // An entry no dictionary maps whose key names a security
        // identifier - `#ISINCODE`, `cusip_code`, a bridge's
        // `OMS_InstrumentID` - states one after the wire's own: trimmed,
        // validated, never a grouped member, and left on the wire as it
        // arrived. A value its type refuses is an anomaly here, whichever
        // set its key names.
        let declared = self.declared_identifiers();
        self.for_each_unmapped(|key, text| {
            if let Some(made) = inferred_identifier(key, text, declared, false)
                .filter(|made| made.as_ref().ok().is_none_or(|id| id.kind().is_security()))
            {
                insert(key, made);
            }
        });
        (ids, anomalies)
    }

    /// The identifier names this message's type declares under
    /// `FIX:identifiers`, comma-separated.
    fn declared_identifiers(&self) -> Option<&str> {
        self.registry
            .get_msgtype(self.header.msgtype())
            .and_then(|definition| declared_identifiers(definition.as_field()))
    }

    /// Each entry this message states that no dictionary maps, as its key
    /// and its text: every `metadata` key, then every top-level scalar child
    /// no tag maps that states a value. Which children a tag maps is marked
    /// once, in a word per 64 children held inline, so neither a set nor a
    /// scan of the tag index is paid per child.
    fn for_each_unmapped(&self, mut visit: impl FnMut(&str, &str)) {
        for (key, value) in &self.metadata {
            visit(key, value);
        }
        let Some(cells) = self.value.as_sequence() else {
            return;
        };
        let mut mapped: SmallVec<[u64; 4]> = SmallVec::from_elem(0, cells.len().div_ceil(64));
        for (_, at) in &self.tags {
            if let Some(word) = mapped.get_mut(at / 64) {
                *word |= 1 << (at % 64);
            }
        }
        for (at, (child, cell)) in self.field.fields().iter().zip(cells).enumerate() {
            if mapped[at / 64] & (1 << (at % 64)) != 0
                || cell.is_null()
                || child.dtype().is_nested()
            {
                continue;
            }
            // A text cell is visited where it lies; only a number is
            // spelled, inline.
            match cell.as_str().map(str::trim) {
                Some("") => {}
                Some(text) => visit(child.name(), text),
                None => {
                    if let Some(text) = scalar_text(cell) {
                        visit(child.name(), &text);
                    }
                }
            }
        }
    }

    /// The stated members of each occurrence of a root repeating group, in
    /// occurrence order, the members' positions resolved once; nothing where
    /// the message carries no such group.
    fn group_rows<const N: usize>(
        &self,
        group: &str,
        members: [&str; N],
    ) -> SmallVec<[Occurrence<N>; 8]> {
        let mut held = SmallVec::new();
        let at = self.field.index_of(group);
        let group = at.and_then(|at| {
            let serie = self.field.fields().get(at)?.dtype().as_serie_type()?;
            let value = self.value.as_sequence()?.get(at)?;
            Some((members.map(|name| serie.item().index_of(name)), value))
        });
        if let Some((positions, value)) = group {
            read_occurrences(value, positions, &mut held);
        }
        held
    }

    /// The values `tags` state in every occurrence of the group `counter`
    /// counts, group and members found by tag: a trade side's own
    /// occurrences first - the more specific statement, which an execution
    /// a trade split off is about - then the message's.
    fn tagged_occurrences<const N: usize>(
        &self,
        counter: i32,
        tags: [i32; N],
    ) -> SmallVec<[Occurrence<N>; 8]> {
        let registry = &*self.registry;
        let positions = |group: &Field| {
            let serie = group.dtype().as_serie_type()?;
            Some(tags.map(|member| child_by_tag(registry, serie.item().fields(), false, member)))
        };
        let mut held = SmallVec::new();
        self.for_each_side(|fields, cells| {
            if let Some(inner) = child_by_tag(registry, fields, true, counter)
                && let (Some(group), Some(value)) = (fields.get(inner), cells.get(inner))
                && let Some(members) = positions(group)
            {
                read_occurrences(value, members, &mut held);
            }
        });
        if let Some((group, value)) = self.root_group(counter)
            && let Some(members) = positions(group)
        {
            read_occurrences(value, members, &mut held);
        }
        held
    }

    /// The field and the value of the root group `counter` counts, where
    /// exactly one root child is that group.
    fn root_group(&self, counter: i32) -> Option<(&Field, &Scalar)> {
        let at = self.index_of_group(counter)?;
        Some((
            self.field.fields().get(at)?,
            self.value.as_sequence()?.get(at)?,
        ))
    }

    /// Visits every occurrence of a trade's or a cross's `NoSides(552)`, in
    /// order, as its item's fields beside the occurrence's cells: an
    /// execution a trade's parse split off holds its one side.
    fn for_each_side(&self, mut visit: impl FnMut(&[Field], &[Scalar])) {
        let Some((group, value)) = self.root_group(SIDES) else {
            return;
        };
        let (Some(serie), Some(rows)) = (group.dtype().as_serie_type(), value.as_serie()) else {
            return;
        };
        for occurrence in rows.iter() {
            if let Some(cells) = occurrence.as_sequence() {
                visit(serie.item().fields(), cells);
            }
        }
    }

    /// Rebuilds the alternate identifiers from the fields that state them,
    /// at every settle: each of the registry's
    /// [`idmap_sources`](FixRegistry::idmap_sources) in turn, a field's own
    /// value or the `PartyID(448)` of every `Parties` occurrence under the
    /// entry's role, then every regulatory trade identifier - and the
    /// accounts: one per `Parties` or `RootParties` role and the
    /// `Account(1)`, a trade side's before the message's. The first value
    /// an alternate identifier's key is stated with fills it and a later
    /// different one is kept as an anomaly; a second party of a role is
    /// ordinary and states nothing.
    fn rebuild_idmaps(&mut self) {
        let mut identifiers = Identifiers::new();
        let mut dropped = Vec::new();
        let registry = Arc::clone(&self.registry);
        for (tag, source) in registry.idmap_sources() {
            let field = if source.role().is_some() {
                PARTIES
            } else {
                registry.get_field_by_tag(*tag).map_or("", Field::name)
            };
            match source.role() {
                None => {
                    if let Some(value) = self.get_by_tag(*tag).as_ref().and_then(scalar_text) {
                        admit_identifier(
                            &mut identifiers,
                            IdSource::Fix,
                            Ok(source.key().clone()),
                            &value,
                            field,
                            &mut dropped,
                        );
                    }
                }
                Some(role) => {
                    for [partyrole, partyid] in self.group_rows(PARTIES, [PARTYROLE, PARTYID]) {
                        if let Some(value) = partyid.filter(|_| partyrole.as_deref() == Some(role))
                        {
                            admit_identifier(
                                &mut identifiers,
                                IdSource::Fix,
                                Ok(source.key().clone()),
                                &value,
                                field,
                                &mut dropped,
                            );
                        }
                    }
                }
            }
        }
        for (counter, id, kind) in REGULATORY_GROUPS {
            for [value, kind] in self.tagged_occurrences(counter, [id, kind]) {
                if let Some(value) = value {
                    admit_identifier(
                        &mut identifiers,
                        IdSource::Fix,
                        regulatory_kind(kind.as_deref()),
                        &value,
                        "regulatorytradeids",
                        &mut dropped,
                    );
                }
            }
        }
        // A side's parties first - the more specific statement, which an
        // execution a trade split off is about - then the message's.
        let mut partyids = Identifiers::new();
        let codes = PartyCodes::new(&registry);
        self.for_each_side(|fields, cells| {
            AccountsAt::new(&registry, fields).read(codes, cells, &mut partyids);
        });
        let groups = PARTY_GROUPS.map(|(counter, ..)| {
            let (field, value) = self.root_group(counter)?;
            Some((value, party_positions(&registry, field, counter)?))
        });
        let account = [ACCOUNT, ACCOUNT_SOURCE].map(|tag| {
            self.unique_index_of_tag(tag)
                .and_then(|at| self.value.as_sequence()?.get(at))
        });
        read_parties(codes, groups, account, &mut partyids);
        // An entry no dictionary maps whose key names an operation's or a
        // party's identifier - a bridge's `firm.x.ParentOrderID`,
        // `OMS_UserID` - states one after the fields' own; a value its type
        // refuses stays on the wire, unread.
        let declared = self.declared_identifiers();
        self.for_each_unmapped(|key, text| {
            let Some(Ok(id)) = inferred_identifier(key, text, declared, false) else {
                return;
            };
            if id.kind().is_party() {
                partyids.insert(id);
            } else if !id.kind().is_security() {
                identifiers.insert(id);
            }
        });
        self.anomalies.extend(dropped);
        // What a caller or a row stated is its word, which no settle
        // restates: each value it holds stands, lineage included, and an
        // identifier the fields state that it lacks - a content merge's
        // `ClOrdID(11)` - fills it.
        // The held set is copied only where the fields name one it lacks.
        let lacks = |held: &Identifiers, stated: &Identifiers| {
            stated
                .iter()
                .any(|id| held.get_from(id.src(), id.kind()).is_none())
        };
        if self.stated & fact::IDENTIFIERS == 0 {
            let _ = self.event.set_identifiers(identifiers, true);
        } else if lacks(self.event.get_identifiers(), &identifiers) {
            let mut held = self.event.get_identifiers().clone();
            held.merge(&identifiers);
            let _ = self.event.set_identifiers(held, true);
        }
        if self.stated & fact::PARTYIDS == 0 {
            let _ = self.event.set_partyids(partyids, true);
        } else if lacks(self.event.get_partyids(), &partyids) {
            let mut held = self.event.get_partyids().clone();
            held.merge(&partyids);
            let _ = self.event.set_partyids(held, true);
        }
    }

    /// What this message states that no typed column of its leaves reads,
    /// keyed as a leaf's metadata keys it: one walk of the row.
    ///
    /// The message's own map opens with the bridge's namespaced keys as the
    /// metadata holds them, then every root child no typed fact reads,
    /// under its name: a scalar as the canonical text it spells, a group,
    /// a component or a map as one JSON text under its folded name -
    /// `miscfees` holds `[{"miscfeeamt":"1.5","miscfeecurr":"EUR"}]` - a
    /// group an array of one object per occurrence, a component or a map one
    /// object, nested groups and components recursing, every leaf inside the
    /// canonical text a root scalar spells, so a decimal keeps its stored
    /// scale and no value is a JSON number. Null, skipped and counter
    /// members are left out, and a child left with nothing writes no key.
    /// Left out too are a child the [envelope](super::digest) holds, since
    /// the message's code leaves the same set out; a typed tag stated twice;
    /// a tag [`identity::MARKET_TAGS`] reads; an identifier map's source;
    /// and a child a read reaches by its name - the execution clock a
    /// bridge states, a detailed classification, a bid's or an ask's
    /// currency, an identifier source's column.
    ///
    /// Where `held`, what the leaf's identifier maps hold leaves the map too:
    /// each `Parties(453)` or `RootParties(1116)` occurrence whose party its
    /// accounts hold under its role - or an identifier map reads under its
    /// role - each regulatory trade identifier its alternate identifiers
    /// hold under its type, and the `Account(1)` its accounts hold. What a
    /// map does not hold stays: a second party of one role, a value no map
    /// takes. An empty snapshot's control holds no map, so nothing is
    /// held for it. A scalar whose key names an identifier - one the
    /// message's type declares under `FIX:identifiers` at its end, an
    /// execution report's `RefOrderID(1080)`, or, for a key no dictionary
    /// field is, one of the crate's identifier names, a bridge's
    /// `firm.x.ParentOrderID` - is set aside, for the leaf to lift into the
    /// identifier set its type belongs to under the source the rest of its
    /// key names; an occurrence's member is read against what its
    /// component declares.
    ///
    /// Where `expanded` names a group the message becomes one leaf per
    /// occurrence of, that group is no child of the message's own: each
    /// occurrence answers its own members instead, keyed bare - a nested
    /// member as its JSON text - less the tags `expanded` says its leaf
    /// reads and what its leaf holds, the occurrence's own parties and
    /// account leading the message's. A child's tag and a group's member
    /// plan are each resolved once per walk, never per occurrence.
    pub(super) fn unmapped(&self, expanded: Option<&Expanded>, held: bool) -> Unmapped {
        let nothing = Identifiers::new();
        let holds = Holds {
            registry: &self.registry,
            codes: PartyCodes::new(&self.registry),
            partyids: if held {
                self.event.get_partyids()
            } else {
                &nothing
            },
            identifiers: if held {
                self.event.get_identifiers()
            } else {
                &nothing
            },
        };
        let mut unmapped = Unmapped {
            message: Metadata::new(),
            lifted: Vec::new(),
            occurrences: Vec::new(),
        };
        let identifiers = self
            .registry
            .get_msgtype(self.header.msgtype())
            .and_then(|definition| declared_identifiers(definition.as_field()));
        for (key, value) in &self.metadata {
            let (message, lifted) = (&mut unmapped.message, &mut unmapped.lifted);
            holds.land(key, value.clone(), identifiers, false, message, lifted);
        }
        let fields = self.field.fields();
        let Some(cells) = self.value.as_sequence() else {
            return unmapped;
        };
        // Each child's tag and counter by position, off the indexes the
        // message already holds.
        let mut resolved: SmallVec<[(Option<i32>, Option<i32>); 32]> =
            SmallVec::from_elem((None, None), fields.len());
        for (tag, at) in &self.tags {
            if let Some(slot) = resolved.get_mut(*at) {
                slot.0 = Some(*tag);
            }
        }
        for (counter, at) in &self.groups {
            if let Some(slot) = resolved.get_mut(*at) {
                slot.1 = Some(*counter);
            }
        }
        let sources = self.registry.idmap_sources();
        let inherited = expanded.map_or(&[][..], |expanded| expanded.inherited);
        let read = |tag: i32| {
            super::digest::is_envelope(tag)
                || identity::is_typed_tag(tag)
                || identity::MARKET_TAGS.contains(&tag)
                || inherited.contains(&tag)
                || sources
                    .iter()
                    .any(|(held, source)| *held == tag && source.role().is_none())
        };
        let skipped = |tag: i32| super::digest::is_envelope(tag) || inherited.contains(&tag);
        // The children a read reaches by name rather than by tag.
        let named: SmallVec<[usize; 6]> =
            std::iter::once(self.child_index(&self.field, EVENT_TIMESTAMP))
                .chain(
                    super::cfi::DETAILED_NAMES
                        .iter()
                        .map(|name| self.index_of_name(name)),
                )
                .chain(
                    BID_CURRENCY
                        .iter()
                        .chain(&ASK_CURRENCY)
                        .map(|name| self.child_index(&self.field, name)),
                )
                .flatten()
                .collect();
        for (at, ((child, cell), (tag, counter))) in
            fields.iter().zip(cells).zip(&resolved).enumerate()
        {
            if cell.is_null() {
                continue;
            }
            let nested = child.dtype().is_nested();
            // A counter beside the group it counts states nothing the group
            // does not.
            if !nested
                && tag.is_some_and(|tag| {
                    self.groups
                        .binary_search_by_key(&tag, |(counter, _)| *counter)
                        .is_ok()
                })
            {
                continue;
            }
            if let Some(expanded) = expanded.filter(|expanded| *counter == Some(expanded.counter)) {
                unmapped.occurrences = self.own_occurrences(child, cell, expanded, &holds);
                continue;
            }
            if tag.is_some_and(read) || counter.is_some_and(read) || named.contains(&at) {
                continue;
            }
            if !nested {
                if let Some(text) = spelled(cell)
                    && (*tag != Some(ACCOUNT) || !holds.holds_account(None, &text))
                {
                    let (message, lifted) = (&mut unmapped.message, &mut unmapped.lifted);
                    let tagged = tag.is_some();
                    holds.land(child.name(), text, identifiers, tagged, message, lifted);
                }
                continue;
            }
            let walk = Walk {
                registry: &self.registry,
                skipped: &skipped,
            };
            let planned = Planned {
                tag: *tag,
                counter: *counter,
                ..Planned::new(child)
            };
            if let Some(text) = holds.render(&walk, &planned, None, cell) {
                unmapped.message.insert(SmolStr::new(child.name()), text);
            }
        }
        unmapped
    }

    /// Each occurrence of the group `expanded` names, in order: its own
    /// members keyed bare - a scalar as its text, a nested member as its
    /// JSON text - less the tags its leaf reads and what its leaf's
    /// identifier maps hold, the occurrence's own parties and account
    /// leading the message's; a scalar whose key ends with an identifier
    /// name is set aside to be lifted.
    fn own_occurrences(
        &self,
        group: &Field,
        cell: &Scalar,
        expanded: &Expanded,
        holds: &Holds<'_>,
    ) -> Vec<(Metadata, Lifted)> {
        let skipped = |tag: i32| super::digest::is_envelope(tag) || expanded.reads.contains(&tag);
        let walk = Walk {
            registry: &self.registry,
            skipped: &skipped,
        };
        let (DataType::Serie(item) | DataType::LargeSerie(item)) = group.dtype() else {
            return Vec::new();
        };
        let Some(rows) = cell.as_serie() else {
            return Vec::new();
        };
        let fields = match item.dtype() {
            DataType::Struct(_) => item.fields(),
            _ => &[],
        };
        let members = walk.members(fields);
        let identifiers = declared_identifiers(item);
        // Where an occurrence states its own parties and account, planned
        // once for every occurrence.
        let accounts_at = AccountsAt::new(&self.registry, fields);
        rows.iter()
            .map(|occurrence| {
                let mut own = (Metadata::new(), Lifted::new());
                if let Some(cells) = occurrence.as_sequence() {
                    let mut accounts = Identifiers::new();
                    accounts_at.read(holds.codes, cells, &mut accounts);
                    let level = Level {
                        holds,
                        own: &accounts,
                        identifiers,
                    };
                    land_members(&walk, &members, cells, &level, &mut own);
                }
                own
            })
            .collect()
    }

    /// The code the message digests to: the event's own facts - its parents, its state and its
    /// predecessor - then every field the message states but the standard
    /// header and trailer - the text, the metadata, the FIX fields it
    /// lifted and every row child that holds a value, by name - and never
    /// the capture, because where a line was read from is a fact about the
    /// capture and not about the message.
    ///
    /// The standard header and trailer are the frame's, less the one tag in
    /// them that says what the message *is*: which session carried the
    /// message, its place in that session, when it was sent and how it was
    /// checked are the frame's, and `MsgType(35)` is not - an order and a
    /// report carrying the same tags are not one message, the exception
    /// [the wire digest](super::digest) states too. A capture logs one
    /// message at every hop it passes and each hop frames it in a session
    /// of its own, so a code over the rest of the frame would make one
    /// message as many messages as hops. Residual header or trailer entries
    /// use the wire digest's same envelope predicate, so storing an
    /// unlifted `OrigSendingTime(122)`, `PossResend(97)`, or `BodyLength(9)`
    /// cannot change this canonical code. The cross code names a chain and
    /// stays outside the content digest too. The derived `msgsesseventid`
    /// records the complete delivery provenance on the capture, and is
    /// excluded - under its former identifier name too - for the same reason
    /// as the frame and capture fields it combines.
    ///
    /// The market is not here either, except for its operation ID: other
    /// market facts derive from FIX fields the content already digests, while
    /// MsgCat is also a generic fact callers may state or mutate directly.
    /// Feeding that ID once makes the derived and serialized readings agree
    /// and makes a category mutation move the generic event identity.
    fn currhashcode(&self) -> u64 {
        let mut state = crate::xxhash::Xxh3::new();
        crate::graph::element::feed_event_facts(&mut state, &*self.event);
        // Inline: text, msgtype, msgcat and the lifted facts are 26 cells,
        // leaving six metadata keys before a message spills to the heap.
        let mut cells: SmallVec<[(SmolStr, Scalar); 32]> = SmallVec::new();
        if let Some(text) = self.text.as_deref() {
            cells.push((SmolStr::new_static("text"), Scalar::from(text)));
        }
        // The names are the dictionary's, read off it once per registry
        // rather than once per message.
        let names = self.registry.lifted_names();
        if !self.header.msgtype().is_empty() {
            cells.push((names.msgtype.clone(), Scalar::from(self.header.msgtype())));
        }
        for (key, value) in &self.metadata {
            cells.push((key.clone(), Scalar::from(value.clone())));
        }
        // The fields the message lifted out of its row, under the names the
        // row's children are digested by.
        for (tag, name) in identity::LIFTED_TAGS.into_iter().zip(&names.lifted) {
            let Some(fact) = self.typed_fact(tag) else {
                continue;
            };
            cells.push((name.clone(), fact));
        }
        // The code, the `int32` it always fed: typing the column moved no
        // message's identity.
        cells.push((
            SmolStr::new_static("msgcat"),
            Scalar::from(self.msgcat().code()),
        ));
        cells.sort_by(|left, right| left.0.cmp(&right.0));
        xxhash::write_named_bytes(
            &mut state,
            cells.iter().map(|(name, value)| (name.as_str(), value)),
            0,
        );
        // Then the content, as the entries state it rather than as the row
        // stores it. Two readings of one message lay its children out
        // differently - a group one reading declares whole and another
        // states member by member is one group, and a child stating null or
        // a list holding nothing beside no stated count says nothing at all
        // - so a code taken off the row's storage would make a message read
        // back out of a row a different message. The entries are what the
        // message says, and they are what this feeds.
        feed_entries(&mut state, self.entries());
        state.as_u64()
    }

    /// Records the message as recorded when it was sent, where its carrier
    /// stated no recording and the message states its `SendingTime(52)`:
    /// the one recording the message itself states is its sender's. Read
    /// once, where the message is built from a row with no `recdunix`
    /// column - a row that has one states its own, a null included.
    fn record_at_sending(&mut self) {
        if self.event.get_recdunix().is_none() && self.header.stated_sendingtime() {
            self.event.set_recdunix(Some(self.header.sendingtime()));
        }
    }

    /// The message dated by the transaction it states, where the parse
    /// dated it by a stand-in: a message whose `SendingTime(52)` was
    /// supplied rather than stated - a carrier's, the codec's default, the
    /// intake's own clock - takes `TransactTime(60)` as its instant where it
    /// states one with a clock, and a resend's `OrigSendingTime(122)` as its
    /// creation where that is earlier, its stand-in sending clock moved to
    /// the same instant, and is enriched again around them - filled and
    /// settled - exactly as a parse dated there would have built it: the row
    /// the parse restated is the row a parse dated there restates, because a
    /// rule reads the row and never the clock. A stated sending clock stands:
    /// the parse dated the message by what it said, and the walk does not
    /// second-guess it. No delay bounds this one: a clock nobody stated is
    /// no reference to measure a distance from, which is why the parse's own
    /// [`FixCodec::official_time_delay_ms`](super::FixCodec::official_time_delay_ms)
    /// reading of the transaction ends where this one begins. What the
    /// [lifecycle](super::FixCodec::lifecycle) reads off the structured
    /// message before it walks, so a capture whose frames state no sending
    /// clock still orders, expires and folds by when its transactions
    /// happened rather than by when it was read. The enriching pass never
    /// refuses the message: a step it cannot write is warned about and the
    /// message keeps what it stated.
    #[must_use]
    pub fn dated_by_transaction(mut self) -> Self {
        if !self.redate_by_transaction() {
            return self;
        }
        let registry = Arc::clone(&self.registry);
        super::enrich::redated(&registry, self)
    }

    /// Moves the message's clock to its transaction where the sending clock
    /// was supplied rather than read, answering whether it did: the instant,
    /// the stand-in sending clock, and the creation it began at - nothing a
    /// field states, so nothing any reading of the fields answers anew.
    pub(super) fn redate_by_transaction(&mut self) -> bool {
        if self.header.stated_sendingtime() {
            return false;
        }
        let Some(unix) = self.transact_unix() else {
            return false;
        };
        let current = self.get_currunix();
        let created = self.get_creaunix();
        self.event.set_currunix(unix);
        // The stand-in sending clock follows: it was never a fact of the
        // message, and a row read back states the instant as the clock.
        self.header.set_sendingtime(unix);
        if created.is_none_or(|held| held == current) {
            self.event.set_creaunix(Some(unix));
        }
        let origin = match self.indexed_by_tag(122) {
            Some(Scalar::DateTime64(origin)) => {
                Scalar::DateTime64(origin).temporal_count_at(crate::TimeUnit::Nanosecond)
            }
            _ => None,
        };
        if let Some(origin) = origin.filter(|origin| *origin < unix) {
            self.event.set_creaunix(Some(origin));
        }
        true
    }

    /// The event this message is: every fact the three graph traits answer,
    /// held as fields.
    #[must_use]
    pub(crate) const fn event(&self) -> &OperationEventFacts {
        &self.event
    }

    /// The standard header and trailer, typed.
    #[must_use]
    pub const fn header(&self) -> &FixHeader {
        &self.header
    }

    /// The business category the message's type files under: the
    /// dictionary's `FIX:msgcat` for its `MsgType(35)`, [`MarketDataKind::Unknown`]
    /// where it files none, and a type filed under `EXEC` - an execution
    /// report - that reports no fill its order's report,
    /// [`MarketDataKind::Order`], or its quote's,
    /// [`MarketDataKind::Quotation`], where it names a `QuoteID(117)`; a
    /// row stating one is the row's word.
    #[must_use]
    pub fn msgcat(&self) -> MarketDataKind {
        self.event.marketdatakind()
    }

    /// The option strike price the message identifies: `StrikePrice(202)`
    /// as the decimal leaf, `None` where it states none or states no
    /// decimal.
    #[must_use]
    pub fn strikeprice(&self) -> Option<Decimal> {
        self.get_by_tag(STRIKEPRICE_TAG)
            .filter(|held| !held.is_null())
            .as_ref()
            .and_then(Decimal::from_scalar)
    }

    /// The FIX fields the message lifted out of its row, exactly as it
    /// stated them: the prices, the quantities and the identifiers.
    ///
    /// What the message *implies* about its market is the
    /// [`Market`](crate::graph::Market) getters' to answer -
    /// `get_price` reads this ladder and the row - and a fact answered there
    /// but absent here is derived, which is why it reaches neither the wire
    /// nor the code the message digests to.
    #[must_use]
    pub const fn lifted(&self) -> &FixLifted {
        &self.lifted
    }

    /// `Text(58)`: the free text the message carries, where it carries one.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// What a bridge stated under its own namespaces - a `TECH.CLIENTID`, an
    /// `AMON.…` key - each under the key as the bridge spelled it, folded,
    /// in sorted order; empty where it stated none.
    #[must_use]
    pub const fn metadata(&self) -> &BTreeMap<SmolStr, SmolStr> {
        &self.metadata
    }

    /// What the line said about the capture it was written for, typed: the
    /// plugin a bridge logged it under, the message context and the session
    /// instance, all read off the line's own bytes, and the session event
    /// the last two join to with the message's type and sequence.
    ///
    /// Not where the line was read from and not when it was recorded: those
    /// are the reader's statements, and they are [the capture's own
    /// columns](Self::from_row) rather than facts of a message.
    #[must_use]
    pub const fn capture(&self) -> &FixCapture {
        &self.capture
    }

    /// The capture's own cells: what the row this message was read from
    /// said for itself, each under the column's name.
    ///
    /// Where the line was read from, its place in the object, the body it
    /// was cut from, when it was recorded, what a bound dropped: the columns
    /// a reader stated beside the payload, the carried ones and the one the
    /// crate tags, `sourceurl`. Provenance and never content: none of them
    /// is an entry, none reaches the wire or the code the message answers
    /// to - the same message read from a second copy of one day's log is
    /// the same message - and [`Self::into_row`] states each again at the
    /// column of its name, which is how a row read back through
    /// [`Self::from_row`] and written again keeps what it said for itself.
    /// A message parsed from bytes carries none; one parsed out of a row
    /// carries that row's, and one read back out of a row carries the
    /// row's.
    #[must_use]
    pub fn carried(&self) -> &[(SmolStr, Scalar)] {
        &self.carried
    }

    /// States the capture's own cells, replaced whole.
    pub fn set_carried(&mut self, cells: Vec<(SmolStr, Scalar)>) {
        self.carried = cells;
    }

    /// The cell carried under `name`, folded as a column is named, else
    /// null.
    pub(super) fn carried_cell(&self, name: &str) -> Scalar {
        self.carried
            .iter()
            .find(|(held, _)| crate::folds_equal(held, name))
            .map_or(Scalar::Null, |(_, value)| value.clone())
    }

    /// The row read as a tree: one entry per child it states, a group's
    /// occurrences and a component's members nested under the entry that
    /// heads them; nothing for a child stating null, nor for a list holding
    /// nothing that no counter beside it states a count for - a group of no
    /// occurrence is stated by its count.
    ///
    /// Derived on the first ask and kept until a write, so a consumer
    /// walking the message twice pays once and a stream that never asks
    /// pays nothing. The typed facts are not entries: the header, the event
    /// and the capture are the holders' to answer.
    #[must_use]
    pub fn entries(&self) -> &[FixEntry] {
        self.entries.get_or_init(|| self.derive_entries().into())
    }

    /// The row read as a tree.
    ///
    /// Every child of the row is content, a key no dictionary explains
    /// included, because the row holds nothing else: the typed facts are
    /// their holders' to answer, and the capture's own columns - the body a
    /// line was read from, its place in the object, the object itself, the
    /// instant it was recorded - never reach a message at all, so there is
    /// nothing here to tell apart from what the line said.
    fn derive_entries(&self) -> Vec<FixEntry> {
        let Some(values) = self.value.as_sequence() else {
            return Vec::new();
        };
        entries_of(&self.registry, self.field.fields(), values)
    }

    /// The entries the wire carries around the row, in wire order: the
    /// standard header and then the FIX fields the message lifted in front
    /// of it, the standard trailer behind it. The row's own entries stand
    /// between the two exactly as [`Self::entries`] holds them, so a digest
    /// and a re-emission read them where they are rather than through a
    /// copy of the whole tree.
    ///
    /// The frame's own bands are the two the wire moves out of tag order,
    /// which is why the header leads and the trailer closes whatever the
    /// body's tags are. Between them stands only what the message *stated*:
    /// a fact the event derived - the price it is about, the state it
    /// reached - is answered by the traits and
    /// emitted nowhere, because a re-emission says what was read.
    fn wire_bands(&self) -> (Vec<FixEntry>, usize) {
        let name_of = |tag: i32| {
            self.registry.get_field_by_tag(tag).map_or_else(
                || format_smolstr!("{tag}"),
                |field| SmolStr::new(field.name()),
            )
        };
        let emit = |entries: &mut Vec<FixEntry>, tag: i32| {
            if tag == 52 && !self.header.stated_sendingtime() {
                return;
            }
            let Some(fact) = self.typed_fact(tag) else {
                return;
            };
            let text = match self.registry.get_field_by_tag(tag) {
                Some(field) => wire_text_under(&self.registry, field, &fact),
                None => wire_text(&fact),
            };
            if let Some(text) = text {
                entries.push(FixEntry::new(tag, name_of(tag), Some(text)));
            }
        };
        // One vector holds both bands, the head first; the split is where
        // the row's own entries stand between them.
        let mut bands = Vec::with_capacity(
            identity::WIRE_HEADER_TAGS.len()
                + identity::LIFTED_TAGS.len()
                + identity::WIRE_TRAILER_TAGS.len(),
        );
        for tag in identity::WIRE_HEADER_TAGS
            .into_iter()
            .chain(identity::LIFTED_TAGS)
        {
            emit(&mut bands, tag);
        }
        let split = bands.len();
        for tag in identity::WIRE_TRAILER_TAGS {
            emit(&mut bands, tag);
        }
        (bands, split)
    }

    /// Writes one value into the message, typed by the field the key reaches.
    ///
    /// The key resolves as every lookup does - through the registry's one
    /// namespace. A key reaching a typed fact - a header tag, a crate
    /// column, one of the event's own tags - records it on the holder that
    /// owns it, and a `Null` clears it. Any other key lands in the row: a
    /// field the dictionary knows types the value through [`Field::scalar`]
    /// under the dictionary's own field, so a written child is
    /// indistinguishable from a stated one and carries the same `FIX:tag` a
    /// reader resolves it by; a `Null` is stored as a stated null. An
    /// existing child is replaced where it stands and an absent one is
    /// appended, so the positions every reader already holding the row
    /// addresses it by do not move. A name the dictionary does not know
    /// still reaches a child spelled that way - exactly, or under the fold
    /// every name resolves by - and keeps that child's field; a bare tag it
    /// does not know appends a nullable `utf8` child named by its decimal,
    /// which is what the builder does with an unknown tag. A name that
    /// reaches neither a field nor a child is refused, and the message is
    /// unchanged.
    ///
    /// A written value is then restated exactly as a read
    /// one is: writing `Rule80A(47)` writes the `OrderCapacity(528)` that
    /// replaced it beside it, and writing `ExecBroker(76)` makes the
    /// `Parties` occurrence it became - from the crate's one table of the
    /// specification's retirements, since a registry states no rule of its
    /// own. What runs is the rules of the tags written, so a
    /// write of a tag no rule speaks for is the write and nothing more. The
    /// pass never overwrites a stated value and is idempotent, so writing
    /// one value twice writes its replacement once, and a caller who states
    /// the replacement keeps it. Every write settles the identity again.
    ///
    /// The written value reaches every sibling of its tag too: another
    /// child carrying the tag takes it, and a metadata key composing into
    /// it - `ULLINK.OFFERPRICE` for `OfferPx(133)` - or an alias that lost
    /// to it is stated again with it, so no statement the message keeps
    /// says another value. No sibling is created.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::graph::{Market, Operation};
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut clordid = DataType::utf8().nullable_field("clordid");
    /// clordid.as_fix_mut().set_tag(11)?;
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([clordid, symbol])?);
    ///
    /// let root = DataType::from(StructType::from_fields([DataType::utf8().required_field("symbol")])?)
    ///     .required_field("D");
    /// let value = Scalar::from_struct([("symbol", Scalar::from("AAPL"))])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// // `ClOrdID(11)` is a fact the message lifts, so it lands on its
    /// // holder and the row grows by nothing.
    /// msg.set(11, Scalar::from("A1"))?;
    /// assert_eq!(msg.by_tag(11)?, Scalar::from("A1"));
    /// assert_eq!(msg.lifted().clordid(), Some("A1"));
    /// assert_eq!(msg.as_field().fields().len(), 1);
    ///
    /// // Replaced in place: the child keeps its position, the value changes.
    /// msg.set("Symbol", Scalar::from("MSFT"))?;
    /// assert_eq!(msg.as_field().fields()[0].name(), "symbol");
    /// assert_eq!(msg.by_tag(55)?, Scalar::from("MSFT"));
    ///
    /// // `Price(44)` and `OrderQty(38)` are lifted too, and what the
    /// // message is *about* is read off them: its price, and what it ordered.
    /// msg.set(44, Scalar::from("82.5"))?;
    /// msg.set(38, Scalar::from(100_i64))?;
    /// assert_eq!(msg.get_price().map(|px| px.to_string()).as_deref(), Some("82.5"));
    /// assert_eq!(msg.get_ordqty().map(|qty| qty.to_string()).as_deref(), Some("100"));
    /// assert_eq!(msg.as_field().fields().len(), 1);
    ///
    /// // A tag no dictionary explains is kept under its decimal spelling.
    /// msg.set(9999, Scalar::from("custom"))?;
    /// assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"));
    ///
    /// // A name nothing reaches is refused, and the row stands as it was.
    /// assert!(msg.set("nosuchfield", Scalar::from("x")).is_err());
    /// assert_eq!(msg.as_field().fields().len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the key when it reaches no field and
    /// no child, the value contract's refusal when the value does not fit
    /// the field the key resolves to, or the schema grammar's refusal when
    /// the written child does not make a root. Any of them leaves the
    /// message unchanged.
    pub fn set<'key>(&mut self, key: impl Into<FixKey<'key>>, value: Scalar) -> Result<()> {
        self.set_many(std::iter::once((key, value)))
    }

    /// Writes several values into the message with one rebuild.
    ///
    /// Each key resolves and each value types exactly as [`Self::set`]
    /// resolves and types one, against the row as it stands before any of
    /// them lands; the row is then rebuilt once and the identity settled
    /// once. Two writes reaching one child, or two appending one field,
    /// land as the later one. Failure leaves the message unchanged,
    /// whichever write refused.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns for the first write that refuses.
    pub fn set_many<'key, I, K>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        self.set_many_with(values, |_, _| Ok(()))
    }

    /// Check each resolved target before its one value canonicalization.
    /// Protocol stamps can require a native layout that coercion would erase.
    pub(super) fn set_many_with<'key, I, K>(
        &mut self,
        values: I,
        check: impl Fn(&FixKey<'key>, &Field) -> Result<()>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        let mut writes: Vec<Write> = Vec::new();
        let mut typed: Vec<(i32, Scalar)> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            match self.staged(&key, value, &check)? {
                Staged::Typed(tag, value) => typed.push((tag, value)),
                Staged::Row(write) => stage(&mut writes, write),
            }
        }
        self.land(typed, writes)
    }

    /// Writes every value the field it reaches can hold, with one rebuild
    /// and no settling, answering how many landed.
    ///
    /// The lenient twin of [`Self::set_many`], for a pass whose answers are
    /// best effort: a value the target refuses - an identifier whose check
    /// digit does not close, a spelling its code set does not read - is
    /// dropped with a deduplicated warning naming the key rather than
    /// refused, and every other value lands as `set_many` lands it. Nothing
    /// else is lenient: the rebuild's refusal, which no single value causes,
    /// is still returned and leaves the message unchanged. The identity is
    /// not settled: the pass that writes settles once, after everything it
    /// writes.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the written children do
    /// not make a root.
    pub(super) fn set_each<'key, I, K>(&mut self, values: I) -> Result<usize>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        let mut writes: Vec<Write> = Vec::new();
        let mut typed: Vec<(i32, Scalar)> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            match self.staged(&key, value, &|_, _| Ok(())) {
                Ok(Staged::Typed(tag, value)) => typed.push((tag, value)),
                Ok(Staged::Row(write)) => stage(&mut writes, write),
                Err(error) => crate::warning::warned!(
                    "FIX value dropped: the field its key reaches refuses it",
                    &match key {
                        FixKey::Tag(tag) => format_smolstr!("{tag}"),
                        FixKey::Id(id) => format_smolstr!("{id}"),
                        FixKey::Name(name) => SmolStr::new(name),
                    },
                    "{error} on a {} message",
                    self.header.msgtype()
                ),
            }
        }
        let landed = typed.len() + writes.len();
        if landed > 0 {
            self.land_unsettled(typed, writes)?;
        }
        Ok(landed)
    }

    /// [`Self::set`] without settling: for the pass that writes several
    /// times and settles once after the last.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns.
    pub(super) fn set_unsettled<'key>(
        &mut self,
        key: impl Into<FixKey<'key>>,
        value: Scalar,
    ) -> Result<()> {
        let key = key.into();
        match self.staged(&key, value, &|_, _| Ok(()))? {
            Staged::Typed(tag, value) => self.land_unsettled(vec![(tag, value)], Vec::new()),
            Staged::Row(write) => self.land_unsettled(Vec::new(), vec![write]),
        }
    }

    /// Lands staged writes: the typed facts on their holders, the row
    /// writes in one rebuild, what the written values imply restated beside
    /// them, then the identity settled once.
    ///
    /// A value a caller writes is restated exactly as a value a line states
    /// is: writing `Rule80A(47)` writes the `OrderCapacity(528)` that
    /// replaced it beside it. The pass is idempotent and never overwrites a
    /// stated value, so a message written to twice is the message, and
    /// writing what a rule would have written stands.
    fn land(&mut self, typed: Vec<(i32, Scalar)>, writes: Vec<Write>) -> Result<()> {
        if typed.is_empty() && writes.is_empty() {
            return Ok(());
        }
        let tags: SmallVec<[i32; 8]> = typed
            .iter()
            .map(|(tag, _)| *tag)
            .chain(
                writes
                    .iter()
                    .filter_map(|write| write.field.as_fix().tag().ok().flatten()),
            )
            .collect();
        // Only what a rule could be about is restated: a write of a tag no
        // rule speaks for is the write, and the pass is not run at all.
        let restates = tags
            .iter()
            .any(|tag| super::latest::restates(&self.registry, *tag));
        // A message detection answered for is detected again when a write
        // reaches what it read, so a changed symbol derives its own pair and
        // takes back what the old one derived; a message it never answered
        // for is the intake's to detect.
        let redetects = (self.detected_fx != 0 || self.derives_pair())
            && tags.iter().any(|tag| super::forex::READS.contains(tag));
        self.land_unsettled(typed, writes)?;
        // A restatement or a detection rewrites fields no write named, so
        // every fact is stated again.
        if restates {
            super::latest::restate(self)?;
            self.stale |= fact::ALL;
        }
        if redetects {
            let registry = Arc::clone(&self.registry);
            self.derive_forex(registry.forex_memo())?;
            self.stale |= fact::ALL;
        }
        self.settle();
        Ok(())
    }

    /// [`Self::land`] without settling the identity after.
    ///
    /// A cell written is the writer's word, so FX detection no longer owns
    /// it; detection marks its own writes after they land.
    fn land_unsettled(&mut self, typed: Vec<(i32, Scalar)>, writes: Vec<Write>) -> Result<()> {
        if self.detected_fx != 0 {
            for write in &writes {
                if let Ok(Some(tag)) = write.field.as_fix().tag() {
                    self.detected_fx &= !super::forex::detected_bit(tag);
                }
            }
        }
        // What each write reaches is stated again at the next settle: a
        // field no tag maps reaches what a bridge's own spelling can state.
        for write in &writes {
            self.stale |= match write.field.as_fix().tag() {
                Ok(Some(tag)) => facts_of_tag(&self.registry, tag),
                _ => fact::NAMED,
            };
        }
        // Only a value something else states too is kept for replication:
        // a write lands on the child its tag already has, so no write makes
        // a sibling, and a tag nothing else states is never copied.
        let mut written: SmallVec<[(i32, Scalar); 4]> = writes
            .iter()
            .filter_map(|write| Some((write.field.as_fix().tag().ok()??, &write.value)))
            .filter(|(tag, _)| self.has_siblings(*tag))
            .map(|(tag, value)| (tag, value.clone()))
            .collect();
        if !writes.is_empty() {
            self.write_all(writes)?;
        }
        for (tag, value) in typed {
            self.record(tag, &value);
            if self.has_siblings(tag) {
                written.push((tag, value));
            }
        }
        self.replicate(&written)
    }

    /// Whether anything besides its own child states `tag`: another child
    /// carrying it, or a metadata key composing into it or lost to it.
    fn has_siblings(&self, tag: i32) -> bool {
        !self.siblings_of(tag).is_empty()
            || self
                .metadata
                .keys()
                .any(|key| super::build::metadata_tag(&self.registry, key) == Some(tag))
    }

    /// Writes each written value into the siblings of its tag: every other
    /// child carrying the tag, and each metadata key that composes into it
    /// or lost to it as an alias, so no statement of a fact the message
    /// holds says another value; a null clears the metadata keys. Nothing
    /// new is created, and a message with no siblings pays one scan of its
    /// tag index.
    fn replicate(&mut self, written: &[(i32, Scalar)]) -> Result<()> {
        if written.is_empty() {
            return Ok(());
        }
        if !self.metadata.is_empty() {
            let registry = Arc::clone(&self.registry);
            self.metadata.retain(|key, held| {
                let Some((tag, value)) = super::build::metadata_tag(&registry, key)
                    .and_then(|tag| written.iter().find(|(written, _)| *written == tag))
                else {
                    return true;
                };
                // Spelled as the wire spells the field.
                let text = match registry.get_field_by_tag(*tag) {
                    Some(field) => super::entry::wire_text_under(&registry, field, value),
                    None => super::entry::wire_text(value),
                };
                match text.filter(|text| !text.is_empty()) {
                    Some(text) => {
                        *held = text;
                        true
                    }
                    None => false,
                }
            });
        }
        let mut writes = Vec::new();
        for (tag, value) in written {
            for at in self.siblings_of(*tag) {
                let Some(sibling) = self.field.fields().get(at) else {
                    continue;
                };
                let value = if value.is_null() {
                    Scalar::Null
                } else if let Ok(typed) = sibling.scalar(value.clone()) {
                    typed
                } else if let Some(text) =
                    spelled(value).filter(|_| sibling.dtype().string_parameters().is_some())
                {
                    Scalar::from(text.as_str())
                } else {
                    continue;
                };
                let mut field = sibling.clone();
                if value.is_null() {
                    field.set_nullable(true);
                }
                writes.push(Write {
                    at: Some(at),
                    field,
                    value,
                });
            }
        }
        if writes.is_empty() {
            return Ok(());
        }
        self.write_all(writes)
    }

    /// The positions of the row children carrying `tag`, where more than
    /// one does; none otherwise.
    fn siblings_of(&self, tag: i32) -> SmallVec<[usize; 2]> {
        let first = self.tags.partition_point(|(held, _)| *held < tag);
        let same = self.tags[first..]
            .iter()
            .take_while(|(held, _)| *held == tag)
            .map(|(_, at)| *at)
            .collect::<SmallVec<[usize; 2]>>();
        if same.len() < 2 {
            return SmallVec::new();
        }
        same
    }

    /// One value resolved and typed for the child its key reaches, exactly
    /// as [`Self::set`] resolves and types one, against the row as it stands.
    fn staged<'key>(
        &self,
        key: &FixKey<'key>,
        value: Scalar,
        check: &impl Fn(&FixKey<'key>, &Field) -> Result<()>,
    ) -> Result<Staged> {
        let (at, mut field) = self.target(key)?;
        check(key, &field)?;
        let tag = field.as_fix().tag()?;
        // The capture's own column is nobody's to write here: a message
        // holds no fact for it, and landing one in the row would make the
        // object a line was read from a pair this message re-emits. Whoever
        // read the line states it on the row instead.
        if let Some(tag) = tag.filter(|tag| identity::is_capture_tag(*tag)) {
            return Err(refused_capture(field.name(), tag));
        }
        if let FixKey::Tag(tag) = *key
            && identity::is_capture_tag(tag)
        {
            return Err(refused_capture(field.name(), tag));
        }
        if let Some(tag) = tag.filter(|tag| identity::is_typed_tag(*tag)) {
            let value = if value.is_null() {
                Scalar::Null
            } else {
                field.scalar(value)?
            };
            return Ok(Staged::Typed(tag, value));
        }
        // Which tags the holders own is this crate's statement and not a
        // dictionary's, so a typed tag written under a registry that
        // declares no field for it still lands on its holder. There is no
        // field to type it through - the one above was invented for the
        // spelling - so the value crosses as it was written and the holder
        // types it.
        if tag.is_none()
            && let FixKey::Tag(tag) = *key
            && identity::is_typed_tag(tag)
        {
            return Ok(Staged::Typed(tag, value));
        }
        let value = if value.is_null() {
            field.set_nullable(true);
            Scalar::Null
        } else {
            field.scalar(value)?
        };
        Ok(Staged::Row(Write { at, field, value }))
    }

    /// [`Self::set`], consuming the message.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns.
    pub fn with_value<'key>(mut self, key: impl Into<FixKey<'key>>, value: Scalar) -> Result<Self> {
        self.set(key, value)?;
        Ok(self)
    }

    /// Removes what a key reaches, answering the value it held.
    ///
    /// A key reaching a typed fact clears it on its holder; one reaching a
    /// row child removes the child, and the children after it move up, so
    /// the tag and group indexes are reread. Its siblings go with it: the
    /// other children carrying its tag and the metadata keys that compose
    /// into it or lost to it as an alias. A key that reaches nothing
    /// answers `None` and changes nothing.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([symbol.clone()])?);
    /// let root = DataType::from(StructType::from_fields([symbol, DataType::utf8().nullable_field("9999")])?)
    ///     .required_field("D");
    /// let value = Scalar::from_struct([
    ///     ("symbol", Scalar::from("AAPL")),
    ///     ("9999", Scalar::from("custom")),
    /// ])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// assert_eq!(msg.remove(55)?, Some(Scalar::from("AAPL")));
    /// assert_eq!(msg.get_by_tag(55), None);
    /// assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"), "the neighbour is still reached");
    /// assert_eq!(msg.remove("nosuchfield")?, None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the remaining children do
    /// not make a root, which leaves the message unchanged.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Result<Option<Scalar>> {
        let key = key.into();
        if let Some(tag) = self.typed_tag_of(&key) {
            let held = self.typed_fact(tag);
            if held.is_some() {
                self.record(tag, &Scalar::Null);
                self.replicate(&[(tag, Scalar::Null)])?;
                self.settle();
            }
            return Ok(held);
        }
        let Some(at) = self.index_of_key(&key) else {
            return Ok(None);
        };
        let mut members = self.field.fields().to_vec();
        let mut values = self
            .value
            .as_sequence()
            .ok_or_else(|| {
                identity::refused(self.field.name(), "a canonical row", self.value.kind())
            })?
            .to_vec();
        if at >= members.len() || at >= values.len() {
            return Ok(None);
        }
        let tag = members[at].as_fix().tag().ok().flatten();
        self.stale |= tag.map_or(fact::NAMED, |tag| facts_of_tag(&self.registry, tag));
        // The children sharing the tag go with it, highest first so each
        // position still names its child.
        let mut gone = tag.map(|tag| self.siblings_of(tag)).unwrap_or_default();
        if !gone.contains(&at) {
            gone.push(at);
        }
        gone.sort_unstable_by(|left, right| right.cmp(left));
        let mut removed = Scalar::Null;
        for position in gone {
            if position >= members.len() || position >= values.len() {
                continue;
            }
            members.remove(position);
            let value = values.remove(position);
            if position == at {
                removed = value;
            }
        }
        if let Some(tag) = tag.filter(|_| !self.metadata.is_empty()) {
            let registry = Arc::clone(&self.registry);
            self.metadata
                .retain(|key, _| super::build::metadata_tag(&registry, key) != Some(tag));
        }
        let dtype = DataType::from(StructType::from_fields(members)?);
        let field = self.rerooted(dtype);
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field, &self.registry);
        self.named = OnceLock::new();
        self.entries = OnceLock::new();
        self.settle();
        Ok(Some(removed))
    }

    /// The typed tag a key reaches, where it reaches one.
    fn typed_tag_of(&self, key: &FixKey<'_>) -> Option<i32> {
        let tag = match *key {
            FixKey::Tag(tag) => tag,
            FixKey::Id(id) => {
                self.registry
                    .identity_of(self.registry.get_field_by_id(id)?)?
                    .0
            }
            FixKey::Name(name) => {
                let known = self.known_by_name(name)?;
                self.registry.identity_of(known)?.0
            }
        };
        identity::is_typed_tag(tag).then_some(tag)
    }

    /// The child a key reaches and the field it is written under: an
    /// existing child's position where one is reached, and the field the
    /// registry resolves the key to, else the reached child's own.
    fn target(&self, key: &FixKey<'_>) -> Result<(Option<usize>, Field)> {
        match *key {
            FixKey::Tag(tag) => {
                let at = self.reached_by_tag(tag);
                match self.known_by_tag(tag) {
                    Some(known) => Ok((at, stated(known))),
                    None => {
                        let field = at
                            .and_then(|at| self.field.get_field_at(at))
                            .cloned()
                            .unwrap_or_else(|| {
                                DataType::utf8().nullable_field(format_smolstr!("{tag}"))
                            });
                        Ok((at, field))
                    }
                }
            }
            FixKey::Id(id) => {
                let known = self
                    .registry
                    .get_field_by_id(id)
                    .ok_or_else(|| absent(key))?;
                let at = self.index_of_name(known.name()).or_else(|| {
                    self.registry
                        .identity_of(known)
                        .and_then(|(tag, _)| self.index_of_tag(tag))
                });
                Ok((at, stated(known)))
            }
            FixKey::Name(name) => match self.known_by_name(name) {
                Some(known) => {
                    let by_tag = known
                        .as_fix()
                        .tag()
                        .ok()
                        .flatten()
                        .and_then(|tag| self.index_of_tag(tag));
                    Ok((
                        by_tag.or_else(|| self.child_index(&self.field, name)),
                        stated(known),
                    ))
                }
                None => {
                    let at = self
                        .child_index(&self.field, name)
                        .ok_or_else(|| absent(key))?;
                    let field = self
                        .field
                        .get_field_at(at)
                        .cloned()
                        .ok_or_else(|| absent(key))?;
                    Ok((Some(at), field))
                }
            },
        }
    }

    /// The position of the row child a key reaches, as a lookup reaches it.
    fn index_of_key(&self, key: &FixKey<'_>) -> Option<usize> {
        match *key {
            FixKey::Tag(tag) => self.reached_by_tag(tag),
            FixKey::Id(id) => {
                let known = self.registry.get_field_by_id(id)?;
                self.field.index_of(known.name())
            }
            FixKey::Name(name) => self.child_index(&self.field, name),
        }
    }

    /// Lands the planned row writes: each replaced at its position,
    /// appended when it has none.
    ///
    /// The whole row is rebuilt once, because a Struct's children and a
    /// row's values are both one shared allocation; everything that can
    /// refuse is asked before anything is stored, so a refusal leaves the
    /// message as it was. The tag and group indexes follow the children that
    /// changed, and the name table is carried through a write that leaves
    /// it true - an append, or a replacement under the child's own name -
    /// and dropped only where a rename makes it wrong.
    fn write_all(&mut self, writes: Vec<Write>) -> Result<()> {
        let (field, values, indexed) = stage_writes(&self.field, &self.value, writes)?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        let mut named = self.named.take();
        for change in indexed {
            if change.renamed {
                named = None;
            } else if change.appended
                && let (Some(named), Some(child)) =
                    (named.as_mut(), self.field.fields().get(change.index))
            {
                Arc::make_mut(named)
                    .entry(SmolStr::new(child.name()))
                    .or_insert(change.index);
            }
            change.apply(&mut self.tags, Some(&mut self.groups));
        }
        self.named = OnceLock::new();
        if let Some(named) = named {
            let _ = self.named.set(named);
        }
        self.entries = OnceLock::new();
        Ok(())
    }

    /// The root over other children: its name, nullability and metadata,
    /// the Struct `dtype` under them.
    fn rerooted(&self, dtype: DataType) -> Field {
        Field::new_with_metadata(
            self.field.name(),
            dtype,
            self.field.is_nullable(),
            self.field.as_metadata().clone(),
        )
    }

    /// Re-emits this message on the wire, separated by `separator`.
    ///
    /// The standard header from the typed header, the event's own FIX tags,
    /// then the row in its order, each entry stating a value as one pair
    /// and an occurrence or a component as the pairs under it. What is
    /// emitted is the message as it now stands, derived values included.
    #[must_use]
    pub fn into_bytes(&self, separator: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let (bands, split) = self.wire_bands();
        let (head, tail) = bands.split_at(split);
        emit_bytes(head, separator, &mut bytes);
        emit_bytes(self.entries(), separator, &mut bytes);
        emit_bytes(tail, separator, &mut bytes);
        bytes
    }

    /// The same, as text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when a value holds a control byte.
    pub fn into_text(&self, separator: char) -> Result<String> {
        let mut text = String::new();
        let (bands, split) = self.wire_bands();
        let (head, tail) = bands.split_at(split);
        emit_text(head, separator, &mut text)?;
        emit_text(self.entries(), separator, &mut text)?;
        emit_text(tail, separator, &mut text)?;
        Ok(text)
    }

    /// The deterministic digest of what the wire carries: every entry of
    /// [`Self::into_bytes`], pre-order, so two messages that re-emit alike
    /// digest alike whatever separator either was read with.
    #[must_use]
    pub fn digest(&self) -> u128 {
        let (bands, split) = self.wire_bands();
        let (head, tail) = bands.split_at(split);
        super::digest::digest_of(&[head, self.entries(), tail])
    }

    /// Returns the registry this message resolves against.
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// Returns the root Struct field: the row's schema, which holds every
    /// child the message states beyond its typed facts.
    pub const fn as_field(&self) -> &Field {
        &self.field
    }

    /// Returns the ordered row value.
    pub const fn as_value(&self) -> &Scalar {
        &self.value
    }

    /// Returns the value the root child an identifier names.
    ///
    /// An identifier is exact: it names one field, and the child is the one
    /// that field's name reaches.
    pub fn get_by_id(&self, id: FixId) -> Option<Scalar> {
        let known = self.registry.get_field_by_id(id)?;
        if let Some((tag, _)) = self.registry.identity_of(known)
            && identity::is_typed_tag(tag)
        {
            return self.typed_fact(tag);
        }
        self.value
            .get(self.field.index_of(known.name())?)
            .map(Cow::into_owned)
    }

    /// Returns the value the root child an identifier names, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the identifier.
    pub fn by_id(&self, id: FixId) -> Result<Scalar> {
        self.get_by_id(id).ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the value a tag names.
    ///
    /// A tag the typed holders own - a header tag, a crate column, one of
    /// the event's own tags - answers the fact the holder states, or nothing
    /// where it states none. Any other tag reaches the row: the registry
    /// resolves it to its canonical name, and that name picks the root
    /// child; a tag the dictionary does not answer is looked for under its
    /// decimal rendering, so an unknown tag a transcriber retained is still
    /// reachable.
    pub fn get_by_tag(&self, tag: i32) -> Option<Scalar> {
        if identity::is_typed_tag(tag) {
            return self.typed_fact(tag);
        }
        self.value
            .get(self.reached_by_tag(tag)?)
            .map(Cow::into_owned)
    }

    /// The value a tag names by the index alone: a typed fact, else the
    /// child declaring the tag, and never the two fallbacks of
    /// [`Self::get_by_tag`], which end in a name table built on the first
    /// miss. The [enriching pass](super::enrich) reads every native source
    /// and gathers every generic working-row column through this.
    pub(super) fn indexed_by_tag(&self, tag: i32) -> Option<Scalar> {
        if identity::is_typed_tag(tag) {
            return self.typed_fact(tag);
        }
        self.value.get(self.index_of_tag(tag)?).map(Cow::into_owned)
    }

    /// Whether [`Self::indexed_by_tag`] answers a value other than null, read
    /// where the value lies rather than copied out of it.
    pub(super) fn states_indexed_tag(&self, tag: i32) -> bool {
        if identity::is_typed_tag(tag) {
            return self.typed_fact(tag).is_some_and(|value| !value.is_null());
        }
        self.index_of_tag(tag)
            .and_then(|index| self.value.get(index))
            .is_some_and(|value| !value.is_null())
    }

    /// The child a tag reaches: the one carrying the tag, by one hash-free
    /// binary search over the index resolved at construction, else the one
    /// either fallback names.
    fn reached_by_tag(&self, tag: i32) -> Option<usize> {
        if let Some(index) = self.index_of_tag(tag) {
            return Some(index);
        }
        self.fallback_index(tag)
    }

    /// The child a tag reaches past the index: the one named as the
    /// dictionary names the tag, else the one named by the tag's decimal
    /// spelling.
    fn fallback_index(&self, tag: i32) -> Option<usize> {
        let named = self.named.get_or_init(|| {
            let mut named = FixMap::default();
            named.reserve(self.field.fields().len());
            for (index, child) in self.field.fields().iter().enumerate() {
                named.entry(SmolStr::new(child.name())).or_insert(index);
            }
            Arc::new(named)
        });
        match self.known_by_tag(tag) {
            Some(known) => named.get(known.name()).copied(),
            None => named.get(format_smolstr!("{tag}").as_str()).copied(),
        }
    }

    /// The root child carrying one tag, by that child's own declaration.
    pub(super) fn index_of_tag(&self, tag: i32) -> Option<usize> {
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        self.tags
            .get(at)
            .filter(|(held, _)| *held == tag)
            .map(|(_, index)| *index)
    }

    /// A tag identifies a member only when exactly one root child carries it.
    pub(super) fn unique_index_of_tag(&self, tag: i32) -> Option<usize> {
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        let (held, index) = self.tags.get(at)?;
        if *held != tag || self.tags.get(at + 1).is_some_and(|(next, _)| *next == tag) {
            return None;
        }
        Some(*index)
    }

    pub(super) fn index_of_group(&self, counter: i32) -> Option<usize> {
        let index = self
            .groups
            .binary_search_by_key(&counter, |(tag, _)| *tag)
            .ok()?;
        if index > 0 && self.groups[index - 1].0 == counter
            || self
                .groups
                .get(index + 1)
                .is_some_and(|(tag, _)| *tag == counter)
        {
            return None;
        }
        Some(self.groups[index].1)
    }

    /// Returns the value a tag names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the tag.
    pub fn by_tag(&self, tag: i32) -> Result<Scalar> {
        self.get_by_tag(tag).ok_or_else(|| absent(FixKey::Tag(tag)))
    }

    /// Returns the value a name reaches.
    ///
    /// The name folds through the registry to its canonical spelling - a
    /// typed fact answers from its holder - and an exact root-child match is
    /// the fallback when the registry does not know it.
    pub fn get_by_name(&self, name: &str) -> Option<Scalar> {
        if let Some(known) = self.known_by_name(name)
            && let Some((tag, _)) = self.registry.identity_of(known)
            && identity::is_typed_tag(tag)
        {
            return self.typed_fact(tag);
        }
        self.value
            .get(self.child_index(&self.field, name)?)
            .map(Cow::into_owned)
    }

    /// Returns the value a name reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the name.
    pub fn by_name(&self, name: &str) -> Result<Scalar> {
        self.get_by_name(name)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// Returns the value a resolved path reaches.
    ///
    /// The path is the crate's one grammar, already parsed: nothing here
    /// splits a string, so a run addressing the same member a million times
    /// resolves the path once. A named segment resolves as
    /// [`Self::get_by_name`] does - the registry's canonical spelling first,
    /// then an exact match - and an indexed segment takes one occurrence of
    /// the Serie a repeating group is, which is what reaching a member
    /// needs: `Parties[0].PartyID`.
    ///
    /// A bare decimal is a name and not a position, exactly as it is one
    /// layer down where a text line's entry keyed `55` is reached by the path
    /// `55`. A path of one named segment is [`Self::get_by_name`].
    pub fn get_by_path(&self, path: &FieldPath) -> Option<Scalar> {
        let mut segments = path.segments().iter();
        let first = segments.next()?;
        let name = first.as_name()?;
        let (mut field, mut value) = match self.known_by_name(name) {
            Some(known)
                if self
                    .registry
                    .identity_of(known)
                    .is_some_and(|(tag, _)| identity::is_typed_tag(tag)) =>
            {
                let (tag, _) = self.registry.identity_of(known)?;
                (known.clone(), self.typed_fact(tag)?)
            }
            _ => {
                let index = self.child_index(&self.field, name)?;
                (
                    self.field.fields().get(index)?.clone(),
                    self.value.get(index)?.into_owned(),
                )
            }
        };
        for segment in segments {
            (field, value) = self.descend(&field, &value, segment)?;
        }
        Some(value)
    }

    /// Returns the value a resolved path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the path.
    pub fn by_path(&self, path: &FieldPath) -> Result<Scalar> {
        self.get_by_path(path)
            .ok_or_else(|| absent(format_args!("path {path}")))
    }

    /// Returns the value a tag, an identifier, a name or a path reaches.
    ///
    /// Matches the key once and redirects: a tag to [`Self::get_by_tag`], an
    /// identifier to [`Self::get_by_id`], a name to [`Self::get_by_name`] -
    /// and, only where that reached nothing and the key spells more than one
    /// segment, to [`Self::get_by_path`] with the path that key states.
    pub fn get<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.get_by_tag(tag),
            FixKey::Id(id) => self.get_by_id(id),
            FixKey::Name(name) => self.named_or_path(name),
        }
    }

    /// Returns the value a tag, an identifier or a name reaches, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::by_tag`], [`Self::by_id`] or
    /// [`Self::by_path`] raises, whichever the key selects.
    pub fn value<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.by_tag(tag),
            FixKey::Id(id) => self.by_id(id),
            FixKey::Name(name) => {
                if let Some(value) = self.get_by_name(name) {
                    return Ok(value);
                }
                match FieldPath::from_str(name) {
                    Ok(path) if path.segments().len() > 1 => self.by_path(&path),
                    _ => Err(absent(FixKey::Name(name))),
                }
            }
        }
    }

    /// The field a bare tag names: the tag's first holder in the registry.
    pub(super) fn known_by_tag(&self, tag: i32) -> Option<&Field> {
        self.registry.get_field_by_tag(tag).or_else(|| {
            self.registry
                .get_group_by_tag(tag)
                .filter(|group| matches!(group.dtype(), DataType::Map(_) | DataType::SortedMap(_)))
        })
    }

    /// The field a bare name reaches, canonical spelling or alias.
    pub(super) fn known_by_name(&self, name: &str) -> Option<&Field> {
        self.registry.get_message_field_by_name(name)
    }

    /// The position of the child `name` reaches under `parent`: the
    /// registry's canonical spelling first, then an exact match.
    fn child_index(&self, parent: &Field, name: &str) -> Option<usize> {
        self.known_by_name(name)
            .and_then(|known| parent.index_of(known.name()))
            .or_else(|| named_index(parent, name))
    }

    /// The position of the root child `name` spells, with no dictionary
    /// consulted: an exact match, else the one child the fold reaches.
    pub(super) fn index_of_name(&self, name: &str) -> Option<usize> {
        named_index(&self.field, name)
    }

    /// What a name no dictionary maps states: the root child it spells -
    /// exactly, else the one the fold reaches. A row read back restores such
    /// a key as the child a parse held it as, so a fact read by name reads
    /// the same off a parse and off its row.
    pub(super) fn named_value(&self, name: &str) -> Option<Scalar> {
        let at = self.index_of_name(name)?;
        self.value.as_sequence()?.get(at).cloned()
    }

    /// One key read as a name, and as the path it spells where it spells one.
    fn named_or_path(&self, key: &str) -> Option<Scalar> {
        if let Some(value) = self.get_by_name(key) {
            return Some(value);
        }
        let path = FieldPath::from_str(key).ok()?;
        (path.segments().len() > 1)
            .then(|| self.get_by_path(&path))
            .flatten()
    }

    /// The position one named segment reaches under `parent`.
    fn segment_index(&self, parent: &Field, segment: &FieldSegment) -> Option<usize> {
        self.child_index(parent, segment.as_name()?)
    }

    /// One step of a path: into a Struct child by name, or into one
    /// occupancy of the Serie a repeating group is.
    fn descend(
        &self,
        field: &Field,
        value: &Scalar,
        segment: &FieldSegment,
    ) -> Option<(Field, Scalar)> {
        match field.dtype() {
            DataType::Struct(_) => {
                let index = self.segment_index(field, segment)?;
                Some((
                    field.fields().get(index)?.clone(),
                    value.get(index)?.into_owned(),
                ))
            }
            DataType::Serie(item)
            | DataType::LargeSerie(item)
            | DataType::FixedSizeSerie(item, _)
            | DataType::SerieView(item)
            | DataType::LargeSerieView(item) => {
                let FieldSegment::Index(position) = segment else {
                    return None;
                };
                let len = value.as_serie()?.len();
                let at = if *position < 0 {
                    len.checked_sub(position.unsigned_abs() as usize)?
                } else {
                    usize::try_from(*position).ok()?
                };
                Some((item.as_ref().clone(), value.get(at)?.into_owned()))
            }
            map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
                let map = &map_dtype
                    .as_mapping()
                    .expect("the variant was just matched");
                let FieldSegment::Key(key) = segment else {
                    return None;
                };
                Some((
                    map.entries().fields().get(1)?.clone(),
                    value.get_key(key.value())?.clone(),
                ))
            }
            _ => None,
        }
    }
}

/// The canonical entry order keeps up to 128 indexes (one KiB) on the stack.
const DIGEST_ENTRY_STACK_INDICES: usize = 128;

/// Fills the one missing member of an FX triple where the other two are
/// stated - `price = spot + points`, `spot = price - points`, `points =
/// price - spot` - by checked decimal arithmetic and no pip scaling; three
/// stated, even disagreeing, and fewer than two are left alone, and so is a
/// sum the decimal cannot hold. Run only over one tag triple, which is one
/// statement: `LastPx(31)`, `LastSpotRate(194)` and `LastForwardPoints(195)`.
fn fx_triple(
    price: &mut Option<Decimal>,
    spot: &mut Option<Decimal>,
    points: &mut Option<Decimal>,
) {
    let (slot, filled) = match (*price, *spot, *points) {
        (None, Some(held_spot), Some(held_points)) => (price, held_spot.checked_add(held_points)),
        (Some(held_price), None, Some(held_points)) => (spot, held_price.checked_sub(held_points)),
        (Some(held_price), Some(held_spot), None) => (points, held_price.checked_sub(held_spot)),
        _ => return,
    };
    *slot = filled;
}

/// The three parts an instrument key names, each read by its own type and
/// absent where that type refuses it, beside the source that named it.
struct InstrumentKey {
    isin: Option<crate::Isin>,
    mic: Option<Mic>,
    ccy: Option<Ccy>,
    src: IdSource,
}

/// The first `{ISIN}_{MIC}_{CCY}` an `INSTRUMENTID` security identifier
/// spells after its last `;` - `dbi;CH0012214059_XSWX_CHF` - read by
/// [`SymbolCode::instrument`](crate::securityid::SymbolCode::instrument),
/// the one reading of that shape a symbol is read by too.
fn instrument_key(ids: &Identifiers) -> Option<InstrumentKey> {
    ids.of_kind(&IdType::InstrumentId).find_map(|id| {
        match crate::securityid::SymbolCode::instrument(id.value().rsplit(';').next()?)? {
            crate::securityid::SymbolCode::Instrument { isin, mic, ccy } => Some(InstrumentKey {
                isin,
                mic,
                ccy,
                src: id.src().clone(),
            }),
            _ => None,
        }
    })
}

/// The suffix a source spelling a venue's own instrument key ends with,
/// whatever namespace leads it.
const INSTRUMENT_ID: &str = "instrumentid";

/// The security identifier a stated `source` and `code` make, from `src`:
/// a source spelled `{NAMESPACE}INSTRUMENTID` - `ULLINKINSTRUMENTID` - is an
/// [`IdType::InstrumentId`] from that namespace, and every other source is
/// the type [`IdType::from_security_source`] reads it as.
pub(super) fn security_identifier(
    source: &str,
    src: IdSource,
    code: &str,
) -> crate::Result<Identifier> {
    match instrument_namespace(source) {
        Some(namespace) => Identifier::new(namespace.parse()?, IdType::InstrumentId, code),
        None => Identifier::new(src, IdType::from_security_source(source)?, code),
    }
}

/// The namespace a source spelled `{NAMESPACE}INSTRUMENTID` names, matched
/// in place ignoring case: the namespace is folded where the identifier is
/// built, so no upper-cased copy of the source is made.
fn instrument_namespace(source: &str) -> Option<&str> {
    let source = source.trim();
    source
        .len()
        .checked_sub(INSTRUMENT_ID.len())
        .filter(|at| *at > 0)
        .and_then(|at| Some((source.get(..at)?, source.get(at..)?)))
        .filter(|(_, suffix)| suffix.eq_ignore_ascii_case(INSTRUMENT_ID))
        .map(|(namespace, _)| namespace)
}

/// Feeds one level of the entry tree to a digest in canonical name order.
/// Equal names retain their arrival order, which keeps occurrences of one
/// repeating group distinct while making a reconstructed Struct's members
/// answer the same code as the parsed message's.
fn feed_entries(state: &mut crate::xxhash::Xxh3, entries: &[FixEntry]) {
    if entries
        .windows(2)
        .all(|pair| pair[0].name() <= pair[1].name())
    {
        for entry in entries {
            feed_entry(state, entry);
        }
        return;
    }
    let mut inline = [0_usize; DIGEST_ENTRY_STACK_INDICES];
    let mut heap = Vec::new();
    let order = if entries.len() <= inline.len() {
        &mut inline[..entries.len()]
    } else {
        heap.resize(entries.len(), 0);
        heap.as_mut_slice()
    };
    for (index, slot) in order.iter_mut().enumerate() {
        *slot = index;
    }
    order.sort_unstable_by(|left, right| {
        entries[*left]
            .name()
            .cmp(entries[*right].name())
            .then(left.cmp(right))
    });
    for index in order {
        feed_entry(state, &entries[*index]);
    }
}

/// Feeds one entry with boundaries around each variable-length member.
fn feed_entry(state: &mut crate::xxhash::Xxh3, entry: &FixEntry) {
    if super::digest::is_envelope(entry.tag()) {
        return;
    }
    state.write_usize(entry.name().len());
    state.write(entry.name().as_bytes());
    if let Some(value) = entry.value() {
        state.write_u8(1);
        state.write_usize(value.len());
        state.write(value.as_bytes());
    } else {
        state.write_u8(0);
    }
    state.write_usize(
        entry
            .entries()
            .iter()
            .filter(|child| !super::digest::is_envelope(child.tag()))
            .count(),
    );
    feed_entries(state, entry.entries());
}

/// One level of the row as the entries it states, in its order: every
/// child through [`entry_of`], except the counter scalar beside the group
/// it counts - at the root, in a component, in an occurrence alike - since
/// a group's count is the group entry's own value and the counter child
/// states nothing the entries do not already; and except a list holding
/// nothing that no counter beside it states the count of.
///
/// A group holding no occurrence is stated by its count alone: the parse
/// holds `NoPartySubIDs(802)=0` as the counter stating zero beside the
/// empty list, which is the group's entry and re-emits. An empty list with
/// no count stated beside it is the group absent - a table may store a null
/// list as an empty one, and PyIceberg reads a null list of structs back as
/// `[]` - so a row read back and settled again feeds its content code, its
/// digest and the delivery a lifecycle folds it by as the parse did.
fn entries_of(registry: &FixRegistry, fields: &[Field], values: &[Scalar]) -> Vec<FixEntry> {
    let counters: Vec<i32> = fields
        .iter()
        .filter(|child| child.dtype().is_nested())
        .filter_map(|child| super::schema::tag_and_counter(registry, child).1)
        .collect();
    let counted = |child: &Field| {
        !counters.is_empty()
            && !child.dtype().is_nested()
            && super::schema::tag_and_counter(registry, child)
                .0
                .is_some_and(|tag| counters.contains(&tag))
    };
    let unstated = |child: &Field, value: &Scalar| {
        super::schema::is_unstated_group(registry, child, value, fields, values)
    };
    let mut entries = Vec::new();
    for entry in fields
        .iter()
        .zip(values)
        .filter(|(child, value)| !counted(child) && !unstated(child, value))
        .filter_map(|(child, value)| entry_of(registry, child, value))
    {
        // Sized once, on the first entry, for every child there is: a level
        // stating nothing allocates nothing here, and one stating a hundred
        // grows the list once.
        if entries.capacity() == 0 {
            entries.reserve_exact(fields.len());
        }
        entries.push(entry);
    }
    entries
}

/// One row child as the entry it is: a scalar as one stated entry, a
/// repeating group as its counter entry with an entry per occurrence and
/// the occurrence's members under each, a component as an entry heading
/// its members; nothing for a child stating null.
fn entry_of(registry: &FixRegistry, field: &Field, value: &Scalar) -> Option<FixEntry> {
    if value.is_null() {
        return None;
    }
    let (tag, counter) = super::schema::tag_and_counter(registry, field);
    let tag = tag.unwrap_or(0);
    match field.dtype() {
        DataType::Serie(item) | DataType::LargeSerie(item) => {
            let occurrences = value.as_serie()?;
            // The item is one field for every occurrence, so its facts are
            // read once for all of them.
            let item_facts = super::schema::tag_and_counter(registry, item);
            let nested: Vec<FixEntry> = occurrences
                .iter()
                .filter_map(|occurrence| match item.dtype() {
                    DataType::Struct(_) => {
                        let members =
                            entries_of(registry, item.fields(), occurrence.as_sequence()?);
                        let own = item_facts.0.unwrap_or(0);
                        Some(FixEntry::new(own, item.name(), None).with_entries(members))
                    }
                    _ => entry_of(registry, item, &occurrence),
                })
                .collect();
            match counter {
                Some(counter) => Some(
                    FixEntry::new(
                        counter,
                        field.name(),
                        Some(format_smolstr!("{}", occurrences.len())),
                    )
                    .with_entries(nested),
                ),
                None => Some(FixEntry::new(tag, field.name(), None).with_entries(nested)),
            }
        }
        DataType::Struct(_) => {
            let members = entries_of(registry, field.fields(), value.as_sequence()?);
            Some(FixEntry::new(tag, field.name(), None).with_entries(members))
        }
        DataType::Map(_) | DataType::SortedMap(_) => {
            let members = value
                .as_mapping()?
                .iter()
                .filter_map(|(key, value)| Some(FixEntry::new(0, key.as_str()?, wire_text(value))))
                .collect();
            Some(FixEntry::new(tag, field.name(), None).with_entries(members))
        }
        _ => Some(FixEntry::new(
            tag,
            field.name(),
            wire_text_under(registry, field, value),
        )),
    }
}

/// The group a message expands into one leaf per occurrence, and what each
/// of those leaves reads out of its own occurrence.
pub(super) struct Expanded {
    /// The group's `FIX:counter`.
    pub(super) counter: i32,
    /// The tags a leaf reads out of its own occurrence, at any depth of it.
    pub(super) reads: &'static [i32],
    /// The root tags every occurrence inherits, which are the root's own
    /// reads: a book message's context, and none for a trade.
    pub(super) inherited: &'static [i32],
}

/// What a message states that no typed column of its leaves reads,
/// [`FixMsg::unmapped`]'s answer.
pub(super) struct Unmapped {
    /// The message's own, every leaf of it carries.
    pub(super) message: Metadata,
    /// The message's own scalars whose keys end with an identifier name,
    /// which every leaf of it lifts.
    pub(super) lifted: Lifted,
    /// Each occurrence of the expanded group, in order: its own members,
    /// keyed bare, and its own scalars to lift, which only the occurrence's
    /// own leaf carries.
    pub(super) occurrences: Vec<(Metadata, Lifted)>,
}

/// The scalars a leaf's metadata sets aside for its identifier sets, each
/// under its key as stated beside the identifier its key and its value
/// name ([`Holds::land`]).
pub(super) type Lifted = Vec<(SmolStr, SmolStr, Identifier)>;

/// What one leaf carries out of its message: its metadata, and the scalars
/// it lifts into its alternate identifiers - the occurrence's before the
/// message's.
pub(super) struct Carried {
    pub(super) metadata: Metadata,
    pub(super) lifted: Lifted,
}

impl Unmapped {
    /// What the leaf of occurrence `index` carries: the message's own, then
    /// the occurrence's, which leads a message field of the same name. The
    /// occurrence's is moved out, so each is asked for once.
    pub(super) fn leaf(&mut self, index: usize) -> Carried {
        let mut metadata = self.message.clone();
        let mut lifted = Lifted::new();
        if let Some((own, own_lifted)) = self.occurrences.get_mut(index) {
            metadata.append(own);
            lifted.append(own_lifted);
        }
        lifted.extend(self.lifted.iter().cloned());
        Carried { metadata, lifted }
    }

    /// What the one leaf of a message expanding no group carries.
    pub(super) fn into_carried(self) -> Carried {
        Carried {
            metadata: self.message,
            lifted: self.lifted,
        }
    }
}

/// What the identifier maps of a leaf hold of what its message states,
/// which the leaf's metadata leaves out: a party its accounts hold under
/// its role's key - or an identifier map reads under its role - a
/// regulatory trade identifier its alternate identifiers hold under its
/// type's key, and the `Account(1)` its accounts hold. What a map does not
/// hold - a second party of a role, a value no map takes - stays.
struct Holds<'a> {
    registry: &'a FixRegistry,
    codes: PartyCodes<'a>,
    partyids: &'a Identifiers,
    identifiers: &'a Identifiers,
}

impl Holds<'_> {
    /// Whether a party of `kind` with `value` is held: an occurrence's
    /// `own` first, then the message's.
    fn holds_party(&self, own: Option<&Identifiers>, kind: &IdType, value: &str) -> bool {
        own.into_iter()
            .chain(std::iter::once(self.partyids))
            .any(|ids| ids.of_kind(kind).any(|held| held.value() == value))
    }

    /// Whether the account the leaf holds is `value`.
    fn holds_account(&self, own: Option<&Identifiers>, value: &str) -> bool {
        self.holds_party(own, &IdType::Account, value.trim())
    }

    /// Whether one occurrence of the party or regulatory group `counter`
    /// counts - its `cells`, read at the identifier's and the role's or
    /// type's `positions` - is held by the leaf's identifiers.
    fn holds_occurrence(
        &self,
        counter: i32,
        own: Option<&Identifiers>,
        positions: [Option<usize>; 2],
        cells: &[Scalar],
    ) -> bool {
        let [id, other] = positions.map(|at| at.and_then(|at| cells.get(at)).and_then(scalar_text));
        let Some(id) = id else {
            return false;
        };
        let other = other.as_deref();
        if PARTY_GROUPS.iter().any(|(held, ..)| *held == counter) {
            return self
                .codes
                .party(&id, other, None)
                .is_some_and(|party| self.holds_party(own, party.kind(), party.value()))
                || self.registry.idmap_sources().iter().any(|(_, source)| {
                    source.role().is_some()
                        && source.role() == other
                        && self.identifiers.get(source.key()) == Some(id.as_str())
                });
        }
        regulatory_kind(other).is_ok_and(|kind| self.identifiers.get(&kind) == Some(id.as_str()))
    }

    /// `planned`'s value as the JSON text a leaf's metadata holds, less the
    /// occurrences the leaf's maps hold where it is a party or a regulatory
    /// group; nothing where none renders.
    fn render<'walk>(
        &self,
        walk: &Walk<'walk>,
        planned: &Planned<'walk>,
        own: Option<&Identifiers>,
        value: &Scalar,
    ) -> Option<SmolStr> {
        let positions = planned.held.get_or_init(|| {
            let counter = planned.counter?;
            let (_, id, other) = PARTY_GROUPS
                .iter()
                .map(|(held, id, role, _)| (*held, *id, *role))
                .chain(REGULATORY_GROUPS)
                .find(|(held, ..)| *held == counter)?;
            let serie = planned.field.dtype().as_serie_type()?;
            let fields = serie.item().fields();
            Some([id, other].map(|tag| child_by_tag(self.registry, fields, false, tag)))
        });
        let held = planned.counter.zip(*positions);
        let rendered = match held {
            Some((counter, positions)) => planned.render_where(walk, value, |occurrence| {
                !occurrence
                    .as_sequence()
                    .is_some_and(|cells| self.holds_occurrence(counter, own, positions, cells))
            }),
            None => planned.shape(walk).render(walk, value),
        };
        rendered.as_ref().and_then(json_text)
    }

    /// Lands one scalar under `key`: set aside to be lifted where the key
    /// names an identifier its value is - read as [`Identifier::from_key`]
    /// reads a key, against the `identifiers` its level declares (the
    /// `FIX:identifiers` of the message's type, or of an occurrence's
    /// component) and, where no dictionary field is `tagged` by it, the
    /// crate's identifier names too: an execution report's
    /// `RefOrderID(1080)` is the `reforderid` it declares, a bridge's
    /// `firm.x.ParentOrderID` `firm.x:parentorderid` - else in `metadata`.
    fn land(
        &self,
        key: &str,
        text: SmolStr,
        identifiers: Option<&str>,
        tagged: bool,
        metadata: &mut Metadata,
        lifted: &mut Lifted,
    ) {
        match inferred_identifier(key, &text, identifiers, tagged).and_then(Result::ok) {
            Some(id) => lifted.push((SmolStr::new(key), text, id)),
            None => {
                metadata.insert(SmolStr::new(key), text);
            }
        }
    }
}

/// The identifier one unmapped scalar names, as [`Holds::land`] reads it:
/// `None` where its key names none or its value states nothing, the type's
/// refusal where the type refuses it.
fn inferred_identifier(
    key: &str,
    value: &str,
    declared: Option<&str>,
    tagged: bool,
) -> Option<crate::Result<Identifier>> {
    let value = value.trim();
    if value.is_empty() || is_null_like(value) {
        return None;
    }
    let names = declared
        .into_iter()
        .flat_map(|names| names.split(','))
        .chain(
            (!tagged)
                .then(IdType::identifier_names)
                .into_iter()
                .flatten(),
        );
    let (src, kind) = Identifier::key_parts(key, names)?;
    Some(Identifier::new(src, kind, value))
}

/// The `FIX:identifiers` a component - a message's definition, an
/// occurrence's item - declares, as the comma-separated names it stores.
fn declared_identifiers(component: &Field) -> Option<&str> {
    component.get_metadata("FIX:identifiers")
}

/// What a walk of one message's row plans a nested child by: the
/// dictionary that types its members, and the tags a leaf reads there.
struct Walk<'a> {
    registry: &'a FixRegistry,
    skipped: &'a dyn Fn(i32) -> bool,
}

impl<'a> Walk<'a> {
    /// The members of one component or occurrence, each resolved once: a
    /// counter beside the group it counts is left out, as [`entries_of`]
    /// leaves it out, and so is a member whose tag or counter the walk
    /// skips. What a member holds is planned only once a value reaches it.
    fn members(&self, fields: &'a [Field]) -> Vec<Planned<'a>> {
        let counters: SmallVec<[i32; 4]> = fields
            .iter()
            .filter(|field| field.dtype().is_nested())
            .filter_map(|field| super::schema::tag_and_counter(self.registry, field).1)
            .collect();
        fields
            .iter()
            .map(|field| {
                let (tag, counter) = super::schema::tag_and_counter(self.registry, field);
                let counts =
                    !field.dtype().is_nested() && tag.is_some_and(|tag| counters.contains(&tag));
                Planned {
                    tag,
                    counter,
                    skipped: counts
                        || tag.is_some_and(self.skipped)
                        || counter.is_some_and(self.skipped),
                    ..Planned::new(field)
                }
            })
            .collect()
    }

    /// How a value of `field` is walked.
    fn shape(&self, field: &'a Field) -> Shape<'a> {
        match field.dtype() {
            DataType::Serie(item) | DataType::LargeSerie(item) => {
                Shape::Occurrences(Box::new(match item.dtype() {
                    DataType::Struct(_) => Shape::Members(self.members(item.fields())),
                    _ => self.shape(item),
                }))
            }
            DataType::Struct(_) => Shape::Members(self.members(field.fields())),
            DataType::Map(_) | DataType::SortedMap(_) => Shape::Map,
            _ => Shape::Leaf,
        }
    }
}

/// One nested child of a row as a walk renders it: whether a leaf reads
/// it, and - planned on the first value that reaches it, and kept for every
/// later one - how its members are reached.
struct Planned<'a> {
    field: &'a Field,
    /// Its own tag, where it states one.
    tag: Option<i32>,
    /// The group it is, by its counter's tag, where it is one.
    counter: Option<i32>,
    /// Where one of its occurrences states the identifier and the role or
    /// type, where it is a group a leaf's identifier maps read; planned on
    /// the first value that reaches it.
    held: OnceCell<Option<[Option<usize>; 2]>>,
    /// Whether it is left out: a counter beside its group, or a tag a leaf
    /// reads.
    skipped: bool,
    shape: OnceCell<Shape<'a>>,
}

/// How a planned child's value is walked.
enum Shape<'a> {
    /// A scalar, rendered as the canonical text it spells.
    Leaf,
    /// A component's members, or an occurrence's.
    Members(Vec<Planned<'a>>),
    /// A repeating group's occurrences, or a list's items, each shaped
    /// alike.
    Occurrences(Box<Shape<'a>>),
    /// A map's entries, each under its key.
    Map,
}

impl<'a> Planned<'a> {
    /// `field`, walked, with nothing planned yet.
    fn new(field: &'a Field) -> Self {
        Self {
            field,
            tag: None,
            counter: None,
            held: OnceCell::new(),
            skipped: false,
            shape: OnceCell::new(),
        }
    }

    /// How this child's value is walked, planned on the first ask.
    fn shape(&self, walk: &Walk<'a>) -> &Shape<'a> {
        self.shape.get_or_init(|| walk.shape(self.field))
    }

    /// `value` rendered, keeping only the occurrences `keep` answers for.
    fn render_where(
        &self,
        walk: &Walk<'a>,
        value: &Scalar,
        keep: impl Fn(&Scalar) -> bool,
    ) -> Option<Scalar> {
        match self.shape(walk) {
            Shape::Occurrences(item) => item.render_occurrences(walk, value, keep),
            shape => shape.render(walk, value),
        }
    }
}

impl<'a> Shape<'a> {
    /// `value` rebuilt as named values: a leaf as the canonical text it
    /// spells - the same text a root scalar lands as, decimals at their
    /// stored scale and codes as their names, so no number is ever a JSON
    /// number - a component or an occurrence as one record of its members
    /// under their names, a group or a list as the sequence of its
    /// occurrences, a map as one record of its text keys. A null, a skipped
    /// member and a record or sequence left empty render nothing.
    fn render(&self, walk: &Walk<'a>, value: &Scalar) -> Option<Scalar> {
        if value.is_null() {
            return None;
        }
        match self {
            Self::Leaf => spelled(value).map(|text| Scalar::from(text.as_str())),
            Self::Members(members) => render_members(walk, members, value.as_sequence()?),
            Self::Occurrences(item) => item.render_occurrences(walk, value, |_| true),
            Self::Map => {
                let entries: Vec<(SmolStr, Scalar)> = value
                    .as_mapping()?
                    .iter()
                    .filter_map(|(key, held)| {
                        Some((SmolStr::new(key.as_str()?), Self::Leaf.render(walk, held)?))
                    })
                    .collect();
                if entries.is_empty() {
                    return None;
                }
                Scalar::from_struct(entries).ok()
            }
        }
    }

    /// The occurrences of `value` `keep` answers for, each rendered as
    /// this shape, in order; nothing where none renders.
    fn render_occurrences(
        &self,
        walk: &Walk<'a>,
        value: &Scalar,
        keep: impl Fn(&Scalar) -> bool,
    ) -> Option<Scalar> {
        let rendered: Vec<Scalar> = value
            .as_serie()?
            .iter()
            .filter(|occurrence| keep(occurrence))
            .filter_map(|occurrence| self.render(walk, &occurrence))
            .collect();
        (!rendered.is_empty()).then(|| Scalar::from_sequence(rendered))
    }
}

/// One component or occurrence `members` planned, as the record of its
/// rendered members under their names; nothing where none renders.
fn render_members<'a>(
    walk: &Walk<'a>,
    members: &[Planned<'a>],
    cells: &[Scalar],
) -> Option<Scalar> {
    let named: Vec<(&str, Scalar)> = members
        .iter()
        .zip(cells)
        .filter(|(member, _)| !member.skipped)
        .filter_map(|(member, cell)| {
            Some((member.field.name(), member.shape(walk).render(walk, cell)?))
        })
        .collect();
    if named.is_empty() {
        return None;
    }
    Scalar::from_struct(named).ok()
}

/// A rendered nested child as the JSON text a leaf's metadata holds: an
/// object's keys sorted, since a record is a sorted map, so one value
/// renders one way.
fn json_text(value: &Scalar) -> Option<SmolStr> {
    crate::json::into_utf8(value).ok().map(SmolStr::from)
}

/// Lands each member of one occurrence `members` planned under its bare
/// name: a scalar as the canonical text it spells - an `Account(1)` the
/// leaf holds left out, and one whose key ends with an identifier name set
/// aside to be lifted - a nested member as the JSON text it renders to,
/// less what the leaf's maps hold, the occurrence's `own` accounts first.
fn land_members<'a>(
    walk: &Walk<'a>,
    members: &[Planned<'a>],
    cells: &[Scalar],
    level: &Level<'_>,
    (metadata, lifted): &mut (Metadata, Lifted),
) {
    let &Level {
        holds,
        own,
        identifiers,
    } = level;
    for (member, cell) in members.iter().zip(cells) {
        if member.skipped || cell.is_null() {
            continue;
        }
        if let Shape::Leaf = member.shape(walk) {
            let Some(text) = spelled(cell) else {
                continue;
            };
            if member.tag != Some(ACCOUNT) || !holds.holds_account(Some(own), &text) {
                let tagged = member.tag.is_some();
                holds.land(
                    member.field.name(),
                    text,
                    identifiers,
                    tagged,
                    metadata,
                    lifted,
                );
            }
            continue;
        }
        if let Some(text) = holds.render(walk, member, Some(own), cell) {
            metadata.insert(SmolStr::new(member.field.name()), text);
        }
    }
}

/// One occurrence's level of a walk: what the leaf holds, the accounts the
/// occurrence states itself, and the identifiers its component declares.
struct Level<'a> {
    holds: &'a Holds<'a>,
    own: &'a Identifiers,
    identifiers: Option<&'a str>,
}

/// The canonical text one scalar spells, which is what a leaf's metadata
/// holds; nothing for a value that spells none.
fn spelled(value: &Scalar) -> Option<SmolStr> {
    crate::string::str_from_value(value)?
        .ok()
        .map(SmolStr::from)
}

/// The metadata a Map value states: every text key under its text value.
fn metadata_of(value: &Scalar) -> BTreeMap<SmolStr, SmolStr> {
    value
        .as_mapping()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| {
                    Some((SmolStr::new(key.as_str()?), SmolStr::new(value.as_str()?)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Report that nothing in the message is reached by `what`.
fn absent(what: impl fmt::Display) -> Error {
    Error::absent("fix value", what)
}

/// The position of the child `name` spells under `parent`: an exact match,
/// else the one child the fold reaches - two children one fold reaches name
/// neither.
fn named_index(parent: &Field, name: &str) -> Option<usize> {
    // A fold drops bytes and never adds one, so an ASCII child shorter than
    // the fold of an ASCII name cannot fold to it, and is passed over with
    // one comparison rather than a walk of both spellings.
    let floor = if name.is_ascii() {
        name.bytes()
            .filter(|byte| !matches!(byte, b'_' | b'-' | b' '))
            .count()
    } else {
        0
    };
    let mut folded = None;
    let mut ambiguous = false;
    for (index, field) in parent.fields().iter().enumerate() {
        let held = field.name();
        if held == name {
            return Some(index);
        }
        if held.len() < floor && held.is_ascii() {
            continue;
        }
        if crate::folds_equal(held, name) {
            ambiguous |= folded.is_some();
            folded = Some(index);
        }
    }
    if ambiguous { None } else { folded }
}

/// Forgets the entry a child at `at` held in a sorted position index.
fn retire(index: &mut Vec<(i32, usize)>, key: Option<i32>, at: usize) {
    if let Some(key) = key
        && let Ok(position) = index.binary_search(&(key, at))
    {
        index.remove(position);
    }
}

/// Records the entry the child at `at` carries in a sorted position index.
fn admit(index: &mut Vec<(i32, usize)>, key: Option<i32>, at: usize) {
    if let Some(key) = key
        && let Err(position) = index.binary_search(&(key, at))
    {
        index.insert(position, (key, at));
    }
}

impl From<FixMsg> for Result<FixMsg> {
    fn from(message: FixMsg) -> Self {
        Ok(message)
    }
}

impl From<FixMsg> for OperationEventFacts {
    /// Moves the message's market data event out without re-reading or
    /// cloning any FIX content.
    fn from(message: FixMsg) -> Self {
        *message.event
    }
}

impl Clone for FixMsg {
    /// The message, its name table and its entries - each derived from the
    /// same row - shared rather than derived again.
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            event: self.event.clone(),
            header: self.header.clone(),
            capture: self.capture.clone(),
            lifted: self.lifted.clone(),
            stale: self.stale,
            stated: self.stated,
            row_stated: self.row_stated,
            text: self.text.clone(),
            metadata: self.metadata.clone(),
            tags: self.tags.clone(),
            named: self.named.clone(),
            groups: self.groups.clone(),
            field: self.field.clone(),
            value: self.value.clone(),
            entries: self.entries.clone(),
            carried: self.carried.clone(),
            derived: self.derived.clone(),
            detected_fx: self.detected_fx,
            anomalies: self.anomalies.clone(),
            arrival_anomalies: self.arrival_anomalies,
        }
    }
}

impl fmt::Debug for FixMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixMsg")
            .field("header", &self.header)
            .field("event", &self.event)
            .field("capture", &self.capture)
            .field("field", &self.field)
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl PartialEq for FixMsg {
    /// Two messages are equal when they state the same facts and the same
    /// row against the same registry - the same `Arc`, or registries that
    /// hold the same fields.
    fn eq(&self, other: &Self) -> bool {
        self.event == other.event
            && self.header == other.header
            && self.capture == other.capture
            && self.text == other.text
            && self.metadata == other.metadata
            && self.field == other.field
            && self.value == other.value
            && self.carried == other.carried
            && (Arc::ptr_eq(&self.registry, &other.registry) || self.registry == other.registry)
    }
}

impl Eq for FixMsg {}

impl Hash for FixMsg {
    /// Hashes the settled code, the row's schema and its value; the
    /// registry is part of equality but not of the hash, which keeps equal
    /// messages hashing alike.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.event.get_currhashcode().hash(state);
        self.field.hash(state);
        self.value.hash(state);
    }
}

impl Element for FixMsg {
    fn get_curruuid(&self) -> Uuid {
        self.event.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.event.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.event.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.event.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.event.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.event.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.event.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.event.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.event.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.event.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.event.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.event.set_srcuuids(sources);
    }

    /// A message's order is its instant.
    fn is_after(&self, other: &Self) -> bool {
        self.event.is_after(&other.event)
    }

    /// The identity settled again from what the message now states.
    fn finalize(&mut self) {
        self.settle();
    }

    /// Another raw observation carrying this complete session-event identity
    /// is the same event and fully merges before predecessor logic. Otherwise
    /// the timed market reading descends from the whole lineage of the one it
    /// follows, the predecessor last. A graph-derived expiry deliberately
    /// follows even though it retains the source delivery identity.
    fn with_previous(self, previous: &Self) -> Option<Self> {
        if self.should_merge_session_event(previous) {
            return self.merge_session_event(previous).ok();
        }
        self.following_operation(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        let mut merged = self.merging_operation_event(other)?;
        merged.fold_anomalies(other);
        Some(merged)
    }
}

impl Event for FixMsg {
    /// Whether this message is an execution: the market data category
    /// `EXEC` and a report of an execution. The execution a parse splits
    /// off an order's, a quote's or a trade's report is one; the report it
    /// was split from is its order's or its quote's, and is not.
    fn is_execution(&self) -> bool {
        self.msgcat() == MarketDataKind::Execution && self.reports_execution()
    }

    /// The timed restatement, and then the market's: a message logged at a
    /// second hop takes the live message's predecessor, place and snapshot
    /// and the step before it - and
    /// what that chain is about where this reading stated none of it.
    fn restating(self, live: &Self) -> Self {
        crate::graph::market::restating_operation(self, live)
    }

    fn get_currunix(&self) -> i64 {
        self.event.get_currunix()
    }

    fn set_currunix(&mut self, unix: i64) {
        self.event.set_currunix(unix);
    }

    fn get_state(&self) -> &State {
        self.event.get_state()
    }

    fn set_state(&mut self, state: State) {
        self.stated |= fact::STATE;
        self.event.set_state(state);
    }

    fn get_seqnum(&self) -> u64 {
        self.event.get_seqnum()
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.event.set_seqnum(seqnum);
    }

    fn get_creaunix(&self) -> Option<i64> {
        self.event.get_creaunix()
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.event.set_creaunix(unix);
    }

    fn get_recdunix(&self) -> Option<i64> {
        self.event.get_recdunix()
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        if unix.is_some() {
            self.row_stated |= ROW_STATED_RECORDING;
        } else {
            self.row_stated &= !ROW_STATED_RECORDING;
        }
        self.event.set_recdunix(unix);
    }

    fn get_exprunix(&self) -> Option<i64> {
        self.event.get_exprunix()
    }

    fn set_exprunix(&mut self, unix: Option<i64>) {
        self.stated |= fact::EXPIRY;
        self.event.set_exprunix(unix);
    }

    fn get_prevunix(&self) -> Option<i64> {
        self.event.get_prevunix()
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.event.set_prevunix(unix);
    }

    fn get_prevuuid(&self) -> Option<Uuid> {
        self.event.get_prevuuid()
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.event.set_prevuuid(uuid);
    }

    fn get_snapunix(&self) -> Option<i64> {
        self.event.get_snapunix()
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.event.set_snapunix(unix);
    }
}

impl Market for FixMsg {
    fn get_price(&self) -> Option<Decimal> {
        self.event.get_price()
    }

    fn set_price(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::PRICE;
        self.event.set_price(px, overwrite);
    }

    fn get_stoppx(&self) -> Option<Decimal> {
        self.event.get_stoppx()
    }

    fn set_stoppx(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::STOPPX;
        self.event.set_stoppx(value, overwrite);
    }

    fn get_currency(&self) -> &Ccy {
        self.event.get_currency()
    }

    fn set_currency(&mut self, currency: Ccy, overwrite: bool) {
        self.stated |= fact::CURRENCY;
        self.event.set_currency(currency, overwrite);
    }

    fn get_quantity(&self) -> Option<Decimal> {
        self.event.get_quantity()
    }

    fn set_quantity(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::QUANTITY;
        self.event.set_quantity(qty, overwrite);
    }

    fn get_displayqty(&self) -> Option<Decimal> {
        self.event.get_displayqty()
    }

    fn set_displayqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::DISPLAYQTY;
        self.event.set_displayqty(value, overwrite);
    }

    fn get_hiddenqty(&self) -> Option<Decimal> {
        self.event.get_hiddenqty()
    }

    fn set_hiddenqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::HIDDENQTY;
        self.event.set_hiddenqty(value, overwrite);
    }

    fn get_unit(&self) -> &Unit {
        self.event.get_unit()
    }

    fn set_unit(&mut self, unit: Unit, overwrite: bool) {
        self.stated |= fact::UNIT;
        self.event.set_unit(unit, overwrite);
    }

    fn get_side(&self) -> Side {
        self.event.get_side()
    }

    fn set_side(&mut self, side: Side, overwrite: bool) {
        self.stated |= fact::SIDE;
        self.event.set_side(side, overwrite);
    }

    /// The message's `msgcat`: an order's, a quote's or an execution's
    /// stores its cross code under its side, and a lifecycle chains a
    /// message with messages of its category only.
    fn marketdatakind(&self) -> MarketDataKind {
        self.event.marketdatakind()
    }

    fn get_marketdatatype(&self) -> crate::MarketDataType {
        self.event.get_marketdatatype()
    }

    fn set_marketdatatype(&mut self, mdtype: crate::MarketDataType, overwrite: bool) {
        self.stated |= fact::MDTYPE;
        self.event.set_marketdatatype(mdtype, overwrite);
    }

    fn get_securityids(&self) -> &Identifiers {
        self.event.get_securityids()
    }

    /// The caller's word: the identifiers become the message's, which no
    /// settle restates from the fields, and the wire stays as the source
    /// sent it. Under `overwrite` the derived overlay goes too.
    fn set_securityids(&mut self, ids: Identifiers, overwrite: bool) -> Result<()> {
        self.stated |= fact::SECURITYIDS;
        if overwrite {
            self.derived = Identifiers::new();
        }
        self.event.set_securityids(ids, overwrite)
    }

    fn insert_securityid(&mut self, id: Identifier) -> Result<bool> {
        if id.src() == &IdSource::Derived {
            return Ok(self.derive_securityid(id.kind(), id.value()));
        }
        // A stated identifier answers before a derived one of its type.
        self.derived.remove_kind(id.kind());
        self.stated |= fact::SECURITYIDS;
        self.event.insert_securityid(id)
    }

    fn remove_securityid(&mut self, src: &IdSource, kind: &IdType) -> Result<bool> {
        self.derived.remove(src, kind);
        if kind == &IdType::Isin {
            // Every derived identifier hangs on the ISIN but the pair, which
            // hangs on the symbol.
            let pair = self.derived.get_identifier(&IdType::Forex).cloned();
            self.derived = Identifiers::new();
            if let Some(pair) = pair {
                self.derived.insert(pair);
            }
        }
        self.stated |= fact::SECURITYIDS;
        self.event.remove_securityid(src, kind)
    }

    fn derive_securityid(&mut self, kind: &IdType, code: &str) -> bool {
        let added = self.event.derive_securityid(kind, code);
        if let Some(id) = added
            .then(|| self.event.get_securityids().get_identifier(kind))
            .flatten()
        {
            self.derived.insert(id.clone());
        }
        added
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.event.get_cficode()
    }

    fn set_cficode(&mut self, cficode: Option<Cfi>, overwrite: bool) {
        self.stated |= fact::CFI;
        self.event.set_cficode(cficode, overwrite);
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.event.get_miccode()
    }

    fn set_miccode(&mut self, miccode: Option<Mic>, overwrite: bool) {
        self.stated |= fact::MIC;
        self.event.set_miccode(miccode, overwrite);
    }

    fn get_execunix(&self) -> Option<i64> {
        self.event.get_execunix()
    }

    /// An execution clock set here is the caller's word, which a settle of
    /// the clock alone leaves standing.
    fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool) {
        self.stated |= fact::EXECUTION;
        self.event.set_execunix(unix, overwrite);
        if self.event.get_execunix().is_some() {
            self.row_stated |= ROW_STATED_EXECUTION;
        } else {
            self.row_stated &= !ROW_STATED_EXECUTION;
        }
    }

    fn get_lastpx(&self) -> Option<Decimal> {
        self.event.get_lastpx()
    }

    fn set_lastpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::LASTPX;
        self.event.set_lastpx(px, overwrite);
    }

    fn get_lastqty(&self) -> Option<Decimal> {
        self.event.get_lastqty()
    }

    fn set_lastqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::FILLS;
        self.event.set_lastqty(qty, overwrite);
    }

    fn get_avgpx(&self) -> Option<Decimal> {
        self.event.get_avgpx()
    }

    fn set_avgpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::FILLS;
        self.event.set_avgpx(px, overwrite);
    }

    fn get_cumqty(&self) -> Option<Decimal> {
        self.event.get_cumqty()
    }

    fn set_cumqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::FILLS;
        self.event.set_cumqty(qty, overwrite);
    }

    fn get_leavesqty(&self) -> Option<Decimal> {
        self.event.get_leavesqty()
    }

    fn set_leavesqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::FILLS;
        self.event.set_leavesqty(qty, overwrite);
    }

    fn get_cxlqty(&self) -> Option<Decimal> {
        self.event.get_cxlqty()
    }

    fn set_cxlqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::CXLQTY;
        self.event.set_cxlqty(value, overwrite);
    }

    fn get_prevpx(&self) -> Option<Decimal> {
        self.event.get_prevpx()
    }

    fn set_prevpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::PREVPX;
        self.event.set_prevpx(px, overwrite);
    }

    fn get_prevqty(&self) -> Option<Decimal> {
        self.event.get_prevqty()
    }

    fn set_prevqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.event.set_prevqty(qty, overwrite);
    }

    fn get_spotrate(&self) -> Option<Decimal> {
        self.event.get_spotrate()
    }

    fn set_spotrate(&mut self, rate: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::LASTPX;
        self.event.set_spotrate(rate, overwrite);
    }

    fn get_forwardpoints(&self) -> Option<Decimal> {
        self.event.get_forwardpoints()
    }

    fn set_forwardpoints(&mut self, points: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::LASTPX;
        self.event.set_forwardpoints(points, overwrite);
    }

    fn get_ticker(&self) -> Option<&str> {
        self.event.get_ticker()
    }

    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool) {
        self.stated |= fact::TICKER;
        self.event.set_ticker(ticker, overwrite);
    }

    fn get_metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn set_metadata(&mut self, metadata: Option<Metadata>, overwrite: bool) {
        match (metadata, overwrite) {
            (metadata, true) => self.metadata = metadata.unwrap_or_default(),
            (Some(stated), false) => {
                for (key, value) in stated {
                    self.metadata.entry(key).or_insert(value);
                }
            }
            (None, false) => {}
        }
    }

    fn get_fxrates(&self) -> &FxRates {
        self.event.get_fxrates()
    }

    fn set_fxrates(&mut self, rates: FxRates, overwrite: bool) {
        self.event.set_fxrates(rates, overwrite);
    }

    fn get_bidpx(&self) -> Option<crate::Decimal> {
        self.event.get_bidpx()
    }

    fn set_bidpx(&mut self, px: Option<crate::Decimal>, overwrite: bool) {
        self.stated |= fact::BIDASK;
        self.event.set_bidpx(px, overwrite);
    }

    fn get_bidqty(&self) -> Option<crate::Decimal> {
        self.event.get_bidqty()
    }

    fn set_bidqty(&mut self, qty: Option<crate::Decimal>, overwrite: bool) {
        self.stated |= fact::BIDASK;
        self.event.set_bidqty(qty, overwrite);
    }

    fn get_bidccy(&self) -> Option<&crate::Ccy> {
        self.event.get_bidccy()
    }

    fn set_bidccy(&mut self, ccy: Option<crate::Ccy>, overwrite: bool) {
        self.stated |= fact::BIDASK;
        self.event.set_bidccy(ccy, overwrite);
    }

    fn get_askpx(&self) -> Option<crate::Decimal> {
        self.event.get_askpx()
    }

    fn set_askpx(&mut self, px: Option<crate::Decimal>, overwrite: bool) {
        self.stated |= fact::BIDASK;
        self.event.set_askpx(px, overwrite);
    }

    fn get_askqty(&self) -> Option<crate::Decimal> {
        self.event.get_askqty()
    }

    fn set_askqty(&mut self, qty: Option<crate::Decimal>, overwrite: bool) {
        self.stated |= fact::BIDASK;
        self.event.set_askqty(qty, overwrite);
    }

    fn get_askccy(&self) -> Option<&crate::Ccy> {
        self.event.get_askccy()
    }

    fn set_askccy(&mut self, ccy: Option<crate::Ccy>, overwrite: bool) {
        self.stated |= fact::BIDASK;
        self.event.set_askccy(ccy, overwrite);
    }
}

impl Operation for FixMsg {
    fn get_ordqty(&self) -> Option<Decimal> {
        self.event.get_ordqty()
    }

    fn set_ordqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.stated |= fact::ORDQTY;
        self.event.set_ordqty(qty, overwrite);
    }

    fn get_timeinforce(&self) -> Option<&TimeInForce> {
        self.event.get_timeinforce()
    }

    fn set_timeinforce(&mut self, tif: Option<TimeInForce>, overwrite: bool) {
        self.stated |= fact::TIF;
        self.event.set_timeinforce(tif, overwrite);
    }

    fn get_tradable(&self) -> Option<bool> {
        self.event.get_tradable()
    }

    fn set_tradable(&mut self, tradable: Option<bool>, overwrite: bool) {
        self.stated |= fact::TRADABLE;
        self.event.set_tradable(tradable, overwrite);
    }

    fn get_identifiers(&self) -> &Identifiers {
        self.event.get_identifiers()
    }

    /// The caller's word: the identifiers become the message's, which no
    /// settle restates from the fields, and the wire stays as the source
    /// sent it.
    fn set_identifiers(&mut self, ids: Identifiers, overwrite: bool) -> Result<()> {
        self.stated |= fact::IDENTIFIERS;
        self.event.set_identifiers(ids, overwrite)
    }

    fn insert_identifier(&mut self, id: Identifier) -> Result<bool> {
        self.stated |= fact::IDENTIFIERS;
        self.event.insert_identifier(id)
    }

    fn remove_identifier(&mut self, src: &IdSource, kind: &IdType) -> Result<bool> {
        self.stated |= fact::IDENTIFIERS;
        self.event.remove_identifier(src, kind)
    }

    fn get_partyids(&self) -> &Identifiers {
        self.event.get_partyids()
    }

    /// The caller's word, as [`Operation::set_identifiers`] is.
    fn set_partyids(&mut self, partyids: Identifiers, overwrite: bool) -> Result<()> {
        self.stated |= fact::PARTYIDS;
        self.event.set_partyids(partyids, overwrite)
    }

    fn insert_partyid(&mut self, partyid: Identifier) -> Result<bool> {
        self.stated |= fact::PARTYIDS;
        self.event.insert_partyid(partyid)
    }

    fn remove_partyid(&mut self, src: &IdSource, kind: &IdType) -> Result<bool> {
        self.stated |= fact::PARTYIDS;
        self.event.remove_partyid(src, kind)
    }

    /// The registry's own answer: the `FIX:parents` its fields state, else
    /// the names' ([`FixRegistry::parents_of`]).
    fn parents_of(&self, base: &IdType) -> std::borrow::Cow<'_, [IdType]> {
        self.registry.parents_of(base)
    }

    /// The registry's own answer ([`FixRegistry::parent_of`]).
    fn parent_of(&self, kind: &IdType) -> Option<(IdType, usize)> {
        self.registry.parent_of(kind)
    }

    /// The registry's own answer: an identifier whose type's `FIX:idmap`
    /// entry follows.
    fn is_followed_identifier(&self, id: &Identifier) -> bool {
        self.registry.idmap_sources().iter().any(|(_, source)| {
            source.follows()
                && source.map() == FixIdMapKind::Identifiers
                && source.key() == id.kind()
        })
    }
}

impl FixMsg {
    /// Each base a parent identifier names and the message does not state,
    /// filled from its nearest parent by the registry's lists once it
    /// settles, after every enrichment ([`Identifiers::fill_parents`]).
    fn fill_parents(&mut self) {
        let parent_of = |kind: &IdType| self.registry.parent_of(kind);
        let held = self.event.get_identifiers();
        // A message stating every base its parents name - most of them -
        // copies no set to learn so.
        if !held.iter().any(|id| {
            parent_of(id.kind()).is_some_and(|(base, _)| held.get_from(id.src(), &base).is_none())
        }) {
            return;
        }
        let mut identifiers = held.clone();
        if identifiers.fill_parents(parent_of) {
            let _ = self.event.set_identifiers(identifiers, true);
        }
    }

    /// Whether the row stated the currency pair, which FX detection leaves.
    pub(super) const fn states_forex(&self) -> bool {
        self.row_stated & ROW_STATED_FOREX != 0
    }

    /// Which cells FX detection wrote, one bit per cell.
    pub(super) const fn detected_fx(&self) -> u8 {
        self.detected_fx
    }

    /// Records which cells FX detection wrote.
    pub(super) fn set_detected_fx(&mut self, cells: u8) {
        self.detected_fx = cells;
    }

    /// Whether the derived overlay holds a currency pair.
    pub(super) fn derives_pair(&self) -> bool {
        self.derived.contains_kind(&IdType::Forex)
    }

    /// Whether the derived overlay holds exactly the pair `code`.
    pub(super) fn derives_pair_of(&self, code: &str) -> bool {
        self.derived.get(&IdType::Forex) == Some(code)
    }

    /// Replaces the derived currency pair, or takes it back: in the overlay,
    /// which every settle re-applies, and on the event, where a pair the
    /// overlay derived stands until then.
    pub(super) fn set_derived_pair(&mut self, pair: Option<&str>) {
        // What a pair implies - the identifiers, and no one market - is
        // stated again at the next settle.
        self.stale |= fact::SECURITYIDS | fact::MIC;
        self.derived.remove_kind(&IdType::Forex);
        let _ = self
            .event
            .remove_securityid(&IdSource::Derived, &IdType::Forex);
        if let Some(pair) = pair {
            self.derive_securityid(&IdType::Forex, pair);
        }
    }
}

/// Whether `text` is one of the spellings a wire uses for no value at all.
fn is_null_like(text: &str) -> bool {
    ["NULL", "NONE", "N/A", "[N/A]", "NA", "-"]
        .iter()
        .any(|null| text.eq_ignore_ascii_case(null))
}

/// A cell's text: a string as it is, a code as its spelling, an integer as
/// its digits; trimmed, and none where empty or of another shape.
pub(super) fn scalar_text(value: &Scalar) -> Option<SmolStr> {
    if let Some(text) = value.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| SmolStr::new(text));
    }
    value.as_i64().map(|held| format_smolstr!("{held}"))
}
