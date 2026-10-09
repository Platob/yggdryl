//! What type of its kind a market element is - the order type, the quote
//! type, the trade type, the book entry type, and the type of a trade
//! report, a quote request, a mass cancel or a market data request - as one
//! enum, stored as a `uint16`.

use crate::implementer::define_field_types;
use crate::implementer::enum_leaf;
use crate::implementer::folded_spelling;

enum_leaf! {
    /// The type of a market data element within its kind: how an order is
    /// priced (`ORDLIMIT`, `ORDMKT`), what a quote commits to (`QUOTRAD`),
    /// what kind of trade was reported (`TRDBLOCK`), what a book entry is
    /// (`BOOKBID`).
    ///
    /// One generic integer set over the eight FIX code sets that type an
    /// element: `OrdType(40)`, `QuoteType(537)`, `TrdType(828)`,
    /// `MDEntryType(269)`, `TradeReportType(856)`, `QuoteRequestType(303)`,
    /// `MassCancelRequestType(530)` and `SubscriptionRequestType(263)`. The
    /// hundreds of the code name the set - `1xx` an order type, `2xx` a
    /// quote type, `3xx` a trade type, `4xx` a book entry type, `5xx` a
    /// trade report type, `6xx` a quote request type, `7xx` a mass cancel
    /// type, `8xx` a market data request type - so the stored integers group
    /// by what they type, and each set closes with an `OTHER` member a wire
    /// value no member names reads as. Which fields type a message is its
    /// type's rule first ([`MARKETDATATYPE_MSGTYPE_RULES`]), else its
    /// kind's ([`Self::fix_tags_of`]). [`Self::from_fix`] reads a wire value and [`Self::fix_code`]
    /// answers it back; a FIX registry may map any field's values onto
    /// members of its own choosing through `FIX:marketdatatype`
    /// ([`FixRegistry::marketdatatype_of`](crate::FixRegistry::marketdatatype_of)).
    ///
    /// ```
    /// use yggdryl::MarketDataType;
    ///
    /// assert_eq!(MarketDataType::from_fix(40, "2"), Some(MarketDataType::OrdLimit));
    /// assert_eq!(MarketDataType::OrdLimit.as_str(), "ORDLIMIT");
    /// assert_eq!(MarketDataType::OrdLimit.code(), 102);
    /// assert_eq!(MarketDataType::OrdLimit.fix_code(), Some((40, "2")));
    /// assert_eq!(MarketDataType::from_spelling("ordmkt"), Some(MarketDataType::OrdMarket));
    /// assert_eq!(MarketDataType::from_spelling("Limit"), Some(MarketDataType::OrdLimit));
    /// // A value the set does not name reads as its set's catch-all.
    /// assert_eq!(MarketDataType::from_fix(828, "999"), Some(MarketDataType::TrdOther));
    /// // A field that types nothing reads as none.
    /// assert_eq!(MarketDataType::from_fix(54, "1"), None);
    /// ```
    ///
    /// Its datatype and a field of it are the kind's own:
    ///
    /// ```
    /// use yggdryl::{MARKETDATATYPE_KIND, MarketDataType};
    ///
    /// assert_eq!(MarketDataType::dtype(), MARKETDATATYPE_KIND.dtype());
    /// assert_eq!(MarketDataType::dtype().to_string(), "marketdatatype");
    /// assert!(MarketDataType::dtype().is_enum());
    /// assert!(MarketDataType::field("marketdatatype").is_nullable());
    /// ```
    #[non_exhaustive]
    pub enum MarketDataType: u16, kind = "marketdatatype", extension = MARKETDATATYPE_EXTENSION_NAME, aliases = marketdatatype_aliases,
    market = MARKETDATATYPE_KIND [0xc4, 30, 74, 65] {
        #[default]
        Unknown = 0 as "UKNW": "No type stated.",
        OrdMarket = 101 as "ORDMKT": "Market order.",
        OrdLimit = 102 as "ORDLIMIT": "Limit order.",
        OrdStop = 103 as "ORDSTOP": "Stop, or stop loss, order.",
        OrdStopLimit = 104 as "ORDSTOPLIMIT": "Stop limit order.",
        OrdMarketOnClose = 105 as "ORDMOC": "Market on close order.",
        OrdWithOrWithout = 106 as "ORDWOW": "With or without order.",
        OrdLimitOrBetter = 107 as "ORDLOB": "Limit or better order.",
        OrdLimitWithOrWithout = 108 as "ORDLWOW": "Limit with or without order.",
        OrdOnBasis = 109 as "ORDBASIS": "On basis order.",
        OrdOnClose = 110 as "ORDONCLOSE": "On close order.",
        OrdLimitOnClose = 111 as "ORDLOC": "Limit on close order.",
        OrdFxMarket = 112 as "ORDFXMKT": "Forex market order.",
        OrdPrevQuoted = 113 as "ORDPREVQUOTED": "Previously quoted order.",
        OrdPrevIndicated = 114 as "ORDPREVINDIC": "Previously indicated order.",
        OrdFxLimit = 115 as "ORDFXLIMIT": "Forex limit order.",
        OrdFxSwap = 116 as "ORDFXSWAP": "Forex swap order.",
        OrdFxPrevQuoted = 117 as "ORDFXPREVQUOTED": "Forex previously quoted order.",
        OrdFunari = 118 as "ORDFUNARI": "Funari order: a limit day order whose unexecuted part becomes a market on close order.",
        OrdMarketIfTouched = 119 as "ORDMIT": "Market if touched order.",
        OrdMarketToLimit = 120 as "ORDMKTLIMIT": "Market order whose unexecuted part stands as a limit order.",
        OrdPrevFundPoint = 121 as "ORDPREVFUND": "Order at the previous fund valuation point.",
        OrdNextFundPoint = 122 as "ORDNEXTFUND": "Order at the next fund valuation point.",
        OrdPegged = 123 as "ORDPEGGED": "Pegged order.",
        OrdCounterSelection = 124 as "ORDCOUNTER": "Counter-order selection.",
        OrdStopOnBidOffer = 125 as "ORDSTOPBO": "Stop on bid or offer order.",
        OrdStopLimitOnBidOffer = 126 as "ORDSTOPLIMITBO": "Stop limit on bid or offer order.",
        OrdMarketInBand = 127 as "ORDMKTBAND": "Market order within a price band.",
        OrdOther = 199 as "ORDOTHER": "An order type no member names.",
        QuoIndicative = 200 as "QUOINDIC": "Indicative quote.",
        QuoTradeable = 201 as "QUOTRAD": "Tradeable quote.",
        QuoRestricted = 202 as "QUORESTR": "Restricted tradeable quote.",
        QuoCounter = 203 as "QUOCOUNTER": "Counter quote.",
        QuoInitTradeable = 204 as "QUOINIT": "Initially tradeable quote.",
        QuoOther = 299 as "QUOOTHER": "A quote type no member names.",
        TrdRegular = 300 as "TRDREG": "Regular trade.",
        TrdBlock = 301 as "TRDBLOCK": "Block trade.",
        TrdEfp = 302 as "TRDEFP": "Exchange for physical.",
        TrdTransfer = 303 as "TRDTRANSFER": "Transfer.",
        TrdLate = 304 as "TRDLATE": "Late trade.",
        TrdTTrade = 305 as "TRDT": "T trade.",
        TrdWap = 306 as "TRDWAP": "Weighted average price trade.",
        TrdBunched = 307 as "TRDBUNCHED": "Bunched trade.",
        TrdLateBunched = 308 as "TRDLATEBUNCHED": "Late bunched trade.",
        TrdPriorRef = 309 as "TRDPRIORREF": "Prior reference price trade.",
        TrdAfterHours = 310 as "TRDAFTERHOURS": "After hours trade.",
        TrdEfr = 311 as "TRDEFR": "Exchange for risk.",
        TrdEfs = 312 as "TRDEFS": "Exchange for swap.",
        TrdAtSettlement = 315 as "TRDTAS": "Trading at settlement.",
        TrdAllOrNone = 316 as "TRDAON": "All or none trade.",
        TrdError = 324 as "TRDERROR": "Error trade.",
        TrdLarge = 338 as "TRDLARGE": "Large trade.",
        TrdOptionExercise = 345 as "TRDEXERCISE": "Option exercise.",
        TrdPortfolio = 350 as "TRDPORTFOLIO": "Portfolio trade.",
        TrdVwap = 351 as "TRDVWAP": "Volume weighted average trade.",
        TrdOtc = 354 as "TRDOTC": "Over the counter trade.",
        TrdOpening = 356 as "TRDOPENING": "Opening trade.",
        TrdNetted = 357 as "TRDNETTED": "Netted trade.",
        TrdDark = 362 as "TRDDARK": "Dark trade.",
        TrdTechnical = 363 as "TRDTECHNICAL": "Technical trade.",
        TrdBenchmark = 364 as "TRDBENCHMARK": "Benchmark trade.",
        TrdPackage = 365 as "TRDPACKAGE": "Package trade.",
        TrdRoll = 366 as "TRDROLL": "Roll trade.",
        TrdClosingPrice = 367 as "TRDCLOSING": "Closing price trade.",
        TrdOther = 399 as "TRDOTHER": "A trade type no member names.",
        BookBid = 400 as "BOOKBID": "Book bid entry.",
        BookOffer = 401 as "BOOKOFFER": "Book offer entry.",
        BookTrade = 402 as "BOOKTRADE": "Book trade entry.",
        BookIndex = 403 as "BOOKINDEX": "Index value entry.",
        BookOpen = 404 as "BOOKOPEN": "Opening price entry.",
        BookClose = 405 as "BOOKCLOSE": "Closing price entry.",
        BookSettle = 406 as "BOOKSETTLE": "Settlement price entry.",
        BookHigh = 407 as "BOOKHIGH": "Session high price entry.",
        BookLow = 408 as "BOOKLOW": "Session low price entry.",
        BookVwap = 409 as "BOOKVWAP": "Volume weighted average price entry.",
        BookImbalance = 410 as "BOOKIMBALANCE": "Imbalance entry.",
        BookVolume = 411 as "BOOKVOLUME": "Trade volume entry.",
        BookOpenInterest = 412 as "BOOKOI": "Open interest entry.",
        BookMid = 417 as "BOOKMID": "Mid price entry.",
        BookEmpty = 418 as "BOOKEMPTY": "Empty book entry.",
        BookOther = 499 as "BOOKOTHER": "A book entry type no member names.",
        TrptSubmit = 500 as "TRPTSUBMIT": "Trade report submitted.",
        TrptAlleged = 501 as "TRPTALLEGED": "Trade report alleged by the counterparty.",
        TrptAccept = 502 as "TRPTACCEPT": "Trade report accepted.",
        TrptDecline = 503 as "TRPTDECLINE": "Trade report declined.",
        TrptAddendum = 504 as "TRPTADDENDUM": "Trade report addendum.",
        TrptNoWas = 505 as "TRPTNOWAS": "Trade report replacing an earlier one: no, was.",
        TrptCancel = 506 as "TRPTCANCEL": "Trade report canceled.",
        TrptBreak = 507 as "TRPTBREAK": "Locked-in trade broken.",
        TrptDefaulted = 508 as "TRPTDEFAULTED": "Trade report defaulted.",
        TrptInvalidCmta = 509 as "TRPTINVALIDCMTA": "Trade report with an invalid give-up agreement.",
        TrptPended = 510 as "TRPTPENDED": "Trade report pended.",
        TrptAllegedNew = 511 as "TRPTALLEGEDNEW": "New trade report alleged.",
        TrptAllegedAddendum = 512 as "TRPTALLEGEDADD": "Trade report addendum alleged.",
        TrptAllegedNoWas = 513 as "TRPTALLEGEDNOWAS": "Replacing trade report alleged.",
        TrptAllegedCancel = 514 as "TRPTALLEGEDCANCEL": "Trade report cancel alleged.",
        TrptAllegedBreak = 515 as "TRPTALLEGEDBREAK": "Trade break alleged.",
        TrptOther = 599 as "TRPTOTHER": "A trade report type no member names.",
        QrqManual = 601 as "QRQMANUAL": "Quote requested by hand.",
        QrqAutomatic = 602 as "QRQAUTO": "Quote requested automatically.",
        QrqOther = 699 as "QRQOTHER": "A quote request type no member names.",
        McxSecurity = 701 as "MCXSECURITY": "Cancel the orders for a security.",
        McxUnderlying = 702 as "MCXUNDERLYING": "Cancel the orders for an underlying security.",
        McxProduct = 703 as "MCXPRODUCT": "Cancel the orders for a product.",
        McxCfi = 704 as "MCXCFI": "Cancel the orders for a CFI code.",
        McxSecurityType = 705 as "MCXSECTYPE": "Cancel the orders for a security type.",
        McxSession = 706 as "MCXSESSION": "Cancel the orders for a trading session.",
        McxAll = 707 as "MCXALL": "Cancel all orders.",
        McxMarket = 708 as "MCXMARKET": "Cancel the orders for a market.",
        McxSegment = 709 as "MCXSEGMENT": "Cancel the orders for a market segment.",
        McxGroup = 710 as "MCXGROUP": "Cancel the orders for a security group.",
        McxIssuer = 711 as "MCXISSUER": "Cancel the orders for a securities issuer.",
        McxUnderlyingIssuer = 712 as "MCXUNDISSUER": "Cancel the orders for the issuer of an underlying security.",
        McxOther = 799 as "MCXOTHER": "A mass cancel request type no member names.",
        MdrSnapshot = 800 as "MDRSNAPSHOT": "Market data requested as one snapshot.",
        MdrSubscribe = 801 as "MDRSUBSCRIBE": "Market data requested as a snapshot and its updates.",
        MdrUnsubscribe = 802 as "MDRUNSUBSCRIBE": "A market data subscription withdrawn.",
        MdrOther = 899 as "MDROTHER": "A market data request type no member names.",
    }
}

