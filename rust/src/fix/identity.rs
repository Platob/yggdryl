//! The typed facts a message holds beside its row, and how they are read
//! from and written to the columns that state them.

use std::collections::BTreeMap;

use smol_str::SmolStr;

use crate::graph::{Element, Event, MarketElement, MarketEventData};
use crate::types::{Bloomberg, Cfi, Currency, Cusip, Decimal, Isin, Mic, Sedol, Side, State};
use crate::{DataType, Error, Field, Result, Scalar, TimeUnit, Timezone};

use super::schema::CLOCK_DATATYPE;
use super::{
    ASKCURRENCY_TAG_NAME, ASKUNIT_TAG_NAME, BIDCURRENCY_TAG_NAME, BIDUNIT_TAG_NAME,
    BLOOMBERGCODE_TAG_NAME, CREATUNIX_TAG_NAME, CROSSCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME,
    CROSSUUID_TAG_NAME, CURRUUID_TAG_NAME, CUSIPCODE_TAG_NAME, EXPIRUNIX_TAG_NAME, FixRegistry,
    HASHCODE_TAG_NAME, IDENTIFIERS_TAG_NAME, ISINCODE_TAG_NAME, MICCODE_TAG_NAME,
    MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGSESSIONID_TAG_NAME, PARENTUUIDS_TAG_NAME,
    PLUGINID_TAG_NAME, PREVPX_TAG_NAME, PREVQTY_TAG_NAME, PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME,
    PX_TAG_NAME, QTY_TAG_NAME, RECORDEDAT_TAG_NAME, SEDOLCODE_TAG_NAME, SEQNUM_TAG_NAME,
    SNAPUNIX_TAG_NAME, SOURCEURL_TAG_NAME, STATE_TAG_NAME, SYMBOLTICKER_TAG_NAME,
    TRADABLE_TAG_NAME, UNIT_TAG_NAME, UNIX_TAG_NAME,
};

/// The standard header facts every message holds typed, beside its row.
///
/// What FIX puts in front of every message and every consumer reads first:
/// the version the message says it speaks, the type it is, who sent it to
/// whom, its place in the session and when it was sent. Each is read and
/// written here rather than looked up in the row, and the row holds none
/// of them: a message is its header, its event and its content, once each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixHeader {
    beginstring: SmolStr,
    msgtype: SmolStr,
    sendercompid: Option<SmolStr>,
    targetcompid: Option<SmolStr>,
    msgseqnum: Option<u64>,
    sendingtime: i64,
    /// Whether the message stated its sending time, or the intake settled
    /// it: only a stated one goes back on the wire.
    stated_sendingtime: bool,
    possdupflag: Option<bool>,
    msgdirection: Option<SmolStr>,
}

impl FixHeader {
    /// The header of a message stating nothing yet: no version, an
    /// `unknown` type, nobody sending it to nobody, sent at the epoch.
    pub(super) fn unknown() -> Self {
        Self {
            beginstring: SmolStr::new_static(""),
            msgtype: SmolStr::new_static(""),
            sendercompid: None,
            targetcompid: None,
            msgseqnum: None,
            sendingtime: 0,
            stated_sendingtime: false,
            possdupflag: None,
            msgdirection: None,
        }
    }

    /// `BeginString(8)`: the FIX the message says it speaks, `FIX.4.4`;
    /// empty where it states none.
    #[must_use]
    pub fn beginstring(&self) -> &str {
        &self.beginstring
    }

    /// `MsgType(35)`: the message's wire code, `D`; empty where it states
    /// none.
    #[must_use]
    pub fn msgtype(&self) -> &str {
        &self.msgtype
    }

    /// `SenderCompID(49)`, where stated.
    #[must_use]
    pub fn sendercompid(&self) -> Option<&str> {
        self.sendercompid.as_deref()
    }

    /// `TargetCompID(56)`, where stated.
    #[must_use]
    pub fn targetcompid(&self) -> Option<&str> {
        self.targetcompid.as_deref()
    }

    /// `MsgSeqNum(34)`, where stated.
    #[must_use]
    pub const fn msgseqnum(&self) -> Option<u64> {
        self.msgseqnum
    }

