//! The typed facts a message holds beside its row, and how they are read
//! from and written to the columns that state them.

use smol_str::SmolStr;

use crate::Decimal18;
use crate::graph::{MarketElement, MarketEventData};
use crate::{
    BloombergCode, CusipCode, DataType, Error, FIGICode, Field, IsinCode, MicCode, Result, Scalar,
    SedolCode, TimeUnit, Timezone, Value,
};

use super::schema::CLOCK_DATATYPE;
use super::{
    FixRegistry, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGPLUGINID_TAG_NAME,
    MSGSESSIONID_TAG_NAME, SOURCEURL_TAG_NAME,
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
/// FIX and none of it is content, so none of it is an entry or a byte on
/// the wire, and none of it reaches the code the message's content digests
/// to. Where message type, session instance, message context and sequence are
/// all present, they are retained together under the message identifier
/// `msgsesseventid`; they remain delivery provenance and do not become the
/// message's content identity or its chain code.
///
/// What the *reader* says about the line is not here: the object the line
/// was read from, the body it was cut from, its place in that object are
/// [the cells the message carries](super::FixMsg::carried), stated by
/// whoever read the line and stated again at their columns by `into_row`,
/// because the same message read out of a second copy of one day's log is
/// the same message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixCapture {
    msgpluginid: Option<SmolStr>,
    msgctxid: Option<SmolStr>,
    msgsessionid: Option<SmolStr>,
}