/// The FIX fields whose code sets a member types, one set each:
/// `OrdType(40)`, `QuoteType(537)`, `TrdType(828)`, `MDEntryType(269)`,
/// `TradeReportType(856)`, `QuoteRequestType(303)`,
/// `MassCancelRequestType(530)` and `SubscriptionRequestType(263)`, in that
/// order.
pub const MARKETDATATYPE_FIX_TAGS: [i32; 8] = [40, 537, 828, 269, 856, 303, 530, 263];

/// The message types whose own field types them before their kind's: each
/// message type and the fields read for it, first stated first. A trade
/// capture report and its acknowledgement are typed by what the report is,
/// `TradeReportType(856)`, before the trade it reports; a quote request
/// and its rejection by `QuoteRequestType(303)`; a mass cancel request and
/// its report by `MassCancelRequestType(530)`; a market data request by
/// `SubscriptionRequestType(263)`.
pub const MARKETDATATYPE_MSGTYPE_RULES: &[(&str, &[i32])] = &[
    ("AE", &[856, 828, 40]),
    ("AR", &[856, 828, 40]),
    ("R", &[303, 537, 40]),
    ("AG", &[303, 537]),
    ("q", &[530]),
    ("r", &[530]),
    ("V", &[263]),
];

impl MarketDataType {
    /// The member one FIX field's wire value types an element as: the
    /// member whose [`Self::fix_code`] it is, else the catch-all of the set
    /// the field draws from (`ORDOTHER` for an `OrdType(40)` no member
    /// names); `None` for a field that types nothing.
    #[must_use]
    pub fn from_fix(tag: i32, wire: &str) -> Option<Self> {
        let catch_all = match tag {
            40 => Self::OrdOther,
            537 => Self::QuoOther,
            828 => Self::TrdOther,
            269 => Self::BookOther,
            856 => Self::TrptOther,
            303 => Self::QrqOther,
            530 => Self::McxOther,
            263 => Self::MdrOther,
            _ => return None,
        };
        Some(
            FIX_CODES
                .iter()
                .find(|(held, code, _)| *held == tag && *code == wire)
                .map_or(catch_all, |(_, _, member)| *member),
        )
    }