    /// `SendingTime(52)` as nanoseconds since the Unix epoch, UTC: what the
    /// message stated, else the clock the intake settled.
    #[must_use]
    pub const fn sendingtime(&self) -> i64 {
        self.sendingtime
    }

    /// `PossDupFlag(43)`, where stated.
    #[must_use]
    pub const fn possdupflag(&self) -> Option<bool> {
        self.possdupflag
    }

    /// `MsgDirection(385)` as the code the dictionary's set spells it,
    /// where the line or the caller stated which way the message moved.
    #[must_use]
    pub fn msgdirection(&self) -> Option<&str> {
        self.msgdirection.as_deref()
    }

    /// The value one header tag holds, as the raw value its column types.
    pub(super) fn fact(&self, tag: i32) -> Option<Scalar> {
        let text = |held: &str| (!held.is_empty()).then(|| Scalar::from(held));
        match tag {
            8 => text(&self.beginstring),
            35 => text(&self.msgtype),
            49 => self.sendercompid.as_deref().and_then(text),
            56 => self.targetcompid.as_deref().and_then(text),
            34 => self.msgseqnum.map(Scalar::from),
            52 => Scalar::datetime64(self.sendingtime, TimeUnit::Nanosecond, Timezone::UTC).ok(),
            43 => self.possdupflag.map(Scalar::from),
            tag if tag == MSGDIRECTION_TAG_NAME.0 => self.msgdirection.as_deref().and_then(text),
            _ => None,
        }
    }

    /// Records what one header tag states; a null clears it. Whether the
    /// tag is a header tag at all.
    fn record(&mut self, tag: i32, value: &Scalar) -> bool {
        let text = || {
            value
                .as_str()
                .map(SmolStr::new)
                .filter(|held| !held.is_empty())
        };
        match tag {
            8 => self.beginstring = text().unwrap_or_default(),
            35 => self.msgtype = text().unwrap_or_default(),
            49 => self.sendercompid = text(),
            56 => self.targetcompid = text(),
            34 => self.msgseqnum = value.as_u64(),
            52 => {
                if let Some(unix) = value.temporal_count_at(TimeUnit::Nanosecond) {
                    self.sendingtime = unix;
                    self.stated_sendingtime = true;
                }
            }
            43 => {
                self.possdupflag = value.as_bool().or_else(|| match value.as_str() {
                    Some("Y" | "y") => Some(true),
                    Some("N" | "n") => Some(false),
                    _ => None,
                });
            }
            tag if tag == MSGDIRECTION_TAG_NAME.0 => self.msgdirection = text(),
            _ => return false,
        }
        true
    }

    /// Records when the message was sent, as the intake settled it.
    pub(super) fn set_sendingtime(&mut self, unix: i64) {
        self.sendingtime = unix;
    }

    /// Whether the message stated its sending time itself.
    #[must_use]
    pub const fn stated_sendingtime(&self) -> bool {
        self.stated_sendingtime
    }
}

/// What a capture states about the line a message was read from, typed.
///
/// Facts about the capture and not about the message: where the line was
/// read from, when the capture recorded it, and what a bridge's own row
/// header says about the line it wrote - the plugin, the message context
/// and the session instance. None of them is FIX and none is content: the
/// same message read out of a second copy of the log is the same message,
/// so nothing here reaches the code the message digests to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixCapture {
    sourceurl: Option<std::sync::Arc<crate::Url>>,
    recordedat: Option<i64>,
    pluginid: Option<SmolStr>,
    msgctxid: Option<SmolStr>,
    msgsessionid: Option<SmolStr>,
}

impl FixCapture {
    /// The object the line was read from, where the capture named it.
    #[must_use]
    pub fn sourceurl(&self) -> Option<&crate::Url> {
        self.sourceurl.as_deref()
    }

    /// When the capture recorded the line, nanoseconds since the Unix
    /// epoch, UTC, where it dated it.
    #[must_use]
    pub const fn recordedat(&self) -> Option<i64> {
        self.recordedat
    }

    /// The plugin that logged the line inside a bridge, as the bridge names
    /// it.
    #[must_use]
    pub fn pluginid(&self) -> Option<&str> {
        self.pluginid.as_deref()
    }

    /// The message context a bridge handled the line in.
    #[must_use]
    pub fn msgctxid(&self) -> Option<&str> {
        self.msgctxid.as_deref()
    }

