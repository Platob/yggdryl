//! The typed facts a message holds beside its row, and how they are read
//! from and written to the columns that state them.

use std::collections::BTreeMap;

use smol_str::SmolStr;

use crate::Decimal;
use crate::graph::{Element, Event, MarketEventData};
use crate::{DataType, Error, Field, Result, Scalar, TimeUnit, Timezone};

use super::schema::CLOCK_DATATYPE;
use super::{
    CREATUNIX_TAG_NAME, CROSSCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME, CROSSUUID_TAG_NAME,
    CURRHASHCODE_TAG_NAME, CURRUNIX_TAG_NAME, CURRUUID_TAG_NAME, FixRegistry, IDENTIFIERS_TAG_NAME,
    MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGSESSIONID_TAG_NAME, PARENTUUIDS_TAG_NAME,
    PLUGINID_TAG_NAME, PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME, RECORDEDAT_TAG_NAME, SEQNUM_TAG_NAME,
    SNAPUNIX_TAG_NAME, SOURCEURL_TAG_NAME,
};

/// The standard header and trailer facts every message holds typed, beside
/// its row.
///
/// What FIX puts in front of every message and every consumer reads first:
/// the version the message says it speaks, the type it is, who sent it to
/// whom, its place in the session and when it was sent. Behind the body
/// stands what closes the frame: the signature it was signed with and the
/// checksum. Each is read and written here rather than looked up in the row,
/// and the row holds none of them: a message is its frame, its event and its
/// content, once each. The frame is also the one band the wire emits out of
/// tag order - the header first, whatever the body says, and the trailer
/// last - which is exactly why it is held rather than left among the
/// children.
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
    signaturelength: Option<i32>,
    signature: Option<Vec<u8>>,
    checksum: Option<SmolStr>,
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
            signaturelength: None,
            signature: None,
            checksum: None,
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

    /// `SignatureLength(93)`, where the message was signed.
    #[must_use]
    pub const fn signaturelength(&self) -> Option<i32> {
        self.signaturelength
    }

    /// `Signature(89)`, where the message was signed.
    #[must_use]
    pub fn signature(&self) -> Option<&[u8]> {
        self.signature.as_deref()
    }

    /// `CheckSum(10)` exactly as the line spelled it, three digits and all,
    /// where the message carried one: a re-emission states what was read
    /// rather than a sum of bytes nobody sent.
    #[must_use]
    pub fn checksum(&self) -> Option<&str> {
        self.checksum.as_deref()
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
            93 => self.signaturelength.map(Scalar::from),
            89 => self.signature.as_deref().map(Scalar::from),
            10 => self.checksum.as_deref().and_then(text),
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
            93 => self.signaturelength = value.as_i64().and_then(|held| i32::try_from(held).ok()),
            89 => self.signature = value.as_bytes().map(<[u8]>::to_vec),
            10 => self.checksum = text(),
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

/// What the line itself said about the capture it was written for, typed.
///
/// What a bridge's own row header states about the line it wrote - the
/// plugin, the message context and the session instance - read off the
/// line's own bytes like every other fact a message holds. None of it is
/// FIX and none of it is content, so nothing here reaches the code the
/// message digests to or the wire it re-emits.
///
/// What the *reader* says about the line is not here and is held nowhere on
/// a message: the object the line was read from, when the capture wrote it
/// down, the body it was cut from, its place in that object. Those are
/// [the capture's own columns](super::FixMsg::from_row), stated by whoever
/// read the line and restated by whoever writes the row back, because the
/// same message read out of a second copy of one day's log is the same
/// message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixCapture {
    pluginid: Option<SmolStr>,
    msgctxid: Option<SmolStr>,
    msgsessionid: Option<SmolStr>,
}