    /// The FIX field and wire value this member stands for - `(40, "2")`
    /// for `ORDLIMIT` - or `None` for `UKNW` and a catch-all, which stand
    /// for no one value.
    #[must_use]
    pub fn fix_code(self) -> Option<(i32, &'static str)> {
        FIX_CODES
            .iter()
            .find(|(_, _, member)| *member == self)
            .map(|(tag, code, _)| (*tag, *code))
    }

    /// The FIX fields that type a message of type `msgtype` filed under
    /// `kind`, first stated first: the message type's own rule where
    /// [`MARKETDATATYPE_MSGTYPE_RULES`] states one, else its kind's
    /// ([`Self::fix_tags`]).
    #[must_use]
    pub fn fix_tags_of(msgtype: &str, kind: crate::MarketDataKind) -> &'static [i32] {
        MARKETDATATYPE_MSGTYPE_RULES
            .iter()
            .find(|(held, _)| *held == msgtype)
            .map_or_else(|| Self::fix_tags(kind), |(_, tags)| *tags)
    }

    /// The FIX fields that type an element of `kind`, the one its kind
    /// names first: an order's `OrdType(40)`, a quote's `QuoteType(537)`,
    /// then `OrdType(40)`, an execution's or a trade's `TrdType(828)`,
    /// then `OrdType(40)`, and every other element's in
    /// [`MARKETDATATYPE_FIX_TAGS`] order with `MDEntryType(269)` first.
    #[must_use]
    pub const fn fix_tags(kind: crate::MarketDataKind) -> &'static [i32] {
        use crate::MarketDataKind as Kind;
        match kind {
            Kind::Order | Kind::OrderBatch => &[40],
            Kind::Quotation | Kind::QuoteBatch => &[537, 40],
            Kind::Execution | Kind::ExecutionBatch | Kind::Trade | Kind::TradeBatch => &[828, 40],
            _ => &[269, 828, 537, 40],
        }
    }

    /// The member one spelling names, or `None` where none does.
    ///
    /// Two vocabularies reach one value: the stored name in any case -
    /// `ORDLIMIT`, `ordlimit` - and the FIX specification's own name for
    /// the value, folded the way every name in this crate folds - `Limit`,
    /// `market_if_touched`, `BLOCK TRADE`. A wire value is not a spelling:
    /// `2` is a limit order only under `OrdType(40)`, which
    /// [`Self::from_fix`] reads. A stored code is an integer, never text:
    /// [`Self::from_code`] reads it.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(held) = Self::from_name(spelling) {
            return Some(held);
        }
        let folded = folded_spelling(spelling);
        Self::ALL
            .iter()
            .copied()
            .find(|member| member.as_str().eq_ignore_ascii_case(folded.as_str()))
            .or_else(|| {
                FIX_NAMES
                    .iter()
                    .find(|(name, _)| *name == folded.as_str())
                    .map(|(_, member)| *member)
            })
            .or_else(|| Self::from_pattern(spelling))
    }
}