    /// The session instance a bridge handled the line on.
    #[must_use]
    pub fn msgsessionid(&self) -> Option<&str> {
        self.msgsessionid.as_deref()
    }

    fn fact(&self, tag: i32) -> Option<Scalar> {
        let text = |held: &Option<SmolStr>| held.as_deref().map(Scalar::from);
        let is = |held: (i32, &str)| held.0 == tag;
        if is(SOURCEURL_TAG_NAME) {
            self.sourceurl.clone().map(Scalar::Url)
        } else if is(RECORDEDAT_TAG_NAME) {
            self.recordedat
                .and_then(|unix| Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok())
        } else if is(PLUGINID_TAG_NAME) {
            text(&self.pluginid)
        } else if is(MSGCTXID_TAG_NAME) {
            text(&self.msgctxid)
        } else if is(MSGSESSIONID_TAG_NAME) {
            text(&self.msgsessionid)
        } else {
            None
        }
    }

    fn record(&mut self, tag: i32, value: &Scalar) -> bool {
        let text = || {
            value
                .as_str()
                .map(SmolStr::new)
                .filter(|held| !held.is_empty())
        };
        let is = |held: (i32, &str)| held.0 == tag;
        if is(SOURCEURL_TAG_NAME) {
            self.sourceurl = match value {
                Scalar::Url(url) => Some(std::sync::Arc::clone(url)),
                other => other
                    .as_str()
                    .and_then(|text| text.parse::<crate::Url>().ok())
                    .map(std::sync::Arc::new),
            };
        } else if is(RECORDEDAT_TAG_NAME) {
            self.recordedat = value.temporal_count_at(TimeUnit::Nanosecond);
        } else if is(PLUGINID_TAG_NAME) {
            self.pluginid = text();
        } else if is(MSGCTXID_TAG_NAME) {
            self.msgctxid = text();
        } else if is(MSGSESSIONID_TAG_NAME) {
            self.msgsessionid = text();
        } else {
            return false;
        }
        true
    }
}

/// The tags the message's cross code is read from, strongest first: the
/// venue's `OrderID(37)`, the client's `ClOrdID(11)` and the one it
/// replaced, `OrigClOrdID(41)`, then a quote's `QuoteID(117)`, the request
/// it answers, `QuoteReqID(131)`, and a market data request's
/// `MDReqID(262)`.
pub(super) const CROSS_TAGS: [i32; 6] = [37, 11, 41, 117, 131, 262];

/// FIX's own tags the event holds beside the crate's columns, in tag
/// order: the price and the quantity a message is about, the trade and the
/// progress it reports, how long it stands, the currency, the side, the
/// classification and the four lane numbers.
///
/// Each is held *on* the event rather than beside it, so the row carries
/// one column per fact instead of a crate column and the field it was
/// lifted from, and this is the order the wire re-emits them in.
pub(super) const OWN_EVENT_TAGS: [i32; 16] = [
    AVGPX_TAG,
    CUMQTY_TAG,
    15,
    LASTPX_TAG,
    LASTQTY_TAG,
    ORDERQTY_TAG,
    PRICE_TAG,
    QUANTITY_TAG,
    54,
    TIMEINFORCE_TAG,
    132,
    133,
    134,
    135,
    LEAVESQTY_TAG,
    super::cfi::CFICODE_TAG,
];

/// FIX's own fields for the market facts the event holds.
///
/// `Quantity(53)` is the newer spelling of `OrderQty(38)` and lands in the
/// same fact; the wire re-emits it as `38`, which is the restatement this
/// crate makes of every retired spelling.
pub(super) const PRICE_TAG: i32 = 44;
pub(super) const ORDERQTY_TAG: i32 = 38;
pub(super) const QUANTITY_TAG: i32 = 53;
pub(super) const LASTPX_TAG: i32 = 31;
pub(super) const LASTQTY_TAG: i32 = 32;
pub(super) const AVGPX_TAG: i32 = 6;
pub(super) const CUMQTY_TAG: i32 = 14;
pub(super) const LEAVESQTY_TAG: i32 = 151;
pub(super) const TIMEINFORCE_TAG: i32 = 59;