impl FixCapture {
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
        if is(PLUGINID_TAG_NAME) {
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
        if is(PLUGINID_TAG_NAME) {
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

/// The FIX fields a message lifts out of its row and holds typed.
///
/// A price and a quantity are what a consumer reads first, and an
/// identifier is what one message of a chain shares with the next, so each
/// is held beside the header rather than looked up among the children: a
/// lookup answers without walking the row, and the wire re-emits them in
/// one band in front of the content. Nothing here is derived - every slot
/// is a fact the line stated, a row restated, or a caller or a derivation
/// wrote through a tag - which is what makes a slot's presence the whole
/// test of whether it reaches the wire, the arrival record or the code the
/// message digests to. What a message *implies* about its market lives on
/// the event, off these and off the row, and reaches none of the three.
pub(super) const LIFTED_TAGS: [i32; 17] = [
    AVGPX_TAG,
    11,
    CUMQTY_TAG,
    17,
    LASTPX_TAG,
    LASTQTY_TAG,
    37,
    ORDERQTY_TAG,
    41,
    PRICE_TAG,
    QUANTITY_TAG,
    117,
    131,
    LEAVESQTY_TAG,
    198,
    262,
    1003,
];

/// FIX's own fields for the numbers a message is about.
pub(super) const PRICE_TAG: i32 = 44;
pub(super) const ORDERQTY_TAG: i32 = 38;
pub(super) const QUANTITY_TAG: i32 = 53;
pub(super) const LASTPX_TAG: i32 = 31;
pub(super) const LASTQTY_TAG: i32 = 32;
pub(super) const AVGPX_TAG: i32 = 6;
pub(super) const CUMQTY_TAG: i32 = 14;
pub(super) const LEAVESQTY_TAG: i32 = 151;
pub(super) const TIMEINFORCE_TAG: i32 = 59;

/// The prices and quantities a message lifted, and the identifiers it is
/// known by, each exactly as the message stated it.
///
/// Eight exact numbers and nine identifiers, every one an `Option`: absent
/// is "the message never said", and that is the only flag this holder
/// needs, because nothing writes here but a tag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixLifted {
    price: Option<Decimal>,
    orderqty: Option<Decimal>,
    quantity: Option<Decimal>,
    lastpx: Option<Decimal>,
    lastqty: Option<Decimal>,
    avgpx: Option<Decimal>,
    cumqty: Option<Decimal>,
    leavesqty: Option<Decimal>,
    clordid: Option<SmolStr>,
    origclordid: Option<SmolStr>,
    orderid: Option<SmolStr>,
    secondaryorderid: Option<SmolStr>,
    execid: Option<SmolStr>,
    quoteid: Option<SmolStr>,
    quotereqid: Option<SmolStr>,
    mdreqid: Option<SmolStr>,
    tradeid: Option<SmolStr>,
}

impl FixLifted {
    /// `Price(44)`, where the message stated one.
    #[must_use]
    pub const fn price(&self) -> Option<Decimal> {
        self.price
    }

    /// `OrderQty(38)`, where the message stated one.
    #[must_use]
    pub const fn orderqty(&self) -> Option<Decimal> {
        self.orderqty
    }

    /// `Quantity(53)`, the newer spelling, where the message stated one.
    ///
    /// Its own slot rather than a second name for `OrderQty`: a line that
    /// said `53=` re-emits `53=`, and a row keeps one column per tag.
    #[must_use]
    pub const fn quantity(&self) -> Option<Decimal> {
        self.quantity
    }

    /// `LastPx(31)`, where the message stated one.
    #[must_use]
    pub const fn lastpx(&self) -> Option<Decimal> {
        self.lastpx
    }

    /// `LastQty(32)`, where the message stated one.
    #[must_use]
    pub const fn lastqty(&self) -> Option<Decimal> {
        self.lastqty
    }

    /// `AvgPx(6)`, where the message stated one.
    #[must_use]
    pub const fn avgpx(&self) -> Option<Decimal> {
        self.avgpx
    }

    /// `CumQty(14)`, where the message stated one.
    #[must_use]
    pub const fn cumqty(&self) -> Option<Decimal> {
        self.cumqty
    }

    /// `LeavesQty(151)`, where the message stated one.
    #[must_use]
    pub const fn leavesqty(&self) -> Option<Decimal> {
        self.leavesqty
    }

    /// `ClOrdID(11)`, where the message stated one.
    #[must_use]
    pub fn clordid(&self) -> Option<&str> {
        self.clordid.as_deref()
    }

    /// `OrigClOrdID(41)`, where the message stated one.
    #[must_use]
    pub fn origclordid(&self) -> Option<&str> {
        self.origclordid.as_deref()
    }

    /// `OrderID(37)`, where the message stated one.
    #[must_use]
    pub fn orderid(&self) -> Option<&str> {
        self.orderid.as_deref()
    }

    /// `SecondaryOrderID(198)`, where the message stated one.
    #[must_use]
    pub fn secondaryorderid(&self) -> Option<&str> {
        self.secondaryorderid.as_deref()
    }

    /// `ExecID(17)`, where the message stated one.
    #[must_use]
    pub fn execid(&self) -> Option<&str> {
        self.execid.as_deref()
    }

    /// `QuoteID(117)`, where the message stated one.
    #[must_use]
    pub fn quoteid(&self) -> Option<&str> {
        self.quoteid.as_deref()
    }

    /// `QuoteReqID(131)`, where the message stated one.
    #[must_use]
    pub fn quotereqid(&self) -> Option<&str> {
        self.quotereqid.as_deref()
    }

    /// `MDReqID(262)`, where the message stated one.
    #[must_use]
    pub fn mdreqid(&self) -> Option<&str> {
        self.mdreqid.as_deref()
    }

    /// `TradeID(1003)`, where the message stated one.
    #[must_use]
    pub fn tradeid(&self) -> Option<&str> {
        self.tradeid.as_deref()
    }

    /// The value one lifted tag holds, as the raw value its column types.
    pub(super) fn fact(&self, tag: i32) -> Option<Scalar> {
        let text = |held: &Option<SmolStr>| held.as_deref().map(Scalar::from);
        match tag {
            PRICE_TAG => self.price.map(Scalar::from),
            ORDERQTY_TAG => self.orderqty.map(Scalar::from),
            QUANTITY_TAG => self.quantity.map(Scalar::from),
            LASTPX_TAG => self.lastpx.map(Scalar::from),
            LASTQTY_TAG => self.lastqty.map(Scalar::from),
            AVGPX_TAG => self.avgpx.map(Scalar::from),
            CUMQTY_TAG => self.cumqty.map(Scalar::from),
            LEAVESQTY_TAG => self.leavesqty.map(Scalar::from),
            11 => text(&self.clordid),
            41 => text(&self.origclordid),
            37 => text(&self.orderid),
            198 => text(&self.secondaryorderid),
            17 => text(&self.execid),
            117 => text(&self.quoteid),
            131 => text(&self.quotereqid),
            262 => text(&self.mdreqid),
            1003 => text(&self.tradeid),
            _ => None,
        }
    }

    /// Records what one lifted tag states; a null clears it. Whether the
    /// tag is a lifted tag at all.
    fn record(&mut self, tag: i32, value: &Scalar) -> bool {
        let number = || Decimal::from_scalar(value);
        let text = || {
            value
                .as_str()
                .map(SmolStr::new)
                .filter(|held| !held.is_empty())
        };
        match tag {
            PRICE_TAG => self.price = number(),
            ORDERQTY_TAG => self.orderqty = number(),
            QUANTITY_TAG => self.quantity = number(),
            LASTPX_TAG => self.lastpx = number(),
            LASTQTY_TAG => self.lastqty = number(),
            AVGPX_TAG => self.avgpx = number(),
            CUMQTY_TAG => self.cumqty = number(),
            LEAVESQTY_TAG => self.leavesqty = number(),
            11 => self.clordid = text(),
            41 => self.origclordid = text(),
            37 => self.orderid = text(),
            198 => self.secondaryorderid = text(),
            17 => self.execid = text(),
            117 => self.quoteid = text(),
            131 => self.quotereqid = text(),
            262 => self.mdreqid = text(),
            1003 => self.tradeid = text(),
            _ => return false,
        }
        true
    }
}

/// The standard header and trailer tags the frame holds.
const HEADER_TAGS: [i32; 11] = [
    8,
    35,
    49,
    56,
    34,
    52,
    43,
    MSGDIRECTION_TAG_NAME.0,
    93,
    89,
    10,
];

/// The standard header tags the wire emits in front of the body, in the
/// order it emits them.
pub(super) const WIRE_HEADER_TAGS: [i32; 7] = [8, 35, 49, 56, 34, 43, 52];

/// The standard trailer tags the wire emits behind the body, in the order
/// FIX closes a frame with: the signature it was signed with, then the
/// checksum, which is always last.
pub(super) const WIRE_TRAILER_TAGS: [i32; 3] = [93, 89, 10];

/// FIX's `Text(58)`: the free text a message carries, which the message
/// holds typed beside its event.
pub(super) const TEXT_TAG: i32 = 58;

/// Every tag a message holds typed outside the crate's own range, in the
/// order the wire states them: the standard header, the FIX fields a
/// message lifts, the standard trailer, and `Text(58)`.
///
/// A caller that has to walk a message's typed facts - a binding rebuilding
/// one from a pickle, a reader listing what left the row - reads this
/// rather than keeping a list of its own, because a list of its own drifts
/// the moment a tag is lifted or retired, and it drifts silently: the facts
/// it stops naming are exactly the ones no column holds either.
///
/// The crate's own tags are not here. They are a contiguous block a caller
/// walks with [`CRATE_TAG_MIN`](crate::CRATE_TAG_MIN) and
/// [`CRATE_TAG_MAX`](crate::CRATE_TAG_MAX), and `sourceurl` and
/// `recordedat` are the two of them no message holds.
pub const FIX_TYPED_TAGS: [i32; 29] = [
    WIRE_HEADER_TAGS[0],
    WIRE_HEADER_TAGS[1],
    WIRE_HEADER_TAGS[2],
    WIRE_HEADER_TAGS[3],
    WIRE_HEADER_TAGS[4],
    WIRE_HEADER_TAGS[5],
    WIRE_HEADER_TAGS[6],
    MSGDIRECTION_TAG_NAME.0,
    LIFTED_TAGS[0],
    LIFTED_TAGS[1],
    LIFTED_TAGS[2],
    LIFTED_TAGS[3],
    LIFTED_TAGS[4],
    LIFTED_TAGS[5],
    LIFTED_TAGS[6],
    LIFTED_TAGS[7],
    LIFTED_TAGS[8],
    LIFTED_TAGS[9],
    LIFTED_TAGS[10],
    LIFTED_TAGS[11],
    LIFTED_TAGS[12],
    LIFTED_TAGS[13],
    LIFTED_TAGS[14],
    LIFTED_TAGS[15],
    LIFTED_TAGS[16],
    WIRE_TRAILER_TAGS[0],
    WIRE_TRAILER_TAGS[1],
    WIRE_TRAILER_TAGS[2],
    TEXT_TAG,
];

/// The crate's own columns that state what the *reader* said about a line
/// rather than what the line said: the object it was read from, and when
/// the capture wrote it down.
///
/// No message holds either. They are columns of the row all the same - a
/// monitor orders, joins and prunes on where a row came out of and when it
/// was recorded - so whoever read the line states them and whoever writes
/// the row back restates them, beside the columns a capture carries under
/// no tag at all: the body the line was cut from, its place in the object,
/// its media type, what a bound dropped.
pub(super) const CAPTURE_TAGS: [i32; 2] = [SOURCEURL_TAG_NAME.0, RECORDEDAT_TAG_NAME.0];

/// Whether a tag names one of the capture's own columns.
///
/// Read wherever a message meets a row: such a column is read past on the
/// way in, answers the reader's cell rather than a fact on the way out, and
/// is never a fill, an entry, a digest input or a byte on the wire.
pub(super) fn is_capture_tag(tag: i32) -> bool {
    CAPTURE_TAGS.contains(&tag)
}

/// Whether a tag names a fact the message holds typed rather than in its
/// row: one of the crate's own columns, a standard header or trailer tag,
/// or one of the FIX fields the message lifted.
///
/// The capture's own columns are none of them: a message states nothing
/// about the reading it arrived through.
pub(super) fn is_typed_tag(tag: i32) -> bool {
    !is_capture_tag(tag)
        && (super::is_crate_tag(tag)
            || HEADER_TAGS.contains(&tag)
            || LIFTED_TAGS.contains(&tag)
            || tag == TEXT_TAG)
}

/// The four typed holders of a message, read and written by tag.
pub(super) struct Typed<'msg> {
    pub(super) event: &'msg MarketEventData,
    pub(super) header: &'msg FixHeader,
    pub(super) capture: &'msg FixCapture,
    pub(super) lifted: &'msg FixLifted,
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
            self.lifted.fact(tag)
        }
    }
}