/// The names a market data type goes by beside its stored one, which its
/// spelling patterns read the words of.
fn marketdatatype_aliases() -> Vec<(&'static str, MarketDataType)> {
    FIX_NAMES.to_vec()
}

/// Every FIX value a member stands for: the field, the wire value, the
/// member. A catch-all stands for none.
static FIX_CODES: &[(i32, &str, MarketDataType)] = &[
    (40, "1", MarketDataType::OrdMarket),
    (40, "2", MarketDataType::OrdLimit),
    (40, "3", MarketDataType::OrdStop),
    (40, "4", MarketDataType::OrdStopLimit),
    (40, "5", MarketDataType::OrdMarketOnClose),
    (40, "6", MarketDataType::OrdWithOrWithout),
    (40, "7", MarketDataType::OrdLimitOrBetter),
    (40, "8", MarketDataType::OrdLimitWithOrWithout),
    (40, "9", MarketDataType::OrdOnBasis),
    (40, "A", MarketDataType::OrdOnClose),
    (40, "B", MarketDataType::OrdLimitOnClose),
    (40, "C", MarketDataType::OrdFxMarket),
    (40, "D", MarketDataType::OrdPrevQuoted),
    (40, "E", MarketDataType::OrdPrevIndicated),
    (40, "F", MarketDataType::OrdFxLimit),
    (40, "G", MarketDataType::OrdFxSwap),
    (40, "H", MarketDataType::OrdFxPrevQuoted),
    (40, "I", MarketDataType::OrdFunari),
    (40, "J", MarketDataType::OrdMarketIfTouched),
    (40, "K", MarketDataType::OrdMarketToLimit),
    (40, "L", MarketDataType::OrdPrevFundPoint),
    (40, "M", MarketDataType::OrdNextFundPoint),
    (40, "P", MarketDataType::OrdPegged),
    (40, "Q", MarketDataType::OrdCounterSelection),
    (40, "R", MarketDataType::OrdStopOnBidOffer),
    (40, "S", MarketDataType::OrdStopLimitOnBidOffer),
    (40, "T", MarketDataType::OrdMarketInBand),
    (537, "0", MarketDataType::QuoIndicative),
    (537, "1", MarketDataType::QuoTradeable),
    (537, "2", MarketDataType::QuoRestricted),
    (537, "3", MarketDataType::QuoCounter),
    (537, "4", MarketDataType::QuoInitTradeable),
    (828, "0", MarketDataType::TrdRegular),
    (828, "1", MarketDataType::TrdBlock),
    (828, "2", MarketDataType::TrdEfp),
    (828, "3", MarketDataType::TrdTransfer),
    (828, "4", MarketDataType::TrdLate),
    (828, "5", MarketDataType::TrdTTrade),
    (828, "6", MarketDataType::TrdWap),
    (828, "7", MarketDataType::TrdBunched),
    (828, "8", MarketDataType::TrdLateBunched),
    (828, "9", MarketDataType::TrdPriorRef),
    (828, "10", MarketDataType::TrdAfterHours),
    (828, "11", MarketDataType::TrdEfr),
    (828, "12", MarketDataType::TrdEfs),
    (828, "15", MarketDataType::TrdAtSettlement),
    (828, "16", MarketDataType::TrdAllOrNone),
    (828, "24", MarketDataType::TrdError),
    (828, "38", MarketDataType::TrdLarge),
    (828, "45", MarketDataType::TrdOptionExercise),
    (828, "50", MarketDataType::TrdPortfolio),
    (828, "51", MarketDataType::TrdVwap),
    (828, "54", MarketDataType::TrdOtc),
    (828, "56", MarketDataType::TrdOpening),
    (828, "57", MarketDataType::TrdNetted),
    (828, "62", MarketDataType::TrdDark),
    (828, "63", MarketDataType::TrdTechnical),
    (828, "64", MarketDataType::TrdBenchmark),
    (828, "65", MarketDataType::TrdPackage),
    (828, "66", MarketDataType::TrdRoll),
    (828, "67", MarketDataType::TrdClosingPrice),
    (269, "0", MarketDataType::BookBid),
    (269, "1", MarketDataType::BookOffer),
    (269, "2", MarketDataType::BookTrade),
    (269, "3", MarketDataType::BookIndex),
    (269, "4", MarketDataType::BookOpen),
    (269, "5", MarketDataType::BookClose),
    (269, "6", MarketDataType::BookSettle),
    (269, "7", MarketDataType::BookHigh),
    (269, "8", MarketDataType::BookLow),
    (269, "9", MarketDataType::BookVwap),
    (269, "A", MarketDataType::BookImbalance),
    (269, "B", MarketDataType::BookVolume),
    (269, "C", MarketDataType::BookOpenInterest),
    (269, "H", MarketDataType::BookMid),
    (269, "J", MarketDataType::BookEmpty),
    (856, "0", MarketDataType::TrptSubmit),
    (856, "1", MarketDataType::TrptAlleged),
    (856, "2", MarketDataType::TrptAccept),
    (856, "3", MarketDataType::TrptDecline),
    (856, "4", MarketDataType::TrptAddendum),
    (856, "5", MarketDataType::TrptNoWas),
    (856, "6", MarketDataType::TrptCancel),
    (856, "7", MarketDataType::TrptBreak),
    (856, "8", MarketDataType::TrptDefaulted),
    (856, "9", MarketDataType::TrptInvalidCmta),
    (856, "10", MarketDataType::TrptPended),
    (856, "11", MarketDataType::TrptAllegedNew),
    (856, "12", MarketDataType::TrptAllegedAddendum),
    (856, "13", MarketDataType::TrptAllegedNoWas),
    (856, "14", MarketDataType::TrptAllegedCancel),
    (856, "15", MarketDataType::TrptAllegedBreak),
    (303, "1", MarketDataType::QrqManual),
    (303, "2", MarketDataType::QrqAutomatic),
    (530, "1", MarketDataType::McxSecurity),
    (530, "2", MarketDataType::McxUnderlying),
    (530, "3", MarketDataType::McxProduct),
    (530, "4", MarketDataType::McxCfi),
    (530, "5", MarketDataType::McxSecurityType),
    (530, "6", MarketDataType::McxSession),
    (530, "7", MarketDataType::McxAll),
    (530, "8", MarketDataType::McxMarket),
    (530, "9", MarketDataType::McxSegment),
    (530, "A", MarketDataType::McxGroup),
    (530, "B", MarketDataType::McxIssuer),
    (530, "C", MarketDataType::McxUnderlyingIssuer),
    (263, "0", MarketDataType::MdrSnapshot),
    (263, "1", MarketDataType::MdrSubscribe),
    (263, "2", MarketDataType::MdrUnsubscribe),
];