/// The standard header tags the header holds.
const HEADER_TAGS: [i32; 8] = [8, 35, 49, 56, 34, 52, 43, MSGDIRECTION_TAG_NAME.0];

/// FIX's `Text(58)`: the free text a message carries, which the message
/// holds typed beside its event.
pub(super) const TEXT_TAG: i32 = 58;

/// Whether a tag names a fact the message holds typed rather than in its
/// row: one of the crate's own columns, a standard header tag, or one of
/// FIX's own event tags.
pub(super) fn is_typed_tag(tag: i32) -> bool {
    super::is_crate_tag(tag)
        || HEADER_TAGS.contains(&tag)
        || OWN_EVENT_TAGS.contains(&tag)
        || tag == TEXT_TAG
}

/// The three typed holders of a message, read and written by tag.
pub(super) struct Typed<'msg> {
    pub(super) event: &'msg MarketEventData,
    pub(super) header: &'msg FixHeader,
    pub(super) capture: &'msg FixCapture,
}

impl Typed<'_> {
    /// What the message states under one typed tag, as the raw value the
    /// tag's column types, or nothing where it states no fact.
    pub(super) fn fact(&self, tag: i32) -> Option<Scalar> {
        if super::is_crate_tag(tag) {
            event_fact(self.event, tag).or_else(|| self.capture.fact(tag))
        } else if HEADER_TAGS.contains(&tag) {
            self.header.fact(tag)
        } else {
            event_fact(self.event, tag)
        }
    }
}

/// Records what one typed tag states on the holder that owns it; a null
/// clears the fact. Whether the tag is a typed tag at all.
pub(super) fn record(
    event: &mut MarketEventData,
    header: &mut FixHeader,
    capture: &mut FixCapture,
    tag: i32,
    value: &Scalar,
) -> bool {
    if HEADER_TAGS.contains(&tag) {
        return header.record(tag, value);
    }
    if capture.record(tag, value) {
        return true;
    }
    record_event(event, tag, value)
}