/// Records what one typed tag states on the holder that owns it; a null
/// clears the fact. Whether the tag is a typed tag at all.
pub(super) fn record(
    event: &mut MarketEventData,
    header: &mut FixHeader,
    capture: &mut FixCapture,
    lifted: &mut FixLifted,
    tag: i32,
    value: &Scalar,
) -> bool {
    if HEADER_TAGS.contains(&tag) {
        return header.record(tag, value);
    }
    if capture.record(tag, value) {
        return true;
    }
    if lifted.record(tag, value) {
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
    let is = |held: (i32, &str)| held.0 == tag;
    if is(CURRUNIX_TAG_NAME) {
        if let Some(unix) = instant() {
            event.set_currunix(unix);
        }
    } else if is(CREATUNIX_TAG_NAME) {
        event.set_creatunix(instant());
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
    } else if is(CURRHASHCODE_TAG_NAME) {
        if let Some(code) = value.as_u64() {
            event.set_currhashcode(code);
        }
    } else if is(CROSSHASHCODE_TAG_NAME) {
        if let Some(code) = value.as_u64() {
            event.set_crosshashcode(code);
        }
    } else if is(CROSSCODE_TAG_NAME) {
        event.set_crosscode(text().map(str::to_owned).unwrap_or_default());
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
    let is = |held: (i32, &str)| held.0 == tag;
    if is(CURRUNIX_TAG_NAME) {
        instant(event.get_currunix())
    } else if is(CREATUNIX_TAG_NAME) {
        stated(event.get_creatunix())
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
    } else if is(CURRHASHCODE_TAG_NAME) {
        Some(Scalar::from(event.get_currhashcode()))
    } else if is(CROSSHASHCODE_TAG_NAME) {
        Some(Scalar::from(event.get_crosshashcode()))
    } else if is(CROSSCODE_TAG_NAME) {
        text(event.get_crosscode())
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
        DataType::Uuid(_) => matches!(value, Scalar::Uuid(_)),
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
    let nanos = crate::txhash::unix_now(TimeUnit::Nanosecond)?;
    Scalar::datetime64(nanos, TimeUnit::Nanosecond, Timezone::UTC)
}