impl FixCapture {
    /// The plugin that logged the line inside a bridge, as the bridge names
    /// it.
    #[must_use]
    pub fn msgpluginid(&self) -> Option<&str> {
        self.msgpluginid.as_deref()
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
        if is(MSGPLUGINID_TAG_NAME) {
            text(&self.msgpluginid)
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
        if is(MSGPLUGINID_TAG_NAME) {
            self.msgpluginid = text();
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
    msgcat: Option<SmolStr>,
    price: Option<Decimal18>,
    orderqty: Option<Decimal18>,
    quantity: Option<Decimal18>,
    lastpx: Option<Decimal18>,
    lastqty: Option<Decimal18>,
    avgpx: Option<Decimal18>,
    cumqty: Option<Decimal18>,
    leavesqty: Option<Decimal18>,
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
    /// The message type's fixed business category.
    #[must_use]
    pub fn msgcat(&self) -> Option<&str> {
        self.msgcat.as_deref()
    }

    pub(super) fn set_msgcat(&mut self, value: Option<&str>) {
        self.msgcat = value.map(SmolStr::new);
    }

    /// `Price(44)`, where the message stated one.
    #[must_use]
    pub const fn price(&self) -> Option<Decimal18> {
        self.price
    }

    /// `OrderQty(38)`, where the message stated one.
    #[must_use]
    pub const fn orderqty(&self) -> Option<Decimal18> {
        self.orderqty
    }

    /// `Quantity(53)`, the newer spelling, where the message stated one.
    ///
    /// Its own slot rather than a second name for `OrderQty`: a line that
    /// said `53=` re-emits `53=`, and a row keeps one column per tag.
    #[must_use]
    pub const fn quantity(&self) -> Option<Decimal18> {
        self.quantity
    }

    /// `LastPx(31)`, where the message stated one.
    #[must_use]
    pub const fn lastpx(&self) -> Option<Decimal18> {
        self.lastpx
    }

    /// `LastQty(32)`, where the message stated one.
    #[must_use]
    pub const fn lastqty(&self) -> Option<Decimal18> {
        self.lastqty
    }

    /// `AvgPx(6)`, where the message stated one.
    #[must_use]
    pub const fn avgpx(&self) -> Option<Decimal18> {
        self.avgpx
    }

    /// `CumQty(14)`, where the message stated one.
    #[must_use]
    pub const fn cumqty(&self) -> Option<Decimal18> {
        self.cumqty
    }

    /// `LeavesQty(151)`, where the message stated one.
    #[must_use]
    pub const fn leavesqty(&self) -> Option<Decimal18> {
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
            tag if tag == super::MSGCAT_TAG_NAME.0 => text(&self.msgcat),
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
        let number = || Decimal18::from_scalar(value);
        let text = || {
            value
                .as_str()
                .map(SmolStr::new)
                .filter(|held| !held.is_empty())
        };
        match tag {
            tag if tag == super::MSGCAT_TAG_NAME.0 => self.msgcat = text(),
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
/// [`CRATE_TAG_MAX`](crate::CRATE_TAG_MAX), and `sourceurl` is the one of
/// them no message holds.
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

/// The crate's own tag for the column that states what the *reader* said
/// about a line rather than what the line said: the object it was read from.
///
/// No message holds it, and [the fixed row](super::fix_schema) has no column
/// for it: a monitor orders, joins and prunes on where a row came out of,
/// and that is the capture's fact beside the row rather than the message's
/// within it. It travels exactly as the columns a capture carries under no
/// tag at all do - the body the line was cut from, its place in the object,
/// its media type, when the reader read it, what a bound dropped - and the
/// tag is here so a carried column typed as a URL is known to be one.
pub(super) const CAPTURE_TAG: i32 = SOURCEURL_TAG_NAME.0;

/// Whether a tag names the capture's own column.
///
/// Read wherever a message meets a row: such a column is read past on the
/// way in, answers the reader's cell rather than a fact on the way out, and
/// is never a fill, an entry, a digest input or a byte on the wire.
pub(super) fn is_capture_tag(tag: i32) -> bool {
    tag == CAPTURE_TAG
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
            event_fact(self.event, tag)
                .or_else(|| self.lifted.fact(tag))
                .or_else(|| self.capture.fact(tag))
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

/// Records what one event column states on the event, through the column
/// it is: a value the fact's type refuses is silence and a null clears the
/// fact. Whether the tag is one the event holds.
pub(super) fn record_event(event: &mut MarketEventData, tag: i32, value: &Scalar) -> bool {
    match tag {
        tag if tag == super::ISINCODE_TAG_NAME.0 => event.set_isincode(
            IsinCode::from_scalar(value)
                .cloned()
                .or_else(|| value.as_str().and_then(|value| IsinCode::new(value).ok())),
        ),
        tag if tag == super::CUSIPCODE_TAG_NAME.0 => event.set_cusipcode(
            CusipCode::from_scalar(value)
                .cloned()
                .or_else(|| value.as_str().and_then(|value| CusipCode::new(value).ok())),
        ),
        tag if tag == super::SEDOLCODE_TAG_NAME.0 => event.set_sedolcode(
            SedolCode::from_scalar(value)
                .cloned()
                .or_else(|| value.as_str().and_then(|value| SedolCode::new(value).ok())),
        ),
        tag if tag == super::BLOOMBERGCODE_TAG_NAME.0 => {
            event.set_bloombergcode(BloombergCode::from_scalar(value).cloned().or_else(|| {
                value
                    .as_str()
                    .and_then(|value| BloombergCode::new(value).ok())
            }))
        }
        tag if tag == super::FIGICODE_TAG_NAME.0 => event.set_figicode(
            FIGICode::from_scalar(value)
                .cloned()
                .or_else(|| value.as_str().and_then(|value| FIGICode::new(value).ok())),
        ),
        tag if tag == super::MICCODE_TAG_NAME.0 => event.set_miccode(
            MicCode::from_scalar(value)
                .cloned()
                .or_else(|| value.as_str().and_then(|value| MicCode::new(value).ok())),
        ),
        _ => match super::crated::event_column_of(tag) {
            Some(column) => {
                column.record(event, value);
                return true;
            }
            None => return false,
        },
    }
    true
}

/// What the event states for one event column, as the raw value the
/// column's field types, or nothing where it states no fact: an empty name,
/// an absent instant, identity or code.
pub(super) fn event_fact(event: &MarketEventData, tag: i32) -> Option<Scalar> {
    match super::crated::event_column_of(tag) {
        Some(column) => column.fact(event),
        None => match tag {
            tag if tag == super::ISINCODE_TAG_NAME.0 => {
                event.get_isincode().cloned().map(Scalar::IsinCode)
            }
            tag if tag == super::CUSIPCODE_TAG_NAME.0 => {
                event.get_cusipcode().cloned().map(Scalar::CusipCode)
            }
            tag if tag == super::SEDOLCODE_TAG_NAME.0 => {
                event.get_sedolcode().cloned().map(Scalar::SedolCode)
            }
            tag if tag == super::BLOOMBERGCODE_TAG_NAME.0 => event
                .get_bloombergcode()
                .cloned()
                .map(Scalar::BloombergCode),
            tag if tag == super::FIGICODE_TAG_NAME.0 => {
                event.get_figicode().cloned().map(Scalar::FIGICode)
            }
            tag if tag == super::MICCODE_TAG_NAME.0 => {
                event.get_miccode().cloned().map(Scalar::MicCode)
            }
            _ => None,
        },
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
    // The tag the field carries is the answer where it carries one, and
    // the name is read only where it does not: a column of a built message
    // carries its tag, so the dictionary is not asked a question the field
    // already answered - and a column stated under the dictionary's own
    // field is answered off the dictionary's index rather than the
    // column's metadata.
    if !field.as_metadata().is_empty() {
        if let Some(explicit) = registry.facts_of(field).and_then(|facts| facts.tag) {
            return Ok(Some(explicit));
        }
        if let Some(explicit) = field.as_fix().tag()? {
            return Ok(Some(explicit));
        }
    }
    Ok(super::field::parse_tag(field.name()).or_else(|| {
        registry
            .get_message_field_by_name(field.name())
            .and_then(|known| registry.identity_of(known).map(|(tag, _)| tag))
    }))
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
    let nanos = crate::txhash::unix_now(TimeUnit::Nanosecond)?;
    Scalar::datetime64(nanos, TimeUnit::Nanosecond, Timezone::UTC)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/mod_.rs` pins and a caller cannot reach.
    //!
    //! [`FIX_TYPED_TAGS`](crate::FIX_TYPED_TAGS) is the published listing;
    //! the predicate behind it is a step inside a lift, so the listing is
    //! pinned against it here rather than against itself.

    /// Whether a message lifts the tag into a typed fact of its own.
    #[must_use]
    pub fn is_typed_tag(tag: i32) -> bool {
        super::is_typed_tag(tag)
    }
}