/// Records what one event column states on the event, typed through the
/// traits; a value the fact's type refuses is silence and a null clears
/// the fact. Whether the tag is one the event holds.
pub(super) fn record_event(event: &mut MarketEventData, tag: i32, value: &Scalar) -> bool {
    let instant = || value.temporal_count_at(TimeUnit::Nanosecond);
    let text = || value.as_str().filter(|held| !held.is_empty());
    let decimal = || Decimal::from_scalar(value);
    let currency = || text().and_then(|held| Currency::new(held).ok());
    let is = |held: (i32, &str)| held.0 == tag;
    if is(UNIX_TAG_NAME) {
        if let Some(unix) = instant() {
            event.set_unix(unix);
        }
    } else if is(CREATUNIX_TAG_NAME) {
        event.set_creatunix(instant());
    } else if is(EXPIRUNIX_TAG_NAME) {
        event.set_expirunix(instant());
    } else if is(PREVUNIX_TAG_NAME) {
        event.set_prevunix(instant());
    } else if is(SNAPUNIX_TAG_NAME) {
        event.set_snapunix(instant());
    } else if is(PREVUUID_TAG_NAME) {
        event.set_prevuuid(match value {
            Scalar::Uuid(uuid) => Some(*uuid),
            _ => None,
        });
    } else if is(CURRUUID_TAG_NAME) {
        if let Scalar::Uuid(uuid) = value {
            event.set_curruuid(*uuid);
        }
    } else if is(CROSSUUID_TAG_NAME) {
        if let Scalar::Uuid(uuid) = value {
            event.set_crossuuid(*uuid);
        }
    } else if is(HASHCODE_TAG_NAME) {
        if let Some(code) = value.as_u64() {
            event.set_hashcode(code);
        }
    } else if is(CROSSHASHCODE_TAG_NAME) {
        if let Some(code) = value.as_u64() {
            event.set_crosshashcode(code);
        }
    } else if is(CROSSCODE_TAG_NAME) {
        event.set_crosscode(text().map(str::to_owned).unwrap_or_default());
    } else if is(STATE_TAG_NAME) {
        event.set_state(
            text()
                .and_then(|held| State::read(held).ok())
                .unwrap_or_else(State::unknown),
        );
    } else if is(SEQNUM_TAG_NAME) {
        event.set_seqnum(value.as_u64().unwrap_or(0));
    } else if is(IDENTIFIERS_TAG_NAME) {
        let identifiers: BTreeMap<String, String> = value
            .as_mapping()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|(scheme, identifier)| {
                        Some((scheme.as_str()?.to_owned(), identifier.as_str()?.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        event.set_identifiers(identifiers);
    } else if is(PARENTUUIDS_TAG_NAME) {
        event.set_parentuuids(
            value
                .as_sequence()
                .map(|parents| {
                    parents
                        .iter()
                        .filter_map(|parent| match parent {
                            Scalar::Uuid(uuid) => Some(*uuid),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        );
    } else if is(PX_TAG_NAME) {
        event.set_px(decimal().unwrap_or(Decimal::ZERO));
    } else if is(PREVPX_TAG_NAME) {
        event.set_prevpx(decimal());
    } else if is(PREVQTY_TAG_NAME) {
        event.set_prevqty(decimal());
    } else if is(TRADABLE_TAG_NAME) {
        event.set_tradable(value.as_bool());
    } else if is(SYMBOLTICKER_TAG_NAME) {
        event.set_symbolticker(text().map(str::to_owned));
    } else if is(QTY_TAG_NAME) {
        event.set_qty(decimal().unwrap_or(Decimal::ZERO));
    } else if is(UNIT_TAG_NAME) {
        event.set_unit(text().map(str::to_owned).unwrap_or_default());
    } else if is(BIDUNIT_TAG_NAME) {
        event.set_bidunit(text().map(str::to_owned));
    } else if is(ASKUNIT_TAG_NAME) {
        event.set_askunit(text().map(str::to_owned));
    } else if is(BIDCURRENCY_TAG_NAME) {
        event.set_bidcurrency(currency());
    } else if is(ASKCURRENCY_TAG_NAME) {
        event.set_askcurrency(currency());
    } else if is(ISINCODE_TAG_NAME) {
        event.set_isincode(text().and_then(|held| Isin::new(held).ok()));
    } else if is(CUSIPCODE_TAG_NAME) {
        event.set_cusipcode(text().and_then(|held| Cusip::new(held).ok()));
    } else if is(SEDOLCODE_TAG_NAME) {
        event.set_sedolcode(text().and_then(|held| Sedol::new(held).ok()));
    } else if is(BLOOMBERGCODE_TAG_NAME) {
        event.set_bloombergcode(text().and_then(|held| Bloomberg::new(held).ok()));
    } else if is(MICCODE_TAG_NAME) {
        event.set_miccode(text().and_then(|held| Mic::new(held).ok()));
    } else if tag == 15 {
        event.set_currency(currency().unwrap_or_else(Currency::none));
    } else if tag == 54 {
        event.set_side(
            text()
                .and_then(|held| Side::read(held).ok())
                .unwrap_or_else(Side::unknown),
        );
    } else if tag == super::cfi::CFICODE_TAG {
        event.set_cficode(text().and_then(|held| Cfi::new(held).ok()));
    } else if tag == PRICE_TAG {
        event.set_px(decimal().unwrap_or(Decimal::ZERO));
    } else if tag == ORDERQTY_TAG || tag == QUANTITY_TAG {
        event.set_qty(decimal().unwrap_or(Decimal::ZERO));
    } else if tag == LASTPX_TAG {
        event.set_lastpx(decimal());
    } else if tag == LASTQTY_TAG {
        event.set_lastqty(decimal());
    } else if tag == AVGPX_TAG {
        event.set_avgpx(decimal());
    } else if tag == CUMQTY_TAG {
        event.set_cumqty(decimal());
    } else if tag == LEAVESQTY_TAG {
        event.set_leavesqty(decimal());
    } else if tag == TIMEINFORCE_TAG {
        event.set_tif(text().map(str::to_owned));
    } else if tag == 132 {
        event.set_bidpx(decimal());
    } else if tag == 133 {
        event.set_askpx(decimal());
    } else if tag == 134 {
        event.set_bidqty(decimal());
    } else if tag == 135 {
        event.set_askqty(decimal());
    } else {
        return false;
    }
    true
}

/// What the event states for one event column, as the raw value the
/// column's field types, or nothing where it states no fact: an unknown
/// state or side, a `XXX` currency, a price or a quantity of nothing, an
/// empty name, an absent instant, identity or code.
pub(super) fn event_fact(event: &MarketEventData, tag: i32) -> Option<Scalar> {
    let instant = |unix: i64| Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok();
    let stated = |unix: Option<i64>| unix.and_then(instant);
    let text = |held: &str| (!held.is_empty()).then(|| Scalar::from(held));
    let number = |held: Decimal| (!held.is_zero()).then(|| Scalar::from(held));
    let is = |held: (i32, &str)| held.0 == tag;
    if is(UNIX_TAG_NAME) {
        instant(event.get_unix())
    } else if is(CREATUNIX_TAG_NAME) {
        stated(event.get_creatunix())
    } else if is(EXPIRUNIX_TAG_NAME) {
        stated(event.get_expirunix())
    } else if is(PREVUNIX_TAG_NAME) {
        stated(event.get_prevunix())
    } else if is(SNAPUNIX_TAG_NAME) {
        stated(event.get_snapunix())
    } else if is(PREVUUID_TAG_NAME) {
        event.get_prevuuid().map(Scalar::Uuid)
    } else if is(CURRUUID_TAG_NAME) {
        Some(Scalar::Uuid(event.get_curruuid()))
    } else if is(CROSSUUID_TAG_NAME) {
        Some(Scalar::Uuid(event.get_crossuuid()))
    } else if is(HASHCODE_TAG_NAME) {
        Some(Scalar::from(event.get_hashcode()))
    } else if is(CROSSHASHCODE_TAG_NAME) {
        Some(Scalar::from(event.get_crosshashcode()))
    } else if is(CROSSCODE_TAG_NAME) {
        text(event.get_crosscode())
    } else if is(STATE_TAG_NAME) {
        (event.get_state() != &State::unknown()).then(|| Scalar::from(event.get_state().as_str()))
    } else if is(SEQNUM_TAG_NAME) {
        (event.get_seqnum() != 0).then(|| Scalar::from(event.get_seqnum()))
    } else if is(IDENTIFIERS_TAG_NAME) {
        let identifiers = event.get_identifiers();
        (!identifiers.is_empty()).then(|| {
            Scalar::from_mapping(identifiers.iter().map(|(scheme, identifier)| {
                (
                    Scalar::from(scheme.as_str()),
                    Scalar::from(identifier.as_str()),
                )
            }))
            .ok()
        })?
    } else if is(PARENTUUIDS_TAG_NAME) {
        let parents = event.get_parentuuids();
        (!parents.is_empty())
            .then(|| Scalar::from_sequence(parents.iter().copied().map(Scalar::Uuid)))
    } else if is(PX_TAG_NAME) {
        number(event.get_px())
    } else if is(PREVPX_TAG_NAME) {
        event.get_prevpx().map(Scalar::from)
    } else if is(PREVQTY_TAG_NAME) {
        event.get_prevqty().map(Scalar::from)
    } else if is(TRADABLE_TAG_NAME) {
        event.get_tradable().map(Scalar::from)
    } else if is(SYMBOLTICKER_TAG_NAME) {
        event.get_symbolticker().and_then(text)
    } else if is(QTY_TAG_NAME) {
        number(event.get_qty())
    } else if is(UNIT_TAG_NAME) {
        text(event.get_unit())
    } else if is(BIDUNIT_TAG_NAME) {
        event.get_bidunit().and_then(text)
    } else if is(ASKUNIT_TAG_NAME) {
        event.get_askunit().and_then(text)
    } else if is(BIDCURRENCY_TAG_NAME) {
        event
            .get_bidcurrency()
            .map(|held| Scalar::from(held.as_str()))
    } else if is(ASKCURRENCY_TAG_NAME) {
        event
            .get_askcurrency()
            .map(|held| Scalar::from(held.as_str()))
    } else if is(ISINCODE_TAG_NAME) {
        event.get_isincode().map(|held| Scalar::from(held.as_str()))
    } else if is(CUSIPCODE_TAG_NAME) {
        event
            .get_cusipcode()
            .map(|held| Scalar::from(held.as_str()))
    } else if is(SEDOLCODE_TAG_NAME) {
        event
            .get_sedolcode()
            .map(|held| Scalar::from(held.as_str()))
    } else if is(BLOOMBERGCODE_TAG_NAME) {
        event
            .get_bloombergcode()
            .map(|held| Scalar::from(held.as_str()))
    } else if is(MICCODE_TAG_NAME) {
        event.get_miccode().map(|held| Scalar::from(held.as_str()))
    } else if tag == 15 {
        (event.get_currency() != &Currency::none())
            .then(|| Scalar::from(event.get_currency().as_str()))
    } else if tag == 54 {
        (event.get_side() != &Side::unknown()).then(|| Scalar::from(event.get_side().as_str()))
    } else if tag == super::cfi::CFICODE_TAG {
        event.get_cficode().map(|held| Scalar::from(held.as_str()))
    } else if tag == PRICE_TAG {
        number(event.get_px())
    } else if tag == ORDERQTY_TAG {
        number(event.get_qty())
    } else if tag == QUANTITY_TAG {
        // One fact, one tag on the wire: `OrderQty` answers for it above.
        None
    } else if tag == LASTPX_TAG {
        event.get_lastpx().map(Scalar::from)
    } else if tag == LASTQTY_TAG {
        event.get_lastqty().map(Scalar::from)
    } else if tag == AVGPX_TAG {
        event.get_avgpx().map(Scalar::from)
    } else if tag == CUMQTY_TAG {
        event.get_cumqty().map(Scalar::from)
    } else if tag == LEAVESQTY_TAG {
        event.get_leavesqty().map(Scalar::from)
    } else if tag == TIMEINFORCE_TAG {
        event.get_tif().map(Scalar::from)
    } else if tag == 132 {
        event.get_bidpx().map(Scalar::from)
    } else if tag == 133 {
        event.get_askpx().map(Scalar::from)
    } else if tag == 134 {
        event.get_bidqty().map(Scalar::from)
    } else if tag == 135 {
        event.get_askqty().map(Scalar::from)
    } else {
        None
    }
}

/// The exact clock the two FIX clocks a row types are held under.
pub(super) fn required_dtype(tag: i32) -> Option<DataType> {
    matches!(tag, 52 | 60).then_some(CLOCK_DATATYPE)
}

pub(super) fn refused(
    name: &str,
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> Error {
    Error::InvalidRecord {
        path: crate::path::Path::root().field(name).render().into(),
        reason: crate::text::expected_got(expected, crate::text::elide_display(&actual)),
    }
}

pub(super) fn validate_field(field: &Field, tag: i32) -> Result<()> {
    if let Some(expected) = required_dtype(tag) {
        if field.dtype() != &expected || field.as_fix().counter()?.is_some() {
            return Err(refused(field.name(), expected, field.dtype()));
        }
    }
    Ok(())
}

/// Resolve once; an explicit tag never falls through to an unrelated name.
pub(super) fn resolve_tag(field: &Field, registry: &FixRegistry) -> Result<Option<i32>> {
    let explicit = field.as_fix().tag()?;
    let named = super::field::parse_tag(field.name()).or_else(|| {
        registry
            .get_message_field_by_name(field.name())
            .and_then(|known| registry.identity_of(known).map(|(tag, _)| tag))
    });
    Ok(explicit.or(named))
}

pub(super) fn validate_value(name: &str, dtype: &DataType, value: &Scalar) -> Result<()> {
    let exact = match dtype {
        DataType::UInt64 => matches!(value, Scalar::UInt64(_)),
        DataType::Uuid => matches!(value, Scalar::Uuid(_)),
        dtype if dtype == &CLOCK_DATATYPE => {
            matches!(value.as_datetime64(), Some((_, TimeUnit::Nanosecond, zone)) if *zone == Timezone::UTC)
        }
        _ => matches!(value, Scalar::String(_)),
    };
    if exact {
        Ok(())
    } else {
        Err(refused(name, dtype, format_args!("{value:?}")))
    }
}

/// The instant now, as the clock a row types.
pub(super) fn now() -> Result<Scalar> {
    let nanos = crate::hashing::txhash::unix_now(TimeUnit::Nanosecond)?;
    Scalar::datetime64(nanos, TimeUnit::Nanosecond, Timezone::UTC)
}