/// The FIX specification's names that reach one member alone, folded; a
/// name two sets share - `Counter` - reaches neither and is read by its
/// stored name instead.
static FIX_NAMES: &[(&str, MarketDataType)] = &[
    ("market", MarketDataType::OrdMarket),
    ("limit", MarketDataType::OrdLimit),
    ("stop", MarketDataType::OrdStop),
    ("stoplimit", MarketDataType::OrdStopLimit),
    ("marketonclose", MarketDataType::OrdMarketOnClose),
    ("withorwithout", MarketDataType::OrdWithOrWithout),
    ("limitorbetter", MarketDataType::OrdLimitOrBetter),
    ("limitwithorwithout", MarketDataType::OrdLimitWithOrWithout),
    ("onbasis", MarketDataType::OrdOnBasis),
    ("onclose", MarketDataType::OrdOnClose),
    ("limitonclose", MarketDataType::OrdLimitOnClose),
    ("forexmarket", MarketDataType::OrdFxMarket),
    ("previouslyquoted", MarketDataType::OrdPrevQuoted),
    ("previouslyindicated", MarketDataType::OrdPrevIndicated),
    ("forexlimit", MarketDataType::OrdFxLimit),
    ("forexswap", MarketDataType::OrdFxSwap),
    ("forexpreviouslyquoted", MarketDataType::OrdFxPrevQuoted),
    ("funari", MarketDataType::OrdFunari),
    ("marketiftouched", MarketDataType::OrdMarketIfTouched),
    (
        "marketwithleftoveraslimit",
        MarketDataType::OrdMarketToLimit,
    ),
    (
        "previousfundvaluationpoint",
        MarketDataType::OrdPrevFundPoint,
    ),
    ("nextfundvaluationpoint", MarketDataType::OrdNextFundPoint),
    ("pegged", MarketDataType::OrdPegged),
    ("counterorderselection", MarketDataType::OrdCounterSelection),
    ("stoponbidoroffer", MarketDataType::OrdStopOnBidOffer),
    (
        "stoplimitonbidoroffer",
        MarketDataType::OrdStopLimitOnBidOffer,
    ),
    ("marketwithinpriceband", MarketDataType::OrdMarketInBand),
    ("indicative", MarketDataType::QuoIndicative),
    ("tradeable", MarketDataType::QuoTradeable),
    ("restrictedtradeable", MarketDataType::QuoRestricted),
    ("initiallytradeable", MarketDataType::QuoInitTradeable),
    ("regulartrade", MarketDataType::TrdRegular),
    ("blocktrade", MarketDataType::TrdBlock),
    ("efp", MarketDataType::TrdEfp),
    ("transfer", MarketDataType::TrdTransfer),
    ("latetrade", MarketDataType::TrdLate),
    ("ttrade", MarketDataType::TrdTTrade),
    ("weightedaveragepricetrade", MarketDataType::TrdWap),
    ("bunchedtrade", MarketDataType::TrdBunched),
    ("latebunchedtrade", MarketDataType::TrdLateBunched),
    ("priorreferencepricetrade", MarketDataType::TrdPriorRef),
    ("afterhourstrade", MarketDataType::TrdAfterHours),
    ("exchangeforrisk", MarketDataType::TrdEfr),
    ("exchangeforswap", MarketDataType::TrdEfs),
    ("tradingatsettlement", MarketDataType::TrdAtSettlement),
    ("allornone", MarketDataType::TrdAllOrNone),
    ("errortrade", MarketDataType::TrdError),
    ("largetrade", MarketDataType::TrdLarge),
    ("optionexercise", MarketDataType::TrdOptionExercise),
    ("portfoliotrade", MarketDataType::TrdPortfolio),
    ("volumeweightedaveragetrade", MarketDataType::TrdVwap),
    ("otc", MarketDataType::TrdOtc),
    ("openingtrade", MarketDataType::TrdOpening),
    ("nettedtrade", MarketDataType::TrdNetted),
    ("darktrade", MarketDataType::TrdDark),
    ("technicaltrade", MarketDataType::TrdTechnical),
    ("benchmark", MarketDataType::TrdBenchmark),
    ("packagetrade", MarketDataType::TrdPackage),
    ("rolltrade", MarketDataType::TrdRoll),
    ("closingpricetrade", MarketDataType::TrdClosingPrice),
    ("bid", MarketDataType::BookBid),
    ("offer", MarketDataType::BookOffer),
    ("trade", MarketDataType::BookTrade),
    ("indexvalue", MarketDataType::BookIndex),
    ("openingprice", MarketDataType::BookOpen),
    ("closingprice", MarketDataType::BookClose),
    ("settlementprice", MarketDataType::BookSettle),
    ("tradingsessionhighprice", MarketDataType::BookHigh),
    ("tradingsessionlowprice", MarketDataType::BookLow),
    ("vwap", MarketDataType::BookVwap),
    ("imbalance", MarketDataType::BookImbalance),
    ("tradevolume", MarketDataType::BookVolume),
    ("openinterest", MarketDataType::BookOpenInterest),
    ("midprice", MarketDataType::BookMid),
    ("emptybook", MarketDataType::BookEmpty),
    ("submit", MarketDataType::TrptSubmit),
    ("alleged", MarketDataType::TrptAlleged),
    ("accept", MarketDataType::TrptAccept),
    ("decline", MarketDataType::TrptDecline),
    ("addendum", MarketDataType::TrptAddendum),
    ("nowas", MarketDataType::TrptNoWas),
    ("tradereportcancel", MarketDataType::TrptCancel),
    ("lockedintradebreak", MarketDataType::TrptBreak),
    ("defaulted", MarketDataType::TrptDefaulted),
    ("invalidcmta", MarketDataType::TrptInvalidCmta),
    ("pended", MarketDataType::TrptPended),
    ("allegednew", MarketDataType::TrptAllegedNew),
    ("allegedaddendum", MarketDataType::TrptAllegedAddendum),
    ("allegednowas", MarketDataType::TrptAllegedNoWas),
    (
        "allegedtradereportcancel",
        MarketDataType::TrptAllegedCancel,
    ),
    ("allegedtradebreak", MarketDataType::TrptAllegedBreak),
    ("manual", MarketDataType::QrqManual),
    ("automatic", MarketDataType::QrqAutomatic),
    ("cancelordersforasecurity", MarketDataType::McxSecurity),
    (
        "cancelordersforanunderlyingsecurity",
        MarketDataType::McxUnderlying,
    ),
    ("cancelordersforaproduct", MarketDataType::McxProduct),
    ("cancelordersforacficode", MarketDataType::McxCfi),
    (
        "cancelordersforasecuritytype",
        MarketDataType::McxSecurityType,
    ),
    ("cancelordersforatradingsession", MarketDataType::McxSession),
    ("cancelallorders", MarketDataType::McxAll),
    ("cancelordersforamarket", MarketDataType::McxMarket),
    ("cancelordersforamarketsegment", MarketDataType::McxSegment),
    ("cancelordersforasecuritygroup", MarketDataType::McxGroup),
    ("cancelordersforsecuritiesissuer", MarketDataType::McxIssuer),
    (
        "cancelordersforissuerofunderlyingsecurity",
        MarketDataType::McxUnderlyingIssuer,
    ),
    ("snapshot", MarketDataType::MdrSnapshot),
    ("snapshotandupdates", MarketDataType::MdrSubscribe),
    ("disablepreviousrequest", MarketDataType::MdrUnsubscribe),
];

/// The Arrow extension name of a market data type, over `uint16` storage.
pub(crate) const MARKETDATATYPE_EXTENSION_NAME: &str = "yggdryl.marketdatatype";

// /// A field declared as a market data element's type.
define_field_types!(
    MarketDataTypeType,
    MarketDataTypeField,
    market = MARKETDATATYPE_KIND,
    MarketDataType
);
