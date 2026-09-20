//! Node.js view of the market's products: what the market did, read out of
//! what a venue said.
//!
//! Five values, one class each, and one stream each. An `Order` is one
//! order's life, a `Quote` a price stated at an instant, an `Execution` one
//! fill, a `Trade` the settled transaction an execution reports, and a
//! `Book` the ladder for one instrument at one instant. Each is a value of
//! the core with its own row and its own identity - not a view over a
//! message - and each is a graph event exactly as a message is, so its
//! sixteen event facts cross as [`JsFixMsg`]'s do: a UUID as its hyphenated
//! text, a hash and an instant as a `bigint`, the instants nanoseconds
//! since the Unix epoch, UTC. The market facts its row publishes cross as
//! [`FixEventView`] crosses them: a price or a quantity as its decimal
//! text, a currency, a side and an instrument code as the text each is,
//! and an absent fact as `null`. A product's `srcuuids` are the identities
//! of the messages it was read from, which is the whole interop story: its
//! table joins the message table on `srcuuids` to `curruuid` with no
//! mapping.
//!
//! The doors are the codec's - one per product over a stream of messages,
//! pulled one message at a time exactly as `lifecycle` pulls them, and one
//! over batches of message rows - and the writers are the message's:
//! `FixMsg.fromOrder` and `FixMsg.fromExecution` state the two products
//! one message states exactly, and the other three refuse with the core's
//! sentence, because the crate does not guess.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use napi::bindgen_prelude::{
    BigInt, ClassInstance, Either, Either4, Either5, Env, Function, Result,
};
use napi_derive::napi;
use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{
    Book as CoreBook, BookData, BookIterator as CoreBookIterator, Execution as CoreExecution,
    ExecutionData, Level, Order as CoreOrder, OrderData, Party, Pricing, Product,
    Quote as CoreQuote, QuoteData, Statement, Symbol as CoreSymbol, Trade as CoreTrade, TradeData,
};
use yggdryl::{
    Bloomberg, Cfi, Currency, Cusip, Decimal18, FixMsg as CoreFixMsg, Isin, Mic, Sedol, Side,
};

use crate::fix::{
    Failed, FixEventView, JsFixCodec, JsFixMsg, Pulled, event_view, identifiers_view, instant,
    parents_view, sources_view,
};
use crate::iomedia::JsBatchReader;
use crate::text::codec::JsScalar;
use crate::types::field::JsField;
use crate::{exact_f64, exact_i64, exact_u32, napi_error};

/// One party to a trade, as the plain object JavaScript reads.
///
/// FIX's `Parties` occurrence, held as the text the venue stated: the role
/// as the message spells `PartyRole(452)`, the identifier `PartyID(448)`,
/// and the scheme `PartyIDSource(447)` issued it under, empty where none
/// was stated.
#[napi(object, object_from_js = false)]
pub struct MarketPartyView {
    /// The role the party plays, as the message spells it.
    pub role: String,
    /// The party's identifier under the source that issued it.
    pub id: String,
    /// The scheme the identifier is issued under; empty where none.
    pub source: String,
}

/// One level of a ladder, as the plain object JavaScript reads.
#[napi(object, object_from_js = false)]
pub struct MarketLevelView {
    /// The price of the level, as decimal text.
    pub px: String,
    /// The size resting at it, as decimal text: what the orders and quote
    /// lanes there have left, summed.
    pub qty: String,
    /// How many orders and quote lanes rest at it.
    pub count: f64,
}

/// One lane a quote states, as the plain object JavaScript reads: what the
/// quoter would pay or be paid, and for how much, both above zero.
#[napi(object, object_from_js = false)]
pub struct MarketLaneView {
    /// The price of the lane, as decimal text.
    pub px: String,
    /// The size quoted at it, as decimal text.
    pub qty: String,
}

fn lane_view(lane: Option<(Decimal18, Decimal18)>) -> Option<MarketLaneView> {
    lane.map(|(px, qty)| MarketLaneView {
        px: px.to_string(),
        qty: qty.to_string(),
    })
}

/// One level as the plain object JavaScript reads; a count is a JavaScript
/// number, exact to 2^53, as every count at this boundary is.
fn level_view(level: &Level) -> Result<MarketLevelView> {
    Ok(MarketLevelView {
        px: level.px.to_string(),
        qty: level.qty.to_string(),
        count: exact_f64(level.count, "count")?,
    })
}

/// A side as `Side.read` reads a spelling: a FIX `Side(54)` code, the
/// specification's name, or the stored value.
fn side_of(side: &str) -> Result<Side> {
    Side::read(side).map_err(napi_error)
}

/// A depth as the non-zero one a book is read to.
fn book_depth(depth: f64) -> Result<NonZeroU32> {
    NonZeroU32::new(exact_u32(depth, "depth")?).ok_or_else(|| {
        napi_error(
            "depth must be at least one level: a book is a ladder to a declared depth, and no \
             ladder is no book",
        )
    })
}

fn party_view(party: &Party) -> MarketPartyView {
    MarketPartyView {
        role: party.role.clone(),
        id: party.id.clone(),
        source: party.source.clone(),
    }
}

/// A ladder's levels, best first.
fn levels_view(levels: &[Level]) -> Result<Vec<MarketLevelView>> {
    levels.iter().map(level_view).collect()
}

/// One decimal fact as its text, or `null` where the product states none.
fn decimal(held: Option<Decimal18>) -> Option<String> {
    held.map(|value| value.to_string())
}

/// One text fact owned, or `null` where the product states none.
fn text(held: Option<&str>) -> Option<String> {
    held.map(ToOwned::to_owned)
}

/// The largest integer a JavaScript number carries exactly.
const SAFE_INTEGER: f64 = 9_007_199_254_740_992.0;

/// A grid step as JavaScript spells an instant: a `bigint`, or a whole
/// `number` of at most 2^53, nanoseconds either way.
fn nanoseconds(value: Either<BigInt, f64>, name: &str) -> Result<i64> {
    match value {
        Either::A(held) => {
            let (count, lossless) = held.get_i64();
            if !lossless {
                return Err(napi_error(format!(
                    "{name} must fit a signed 64-bit nanosecond count"
                )));
            }
            Ok(count)
        }
        Either::B(held) => exact_i64(held, name),
    }
}

/// A document as `JSON.parse` reads it: an integer beyond 2^53 - a hash
/// code, an instant - is the double a JSON reader answers, as
/// `Scalar.toJSON` answers it, never the `bigint` `JSON.stringify` refuses.
fn as_json_reads(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(held) if !held.is_f64() => {
            if let Some(double) = held.as_f64().filter(|double| double.abs() > SAFE_INTEGER) {
                *value = serde_json::Number::from_f64(double)
                    .map_or(serde_json::Value::Null, serde_json::Value::Number);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(as_json_reads),
        serde_json::Value::Object(members) => members.values_mut().for_each(as_json_reads),
        _ => {}
    }
}

/// The row's two documents: its schema and the row itself, as `FixMsg`
/// renders its own.
fn row_document<P: Product>(product: &P) -> Result<serde_json::Value> {
    let field =
        serde_json::to_value(product.row_field().map_err(napi_error)?).map_err(napi_error)?;
    let row = product.into_row().map_err(napi_error)?;
    let mut value: serde_json::Value =
        serde_json::from_str(&yggdryl::into_json_scalar(&row).map_err(napi_error)?)
            .map_err(napi_error)?;
    as_json_reads(&mut value);
    let mut document = serde_json::Map::with_capacity(2);
    document.insert("field".to_owned(), field);
    document.insert("value".to_owned(), value);
    Ok(serde_json::Value::Object(document))
}

/// The sixteen event facts every product answers, crossed as `FixMsg`
/// crosses them, with the doors every product has: its event as one plain
/// object, its row and the row's inverse, its equality and its renderings.
macro_rules! event_facts {
    ($js:ident wraps $core:ty as $name:literal) => {
        impl $js {
            /// Wrap a product the core read.
            pub(crate) const fn from_core(inner: $core) -> Self {
                Self { inner }
            }
        }

        #[napi]
        impl $js {
            /// The product one row of `field` states: the inverse of
            /// `intoRow`, the row canonicalized under the field first, so a
            /// code spelled as text and a number spelled at another scale
            /// read as what the column types. The loader widens `row` from
            /// whatever `Scalar.from` reads.
            #[napi(factory)]
            pub fn from_row(field: &JsField, row: &JsScalar) -> Result<Self> {
                <$core>::from_row(&field.inner, &row.inner)
                    .map(Self::from_core)
                    .map_err(napi_error)
            }

            /// This product's own identity, as its hyphenated text: a time
            /// UUID over its instant and its `currhashcode`.
            #[napi(getter)]
            pub fn curruuid(&self) -> String {
                self.inner.get_curruuid().to_string()
            }

            /// The identity of the chain this product belongs to, as its
            /// hyphenated text: `curruuid` when no cross code names a chain.
            #[napi(getter)]
            pub fn crossuuid(&self) -> String {
                self.inner.get_crossuuid().to_string()
            }

            /// The code the chain is named by, or empty.
            #[napi(getter)]
            pub fn crosscode(&self) -> String {
                self.inner.get_crosscode().to_owned()
            }

            /// The XXH3-64 over everything this product states.
            #[napi(getter)]
            pub fn currhashcode(&self) -> BigInt {
                BigInt::from(self.inner.get_currhashcode())
            }

            /// The XXH3-64 of the cross code, `0n` where there is none.
            #[napi(getter)]
            pub fn crosshashcode(&self) -> BigInt {
                BigInt::from(self.inner.get_crosshashcode())
            }

            /// The identifiers the product is known by, scheme to value,
            /// sorted.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn identifiers(&self) -> BTreeMap<String, String> {
                identifiers_view(self.inner.event())
            }

            /// The identities of the statements this one descends from: the
            /// whole chain before it, oldest first.
            #[napi(getter)]
            pub fn parentuuids(&self) -> Vec<String> {
                parents_view(self.inner.event())
            }

            /// The identities of the messages this product was read from.
            /// Provenance, never lineage: no walk moves it.
            #[napi(getter)]
            pub fn srcuuids(&self) -> Vec<String> {
                sources_view(self.inner.event())
            }

            /// When the product was stated, nanoseconds since the Unix
            /// epoch, UTC.
            #[napi(getter)]
            pub fn currunix(&self) -> BigInt {
                instant(self.inner.get_currunix())
            }

            /// The state the chain reached, ranked: `00UNKNOWN` where it
            /// states none.
            #[napi(getter)]
            pub fn state(&self) -> String {
                self.inner.get_state().as_str().to_owned()
            }

            /// The product's place in its chain, `0` until a walk states it.
            #[napi(getter)]
            pub fn seqnum(&self) -> Result<f64> {
                exact_f64(self.inner.get_seqnum(), "seqnum")
            }

            /// When the chain was created, or `null`.
            #[napi(getter)]
            pub fn creaunix(&self) -> Option<BigInt> {
                self.inner.get_creaunix().map(instant)
            }

            /// When the chain expires, or `null`.
            #[napi(getter)]
            pub fn expirunix(&self) -> Option<BigInt> {
                self.inner.get_expirunix().map(instant)
            }

            /// The instant of the statement this one follows, or `null`.
            #[napi(getter)]
            pub fn prevunix(&self) -> Option<BigInt> {
                self.inner.get_prevunix().map(instant)
            }

            /// The identity of the statement this one follows, or `null`.
            #[napi(getter)]
            pub fn prevuuid(&self) -> Option<String> {
                self.inner.get_prevuuid().map(|uuid| uuid.to_string())
            }

            /// The grid step this product is the snapshot of, or `null`.
            #[napi(getter)]
            pub fn snapunix(&self) -> Option<BigInt> {
                self.inner.get_snapunix().map(instant)
            }

            /// Whether this product can still be followed: its state can
            /// still change, and it is not past its expiration.
            #[napi(getter)]
            pub fn is_alive(&self) -> bool {
                self.inner.is_alive()
            }

            /// The event this product is: every fact the graph traits
            /// answer, as one plain object read once.
            #[napi]
            pub fn event(&self) -> Result<FixEventView> {
                event_view(self.inner.event())
            }

            /// This product as one row of `field()`: the sixteen event
            /// columns, the market columns it publishes, then its own, every
            /// cell the raw value its column types and null where the
            /// product states no fact.
            #[napi]
            pub fn into_row(&self) -> Result<JsScalar> {
                self.inner
                    .into_row()
                    .map(JsScalar::from_core)
                    .map_err(napi_error)
            }

            /// Whether two statements carry the same facts.
            #[napi]
            pub fn equals(&self, other: &$js) -> bool {
                self.inner == other.inner
            }

            /// A one-line summary: the chain, the state and the instant.
            #[napi(js_name = "toString")]
            pub fn js_string(&self) -> String {
                format!(
                    "{}({:?}, {}, {})",
                    $name,
                    self.inner.get_crosscode(),
                    self.inner.get_state().as_str(),
                    self.inner.get_currunix()
                )
            }

            /// The row's schema document and value document.
            #[napi(js_name = "toJSON")]
            pub fn js_json(&self) -> Result<serde_json::Value> {
                row_document(&self.inner)
            }
        }
    };
}

/// The instrument a product names: the ticker and the six codes, each
/// `null` where the product states none.
macro_rules! instrument_facts {
    ($js:ident) => {
        #[napi]
        impl $js {
            /// The ticker the instrument is known by, or `null` where it has
            /// none and the codes beside it are what name it.
            #[napi(getter)]
            pub fn symbolticker(&self) -> Option<String> {
                text(self.inner.get_symbolticker())
            }

            /// The instrument's ISIN, or `null`.
            #[napi(getter)]
            pub fn isincode(&self) -> Option<String> {
                text(self.inner.get_isincode().map(Isin::as_str))
            }

            /// The instrument's CUSIP, or `null`.
            #[napi(getter)]
            pub fn cusipcode(&self) -> Option<String> {
                text(self.inner.get_cusipcode().map(Cusip::as_str))
            }

            /// The instrument's SEDOL, or `null`.
            #[napi(getter)]
            pub fn sedolcode(&self) -> Option<String> {
                text(self.inner.get_sedolcode().map(Sedol::as_str))
            }

            /// The instrument's Bloomberg identifier, or `null`.
            #[napi(getter)]
            pub fn bloombergcode(&self) -> Option<String> {
                text(self.inner.get_bloombergcode().map(Bloomberg::as_str))
            }

            /// The instrument's CFI classification, or `null`.
            #[napi(getter)]
            pub fn cficode(&self) -> Option<String> {
                text(self.inner.get_cficode().map(Cfi::as_str))
            }

            /// The market the product names, an ISO 10383 MIC, or `null`.
            #[napi(getter)]
            pub fn miccode(&self) -> Option<String> {
                text(self.inner.get_miccode().map(Mic::as_str))
            }
        }
    };
}

/// The one price, quantity and side a product is about, with the currency
/// and the unit it is counted in.
macro_rules! lane_facts {
    ($js:ident) => {
        #[napi]
        impl $js {
            /// The price, as decimal text; `0` where none is stated.
            #[napi(getter)]
            pub fn px(&self) -> String {
                self.inner.get_px().to_string()
            }

            /// The quantity, as decimal text; `0` where none is stated.
            #[napi(getter)]
            pub fn qty(&self) -> String {
                self.inner.get_qty().to_string()
            }

            /// The side: `BUY`, `SELL`, or `UNKNOWN`.
            #[napi(getter)]
            pub fn side(&self) -> String {
                self.inner.get_side().as_str().to_owned()
            }

            /// The currency; `XXX` where none is stated.
            #[napi(getter)]
            pub fn currency(&self) -> String {
                self.inner.get_currency().as_str().to_owned()
            }

            /// The unit the quantity is counted in, empty where none is
            /// stated.
            #[napi(getter)]
            pub fn unit(&self) -> String {
                self.inner.get_unit().to_owned()
            }

            /// What the product is worth at its price, `px * qty` as decimal
            /// text, or `null` where either is zero.
            #[napi(getter)]
            pub fn notional(&self) -> Option<String> {
                decimal(self.inner.notional())
            }
        }
    };
}

/// One order's life: its placement and every report against it, chained.
///
/// The chain is the order's, under the identifier the venue gave it, and
/// the row publishes the order's price ladder - the price, the average,
/// the quantity against what filled and what is left - the side, the
/// currency and the unit, how long it stands, whether the instrument could
/// be traded, the instrument, and the order's own stop price.
#[napi(js_name = "Order")]
pub struct JsOrder {
    inner: OrderData,
}

event_facts!(JsOrder wraps OrderData as "Order");
lane_facts!(JsOrder);
instrument_facts!(JsOrder);

#[napi]
impl JsOrder {
    /// The row every order publishes, a non-null Struct named `order`.
    #[napi]
    pub fn field() -> Result<JsField> {
        OrderData::field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// The price it averaged, `AvgPx(6)`, as decimal text, or `null`.
    #[napi(getter)]
    pub fn avgpx(&self) -> Option<String> {
        decimal(self.inner.get_avgpx())
    }

    /// How much of its quantity is done, `CumQty(14)`, or `null`.
    #[napi(getter)]
    pub fn cumqty(&self) -> Option<String> {
        decimal(self.inner.get_cumqty())
    }

    /// How much of it is still open, `LeavesQty(151)`, or `null`.
    #[napi(getter)]
    pub fn leavesqty(&self) -> Option<String> {
        decimal(self.inner.get_leavesqty())
    }

    /// How long the order stands, `TimeInForce(59)`, as it states it, or
    /// `null`.
    #[napi(getter)]
    pub fn tif(&self) -> Option<String> {
        text(self.inner.get_tif())
    }

    /// Whether the instrument could be traded when the order was stated,
    /// or `null` where the market said nothing either way.
    #[napi(getter)]
    pub fn tradable(&self) -> Option<bool> {
        self.inner.get_tradable()
    }

    /// The stop price, `StopPx(99)`, as decimal text, or `null`.
    #[napi(getter)]
    pub fn stoppx(&self) -> Option<String> {
        decimal(self.inner.get_stoppx())
    }

    /// The type the order states, `OrdType(40)` as the wire spells it -
    /// `1` market, `2` limit, `3` stop, `4` stop limit - or `null`.
    #[napi(getter)]
    pub fn ordtype(&self) -> Option<String> {
        text(self.inner.get_ordtype())
    }

    /// What the order prices: `market`, `limit`, `stop` or `stoplimit` -
    /// the type it states, else what its limit and stop imply - or `null`
    /// where it states a type outside the four.
    #[napi(getter)]
    pub fn pricing(&self) -> Option<String> {
        self.inner.pricing().map(|pricing| {
            match pricing {
                Pricing::Market => "market",
                Pricing::Limit => "limit",
                Pricing::Stop => "stop",
                Pricing::StopLimit => "stoplimit",
            }
            .to_owned()
        })
    }

    /// What is left to trade, as decimal text: what the order states is
    /// left, else what it ordered less what filled.
    #[napi(getter)]
    pub fn remaining(&self) -> String {
        self.inner.remaining().to_string()
    }

    /// What traded, as decimal text: what the order states filled, else
    /// what it ordered less what is left.
    #[napi(getter)]
    pub fn filled(&self) -> String {
        self.inner.filled().to_string()
    }

    /// How much of what was ordered traded, `filled / qty` as decimal text,
    /// or `null` where nothing was ordered.
    #[napi(getter)]
    pub fn filled_ratio(&self) -> Option<String> {
        decimal(self.inner.filled_ratio())
    }

    /// Whether the order rests on a ladder right now: alive, on a side that
    /// takes a lane, a limit above zero, something left.
    #[napi(getter)]
    pub fn is_resting(&self) -> bool {
        self.inner.is_resting()
    }
}

/// One fill: the one execution report that states it, in the chain of the
/// order it fills.
#[napi(js_name = "Execution")]
pub struct JsExecution {
    inner: ExecutionData,
}

event_facts!(JsExecution wraps ExecutionData as "Execution");
lane_facts!(JsExecution);
instrument_facts!(JsExecution);

#[napi]
impl JsExecution {
    /// The row every execution publishes, a non-null Struct named
    /// `execution`.
    #[napi]
    pub fn field() -> Result<JsField> {
        ExecutionData::field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Whether the fill left its order open: the order's status after it
    /// is still live.
    #[napi(getter)]
    pub fn is_partial(&self) -> bool {
        self.inner.is_partial()
    }

    /// Whether the fill completed its order: the order's status after it
    /// is done.
    #[napi(getter)]
    pub fn completes(&self) -> bool {
        self.inner.completes()
    }
}

/// The settled transaction an execution reports: the matched quantity at
/// its price, its parties and its clocks, the two sides' reports of one
/// match in one chain.
#[napi(js_name = "Trade")]
pub struct JsTrade {
    inner: TradeData,
}

event_facts!(JsTrade wraps TradeData as "Trade");
lane_facts!(JsTrade);
instrument_facts!(JsTrade);

#[napi]
impl JsTrade {
    /// The row every trade publishes, a non-null Struct named `trade`.
    #[napi]
    pub fn field() -> Result<JsField> {
        TradeData::field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// The day the trade was done, `TradeDate(75)`, as days since the Unix
    /// epoch, or `null`.
    #[napi(getter)]
    pub fn tradedate(&self) -> Option<i32> {
        self.inner.get_tradedate()
    }

    /// The day the trade settles, `SettlDate(64)`, as days since the Unix
    /// epoch, or `null`.
    #[napi(getter)]
    pub fn settldate(&self) -> Option<i32> {
        self.inner.get_settldate()
    }

    /// The parties to the trade, in the order the report states them.
    #[napi(getter)]
    pub fn parties(&self) -> Vec<MarketPartyView> {
        self.inner.get_parties().iter().map(party_view).collect()
    }

    /// The first party whose role is `role`, exactly as the report spells
    /// it, or `null` where none is.
    #[napi]
    pub fn party_by_role(&self, role: String) -> Option<MarketPartyView> {
        self.inner.party_by_role(&role).map(party_view)
    }

    /// How many days after the trade date it settles, where both are
    /// stated: `T+2` answers `2`; else `null`.
    #[napi(getter)]
    pub fn settlement_days(&self) -> Option<i32> {
        self.inner.settlement_days()
    }
}

/// A price stated at an instant: one or two lanes under the quote's own
/// identifiers and its validity, its cancel ending the chain.
#[napi(js_name = "Quote")]
pub struct JsQuote {
    inner: QuoteData,
}

event_facts!(JsQuote wraps QuoteData as "Quote");
instrument_facts!(JsQuote);

#[napi]
impl JsQuote {
    /// The row every quote publishes, a non-null Struct named `quote`.
    #[napi]
    pub fn field() -> Result<JsField> {
        QuoteData::field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// The bid lane's price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn bidpx(&self) -> Option<String> {
        decimal(self.inner.get_bidpx())
    }

    /// The bid lane's quantity, as decimal text, or `null`.
    #[napi(getter)]
    pub fn bidqty(&self) -> Option<String> {
        decimal(self.inner.get_bidqty())
    }

    /// The bid lane's currency, or `null`.
    #[napi(getter)]
    pub fn bidcurrency(&self) -> Option<String> {
        text(self.inner.get_bidcurrency().map(Currency::as_str))
    }

    /// The bid lane's unit, or `null`.
    #[napi(getter)]
    pub fn bidunit(&self) -> Option<String> {
        text(self.inner.get_bidunit())
    }

    /// The ask lane's price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn askpx(&self) -> Option<String> {
        decimal(self.inner.get_askpx())
    }

    /// The ask lane's quantity, as decimal text, or `null`.
    #[napi(getter)]
    pub fn askqty(&self) -> Option<String> {
        decimal(self.inner.get_askqty())
    }

    /// The ask lane's currency, or `null`.
    #[napi(getter)]
    pub fn askcurrency(&self) -> Option<String> {
        text(self.inner.get_askcurrency().map(Currency::as_str))
    }

    /// The ask lane's unit, or `null`.
    #[napi(getter)]
    pub fn askunit(&self) -> Option<String> {
        text(self.inner.get_askunit())
    }

    /// The bid lane where it is quoted - its price and its size both above
    /// zero - as `{ px, qty }`, or `null`.
    #[napi(getter)]
    pub fn bid(&self) -> Option<MarketLaneView> {
        lane_view(self.inner.bid())
    }

    /// The ask lane where it is quoted, as `{ px, qty }`, or `null`.
    #[napi(getter)]
    pub fn ask(&self) -> Option<MarketLaneView> {
        lane_view(self.inner.ask())
    }

    /// The lane `side` takes - a spelling `Side.read` reads, `Buy` or `1`,
    /// `Sell` or `2` - the bid for a side that pays, the ask for one that is
    /// paid, `null` for a side that takes no lane. A spelling that is no
    /// side throws.
    #[napi]
    pub fn lane(&self, side: String) -> Result<Option<MarketLaneView>> {
        Ok(lane_view(self.inner.lane(&side_of(&side)?)))
    }

    /// Whether both lanes are quoted.
    #[napi(getter)]
    pub fn is_two_sided(&self) -> bool {
        self.inner.is_two_sided()
    }

    /// The middle of the two lanes, `(bidpx + askpx) / 2` as decimal text,
    /// or `null` on a one-sided quote.
    #[napi(getter)]
    pub fn mid(&self) -> Option<String> {
        decimal(self.inner.mid())
    }

    /// The distance between the two lanes, `askpx - bidpx` as decimal text,
    /// or `null` on a one-sided quote.
    #[napi(getter)]
    pub fn spread(&self) -> Option<String> {
        decimal(self.inner.spread())
    }
}

/// The ladder for one instrument at one instant, to a declared depth,
/// with what printed against it and what the ladders imply.
///
/// The chain is the instrument's - the symbol the book was read under -
/// so every book of one symbol stands in one chain; `currunix` is when the
/// ladder was read and `snapunix` the grid step it closed, where a grid was
/// declared. A book is a market event whose `px` is the mid, `qty` the
/// size resting on both ladders, `bidpx`/`bidqty`/`askpx`/`askqty` the
/// tops, `lastpx`/`lastqty`/`avgpx`/`cumqty` the prints since the chain
/// began, and `tradable` the makers' stated fact. A missing top is `null`
/// on every reading that divides by it; a locked or crossed book is
/// stated, never refused.
#[napi(js_name = "Book")]
pub struct JsBook {
    inner: BookData,
}

event_facts!(JsBook wraps BookData as "Book");
instrument_facts!(JsBook);

#[napi]
impl JsBook {
    /// The row every book of `depth` levels per side publishes, a non-null
    /// Struct named `book`; a depth of zero is no ladder and is refused.
    #[napi]
    pub fn field(depth: f64) -> Result<JsField> {
        BookData::field(book_depth(depth)?)
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// How many levels per side the book was declared to hold.
    #[napi(getter)]
    pub fn depth(&self) -> u32 {
        self.inner.get_depth().get()
    }

    /// How many statements have been applied to the book's chain since it
    /// began - every order, quote and print.
    #[napi(getter)]
    pub fn updates(&self) -> Result<f64> {
        exact_f64(self.inner.get_updates(), "updates")
    }

    /// The mid, as decimal text; `0` where there is none.
    #[napi(getter)]
    pub fn px(&self) -> String {
        self.inner.get_px().to_string()
    }

    /// The size resting on both ladders, as decimal text.
    #[napi(getter)]
    pub fn qty(&self) -> String {
        self.inner.get_qty().to_string()
    }

    /// The best bid's price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn bidpx(&self) -> Option<String> {
        decimal(self.inner.get_bidpx())
    }

    /// The size resting at the best bid, as decimal text, or `null`.
    #[napi(getter)]
    pub fn bidqty(&self) -> Option<String> {
        decimal(self.inner.get_bidqty())
    }

    /// The best ask's price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn askpx(&self) -> Option<String> {
        decimal(self.inner.get_askpx())
    }

    /// The size resting at the best ask, as decimal text, or `null`.
    #[napi(getter)]
    pub fn askqty(&self) -> Option<String> {
        decimal(self.inner.get_askqty())
    }

    /// The last print's price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn lastpx(&self) -> Option<String> {
        decimal(self.inner.get_lastpx())
    }

    /// The last print's size, as decimal text, or `null`.
    #[napi(getter)]
    pub fn lastqty(&self) -> Option<String> {
        decimal(self.inner.get_lastqty())
    }

    /// The average price of what printed since the chain began, or `null`.
    #[napi(getter)]
    pub fn avgpx(&self) -> Option<String> {
        decimal(self.inner.get_avgpx())
    }

    /// The volume printed since the chain began, or `null`.
    #[napi(getter)]
    pub fn cumqty(&self) -> Option<String> {
        decimal(self.inner.get_cumqty())
    }

    /// Whether the instrument can trade, the makers' stated fact, or
    /// `null` where they said nothing either way or disagreed.
    #[napi(getter)]
    pub fn tradable(&self) -> Option<bool> {
        self.inner.get_tradable()
    }

    /// The currency the ladder is priced in; `XXX` where none is stated.
    #[napi(getter)]
    pub fn currency(&self) -> String {
        self.inner.get_currency().as_str().to_owned()
    }

    /// The unit the sizes are counted in, empty where none is stated.
    #[napi(getter)]
    pub fn unit(&self) -> String {
        self.inner.get_unit().to_owned()
    }

    /// The bid ladder, best first: at most `depth` levels.
    #[napi(getter)]
    pub fn bids(&self) -> Result<Vec<MarketLevelView>> {
        levels_view(self.inner.get_bids())
    }

    /// The ask ladder, best first: at most `depth` levels.
    #[napi(getter)]
    pub fn asks(&self) -> Result<Vec<MarketLevelView>> {
        levels_view(self.inner.get_asks())
    }

    /// The top of the bid ladder, or `null`.
    #[napi(getter)]
    pub fn best_bid(&self) -> Result<Option<MarketLevelView>> {
        self.inner.best_bid().map(level_view).transpose()
    }

    /// The top of the ask ladder, or `null`.
    #[napi(getter)]
    pub fn best_ask(&self) -> Result<Option<MarketLevelView>> {
        self.inner.best_ask().map(level_view).transpose()
    }

    /// The level at `index` of the ladder `side` takes - a spelling
    /// `Side.read` reads - the top at zero; `null` past the ladder, or for
    /// a side that takes no lane. A spelling that is no side throws.
    #[napi]
    pub fn level(&self, side: String, index: f64) -> Result<Option<MarketLevelView>> {
        let index = exact_u32(index, "index")? as usize;
        self.inner
            .level(&side_of(&side)?, index)
            .map(level_view)
            .transpose()
    }

    /// Whether both ladders rest something.
    #[napi(getter)]
    pub fn is_two_sided(&self) -> bool {
        self.inner.is_two_sided()
    }

    /// Whether the two tops are at one price: a spread of zero.
    #[napi(getter)]
    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    /// Whether the best bid is above the best ask: a negative spread.
    #[napi(getter)]
    pub fn is_crossed(&self) -> bool {
        self.inner.is_crossed()
    }

    /// The middle of the tops, `(bidpx + askpx) / 2` as decimal text, or
    /// `null` where a top is missing.
    #[napi(getter)]
    pub fn mid(&self) -> Option<String> {
        decimal(self.inner.mid())
    }

    /// The distance between the tops, `askpx - bidpx` as decimal text -
    /// zero where they lock, negative where they cross - or `null` where a
    /// top is missing.
    #[napi(getter)]
    pub fn spread(&self) -> Option<String> {
        decimal(self.inner.spread())
    }

    /// The spread as a share of the mid, `spread * 10000 / mid` as decimal
    /// text, or `null` where there is no mid.
    #[napi(getter)]
    pub fn spread_bps(&self) -> Option<String> {
        decimal(self.inner.spread_bps())
    }

    /// The size-weighted mid, `(b * Qa + a * Qb) / (Qb + Qa)` as decimal
    /// text, or `null` where a top is missing.
    #[napi(getter)]
    pub fn microprice(&self) -> Option<String> {
        decimal(self.inner.microprice())
    }

    /// The imbalance at the tops, `(Qb - Qa) / (Qb + Qa)` in `[-1, 1]` as
    /// decimal text, a missing top counting as no size; `null` where both
    /// are missing.
    #[napi(getter)]
    pub fn imbalance(&self) -> Option<String> {
        decimal(self.inner.imbalance())
    }

    /// The same imbalance over the sizes summed across the first `levels`
    /// of each ladder, a shorter ladder contributing what it has; `null`
    /// where both sums are zero.
    #[napi]
    pub fn imbalance_to_depth(&self, levels: f64) -> Result<Option<String>> {
        let levels = exact_u32(levels, "levels")? as usize;
        Ok(decimal(self.inner.imbalance_to_depth(levels)))
    }

    /// The size resting on the bid ladder, summed, as decimal text.
    #[napi(getter)]
    pub fn bid_size(&self) -> String {
        self.inner.bid_size().to_string()
    }

    /// The size resting on the ask ladder, summed, as decimal text.
    #[napi(getter)]
    pub fn ask_size(&self) -> String {
        self.inner.ask_size().to_string()
    }

    /// The orders and quote lanes resting on the bid ladder, summed.
    #[napi(getter)]
    pub fn bid_count(&self) -> Result<f64> {
        exact_f64(self.inner.bid_count(), "bidCount")
    }

    /// The orders and quote lanes resting on the ask ladder, summed.
    #[napi(getter)]
    pub fn ask_count(&self) -> Result<f64> {
        exact_f64(self.inner.ask_count(), "askCount")
    }
}

/// A stream of one product, one at a time: what the product's codec door
/// answers, mirroring `FixMessages`.
///
/// Nothing is collected: the core iterator is the stream, and the
/// JavaScript iterable behind it is pulled one message at a time. A
/// message the door refuses throws where it is met and the stream goes on
/// past it; a failure in the iterable behind the stream throws once, in
/// place of the end. The loader supplies `Symbol.iterator` over `next`.
macro_rules! product_stream {
    ($(#[$doc:meta])* $stream:ident as $name:literal over $js:ident wraps $core:ty, $result:literal) => {
        $(#[$doc])*
        #[napi(js_name = $name)]
        pub struct $stream {
            inner: Box<dyn Iterator<Item = yggdryl::Result<$core>>>,
            /// Where the JavaScript source behind `inner` failed, when there
            /// is one.
            failed: Option<Failed>,
        }

        impl $stream {
            /// A stream over a core door fed by a JavaScript iterable.
            fn pulling<I>(inner: I, failed: Failed) -> Self
            where
                I: Iterator<Item = yggdryl::Result<$core>> + 'static,
            {
                Self {
                    inner: Box::new(inner),
                    failed: Some(failed),
                }
            }
        }

        #[napi]
        impl $stream {
            /// Advance the stream: the next product, or `null` at its end.
            ///
            /// A message the door refused throws here and the stream
            /// continues on the next call; a failure behind the stream
            /// throws once, in place of the end.
            #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
            #[napi(ts_return_type = $result)]
            pub fn next(&mut self) -> Result<Option<$js>> {
                match self.inner.next() {
                    Some(held) => held.map($js::from_core).map(Some).map_err(napi_error),
                    None => match self.failed.as_ref().and_then(Failed::take) {
                        Some(error) => Err(error),
                        None => Ok(None),
                    },
                }
            }
        }
    };
}

product_stream!(
    /// A stream of orders: what `FixCodec.orders` answers.
    JsOrders as "Orders" over JsOrder wraps OrderData, "IteratorResult<Order>"
);
product_stream!(
    /// A stream of executions: what `FixCodec.executions` answers.
    JsExecutions as "Executions" over JsExecution wraps ExecutionData, "IteratorResult<Execution>"
);
product_stream!(
    /// A stream of trades: what `FixCodec.trades` answers.
    JsTrades as "Trades" over JsTrade wraps TradeData, "IteratorResult<Trade>"
);
product_stream!(
    /// A stream of quotes: what `FixCodec.quotes` answers.
    JsQuotes as "Quotes" over JsQuote wraps QuoteData, "IteratorResult<Quote>"
);
product_stream!(
    /// A stream of books: what `FixCodec.books` answers.
    JsBooks as "Books" over JsBook wraps BookData, "IteratorResult<Book>"
);

/// One statement as the arm it holds: the product class the core arm maps
/// to, so a mixed stream yields the four classes and never a fifth.
type JsStatement = Either4<JsOrder, JsQuote, JsExecution, JsTrade>;

fn statement_view(statement: Statement) -> JsStatement {
    match statement {
        Statement::Order(held) => Either4::A(JsOrder::from_core(held)),
        Statement::Quote(held) => Either4::B(JsQuote::from_core(held)),
        Statement::Execution(held) => Either4::C(JsExecution::from_core(held)),
        Statement::Trade(held) => Either4::D(JsTrade::from_core(held)),
    }
}

/// A stream of every statement a stream of messages makes, in instant
/// order: what `FixCodec.statements` answers, each item the product class
/// its arm is - an `Order`, a `Quote` or an `Execution`; a trade is the
/// print an execution already is, so the door yields none.
///
/// Nothing is collected on the binding path: the core iterator is the
/// stream, and the JavaScript iterable behind it is pulled one message at a
/// time. A message the door refuses throws where it is met and the stream
/// goes on past it; a failure in the iterable behind the stream throws
/// once, in place of the end. The loader supplies `Symbol.iterator` over
/// `next`.
#[napi(js_name = "Statements")]
pub struct JsStatements {
    inner: Box<dyn Iterator<Item = yggdryl::Result<Statement>>>,
    /// Where the JavaScript source behind `inner` failed, when there is
    /// one.
    failed: Option<Failed>,
}

impl JsStatements {
    /// A stream over the core door fed by a JavaScript iterable.
    fn pulling<I>(inner: I, failed: Failed) -> Self
    where
        I: Iterator<Item = yggdryl::Result<Statement>> + 'static,
    {
        Self {
            inner: Box::new(inner),
            failed: Some(failed),
        }
    }
}

#[napi]
impl JsStatements {
    /// Advance the stream: the next statement as the class its arm is, or
    /// `null` at its end.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<Order | Quote | Execution>")]
    pub fn next(&mut self) -> Result<Option<JsStatement>> {
        match self.inner.next() {
            Some(held) => held.map(statement_view).map(Some).map_err(napi_error),
            None => match self.failed.as_ref().and_then(Failed::take) {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}

/// The key a book is read under: the text that names an instrument across
/// the venues that trade it, or `GLOBAL` for no instrument.
///
/// `Symbol.of` reads one key off a product's instrument codes, the
/// strongest first: the ISIN, else the ticker, else the CUSIP, the SEDOL or
/// the Bloomberg identifier; a product naming none keys the global symbol,
/// which is also the one symbol a global book - every statement of a
/// stream in one ladder - is read under. A book's `crosscode` is the symbol
/// it was read under. The loader publishes this as `market.Symbol`, with
/// `market.Symbol.GLOBAL` beside `of`.
#[napi(js_name = "MarketSymbol")]
pub struct JsMarketSymbol {
    inner: CoreSymbol,
}

impl JsMarketSymbol {
    const fn from_core(inner: CoreSymbol) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsMarketSymbol {
    /// A symbol spelled by the caller, trimmed; blank text is the global
    /// symbol.
    #[napi(constructor)]
    #[must_use]
    pub fn new(text: String) -> Self {
        Self::from_core(CoreSymbol::new(&text))
    }

    /// The symbol a product names: its ISIN, else the ticker it is known
    /// by, else its CUSIP, else its SEDOL, else its Bloomberg identifier,
    /// else the global symbol. A value of another class throws naming the
    /// classes.
    #[napi(factory)]
    pub fn of(
        product: Either5<
            ClassInstance<'_, JsOrder>,
            ClassInstance<'_, JsQuote>,
            ClassInstance<'_, JsExecution>,
            ClassInstance<'_, JsTrade>,
            ClassInstance<'_, JsBook>,
        >,
    ) -> Self {
        Self::from_core(match product {
            Either5::A(held) => CoreSymbol::of(&held.inner),
            Either5::B(held) => CoreSymbol::of(&held.inner),
            Either5::C(held) => CoreSymbol::of(&held.inner),
            Either5::D(held) => CoreSymbol::of(&held.inner),
            Either5::E(held) => CoreSymbol::of(&held.inner),
        })
    }

    /// The text the symbol is.
    #[napi(getter)]
    pub fn text(&self) -> String {
        self.inner.as_str().to_owned()
    }

    /// Whether this is the symbol of no instrument.
    #[napi(getter)]
    pub fn is_global(&self) -> bool {
        self.inner.is_global()
    }

    /// Whether two symbols are one text.
    #[napi]
    pub fn equals(&self, other: &JsMarketSymbol) -> bool {
        self.inner == other.inner
    }

    /// The text the symbol is.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// One statement pulled off a JavaScript iterable: an instance of one of
/// the four product classes that state something to a ladder, converted to
/// the core statement by its class; the loader refuses anything else where
/// it is met.
type PulledStatement = Either4<
    ClassInstance<'static, JsOrder>,
    ClassInstance<'static, JsQuote>,
    ClassInstance<'static, JsExecution>,
    ClassInstance<'static, JsTrade>,
>;

fn statement_of(pulled: PulledStatement) -> Statement {
    match pulled {
        Either4::A(held) => Statement::from(held.inner.clone()),
        Either4::B(held) => Statement::from(held.inner.clone()),
        Either4::C(held) => Statement::from(held.inner.clone()),
        Either4::D(held) => Statement::from(held.inner.clone()),
    }
}

/// One book per symbol per instant, read out of a stream of statements -
/// orders, quotes, executions and trades, any product a door read or a row
/// stated - to a declared depth.
///
/// The orders and the quotes rest on each symbol's ladders under the
/// identity their chain shares, a later statement of one replacing the
/// earlier and a dead or expired one leaving; the executions and the
/// trades print against them - the last price and size, the volume and its
/// average price - never resting; and the book of every symbol an instant
/// touched is read once the stream moves past that instant, in symbol
/// order. A stream carrying both the execution and the trade of one fill
/// counts it twice, so a caller feeds one of the two. The statements are
/// taken as the caller's word that they arrive in instant order, which a
/// door's stream does; one before the open instant throws naming
/// `currunix` and moves nothing. With a step, one book per symbol per grid
/// step it was touched in, the step's closing state, stamped with the
/// step; with a symbol, every statement keyed under it, `Symbol.GLOBAL`
/// being the global book of the whole stream. Held state is bounded by the
/// live makers and the open instant. The loader supplies `Symbol.iterator`
/// over `next`, and builds one through `new market.BookIterator`.
#[napi(js_name = "BookIterator")]
pub struct JsBookIterator {
    inner: Box<dyn Iterator<Item = yggdryl::Result<BookData>>>,
    /// Where the JavaScript source behind `inner` failed, when there is
    /// one.
    failed: Option<Failed>,
    depth: NonZeroU32,
    snapshot_ns: i64,
    symbol: Option<CoreSymbol>,
}

#[napi]
impl JsBookIterator {
    /// Advance the iterator: the next book, or `null` at its end.
    ///
    /// A statement the ladder refuses throws here and the iterator
    /// continues on the next call; a failure behind it throws once, in
    /// place of the end.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<Book>")]
    pub fn next(&mut self) -> Result<Option<JsBook>> {
        match self.inner.next() {
            Some(held) => held.map(JsBook::from_core).map(Some).map_err(napi_error),
            None => match self.failed.as_ref().and_then(Failed::take) {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }

    /// How many levels a side every book is read to.
    #[napi(getter)]
    pub fn depth(&self) -> u32 {
        self.depth.get()
    }

    /// The grid step in nanoseconds, `0n` for one book per instant.
    #[napi(getter)]
    pub fn snapshot_ns(&self) -> BigInt {
        instant(self.snapshot_ns)
    }

    /// The one symbol every statement is keyed under, or `null` where each
    /// keys the symbol it names.
    #[napi(getter)]
    pub fn symbol(&self) -> Option<JsMarketSymbol> {
        self.symbol.clone().map(JsMarketSymbol::from_core)
    }
}

/// The book iterator over a bound pull function answering statements one
/// at a time, `depth` levels a side, a grid of `snapshotNs` nanoseconds
/// where positive, every statement keyed under `symbol` where one is
/// given. A depth of zero is refused naming `depth` and a negative step
/// naming `snapshot_ns`, before a statement is pulled. The loader turns
/// the iterable into the pull function this takes.
#[napi(js_name = "_bookIteratorNative", skip_typescript)]
pub fn book_iterator_native(
    env: Env,
    pull: Function<'_, (), Option<PulledStatement>>,
    depth: f64,
    snapshot_ns: Either<BigInt, f64>,
    symbol: Option<&JsMarketSymbol>,
) -> Result<JsBookIterator> {
    let depth = book_depth(depth)?;
    let snapshot_ns = nanoseconds(snapshot_ns, "snapshotNs")?;
    if snapshot_ns < 0 {
        return Err(napi_error(
            "snapshot_ns must be a grid step in nanoseconds, or zero for one book per instant",
        ));
    }
    let symbol = symbol.map(|held| held.inner.clone());
    let pulled: Pulled<PulledStatement> = Pulled::new(env, pull)?;
    let failed = pulled.failed.clone();
    let statements = pulled.map(|held| Ok(statement_of(held)));
    let mut books = CoreBookIterator::new(statements, depth, true).with_snapshot_ns(snapshot_ns);
    if let Some(symbol) = symbol.clone() {
        books = books.with_symbol(symbol);
    }
    Ok(JsBookIterator {
        inner: Box::new(books),
        failed: Some(failed),
        depth,
        snapshot_ns,
        symbol,
    })
}

/// The messages a bound pull function answers, one at a time, as the core
/// door takes them, with the holder the pull's own failure lands in.
type PulledMessages = Pulled<ClassInstance<'static, JsFixMsg>>;

fn pulled_messages(
    env: Env,
    pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
) -> Result<(impl Iterator<Item = CoreFixMsg> + 'static, Failed)> {
    let pulled: PulledMessages = Pulled::new(env, pull)?;
    let failed = pulled.failed.clone();
    Ok((pulled.map(|message| message.as_core().clone()), failed))
}

#[napi]
impl JsFixCodec {
    /// The orders a stream of messages states, chained: one statement per
    /// message that states an order - a placement, a replace, a cancel, a
    /// report against it, a cancel reject - each naming the message it was
    /// read from as its source, the statements of one order in one chain
    /// under the identifier the venue gave it.
    ///
    /// The lifecycle first: the messages are chained before a product is
    /// read, so a statement carries what its message's chain folded
    /// forward; a message logged at two hops states its order twice and
    /// the two statements fold into one, naming both messages among its
    /// sources; the walk then chains the statements of one order exactly as
    /// `lifecycle` chains messages. The loader turns the iterable into the
    /// pull function this takes.
    #[napi(js_name = "_ordersNative", skip_typescript)]
    pub fn orders_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsOrders> {
        let (messages, failed) = pulled_messages(env, pull)?;
        Ok(JsOrders::pulling(self.inner.orders(messages), failed))
    }

    /// The fills a stream of messages reports, chained: one per execution
    /// report stating a quantity that traded, each in the chain of the
    /// order it fills. Read as `orders` reads: the lifecycle first, twins
    /// folded, the walk after.
    #[napi(js_name = "_executionsNative", skip_typescript)]
    pub fn executions_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsExecutions> {
        let (messages, failed) = pulled_messages(env, pull)?;
        Ok(JsExecutions::pulling(
            self.inner.executions(messages),
            failed,
        ))
    }

    /// The trades a stream of messages reports, chained: one per execution
    /// report or trade capture report stating a quantity that traded, the
    /// two sides' reports of one match in one chain under the identifier
    /// the venue matched them by. Read as `orders` reads.
    #[napi(js_name = "_tradesNative", skip_typescript)]
    pub fn trades_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsTrades> {
        let (messages, failed) = pulled_messages(env, pull)?;
        Ok(JsTrades::pulling(self.inner.trades(messages), failed))
    }

    /// The quotes a stream of messages states, chained: one statement per
    /// quote, quote status report or quote cancel, the statements of one
    /// quote in one chain under its identifier, a cancel ending it. Read
    /// as `orders` reads.
    #[napi(js_name = "_quotesNative", skip_typescript)]
    pub fn quotes_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsQuotes> {
        let (messages, failed) = pulled_messages(env, pull)?;
        Ok(JsQuotes::pulling(self.inner.quotes(messages), failed))
    }

    /// Every statement a stream of messages makes, in instant order: the
    /// orders and the quotes, walked, and each fill in its order's chain,
    /// each yielded as the class its arm is. Trades are not among them: a
    /// fill is the print, and the trade of the same report would print it
    /// again. This is the stream `books` reads. The loader turns the
    /// iterable into the pull function this takes.
    #[napi(js_name = "_statementsNative", skip_typescript)]
    pub fn statements_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsStatements> {
        let (messages, failed) = pulled_messages(env, pull)?;
        Ok(JsStatements::pulling(
            self.inner.statements(messages),
            failed,
        ))
    }

    /// The books a stream of messages makes: one per symbol per instant the
    /// symbol was touched at, or per grid step of `snapshotNs` nanoseconds
    /// where the step is positive, to `depth` levels per side, chained per
    /// symbol.
    ///
    /// A `BookIterator` over `statements`: the orders and the quotes rest,
    /// the fills print, and the book of every symbol an instant touched is
    /// read once the stream moves past it; with a grid, the book of a step
    /// is its closing state, dated at the last instant that moved the
    /// symbol and stamped with the step. A depth of zero and a negative step
    /// are refused before a message is pulled, with the core's sentence
    /// naming `depth` or `snapshot_ns`.
    #[napi(js_name = "_booksNative", skip_typescript)]
    pub fn books_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
        depth: f64,
        snapshot_ns: Either<BigInt, f64>,
    ) -> Result<JsBooks> {
        let depth = exact_u32(depth, "depth")?;
        let snapshot_ns = nanoseconds(snapshot_ns, "snapshotNs")?;
        let (messages, failed) = pulled_messages(env, pull)?;
        let books = self
            .inner
            .books(messages, depth, snapshot_ns)
            .map_err(napi_error)?;
        Ok(JsBooks::pulling(books, failed))
    }

    /// `orders` over a stream of batches of message rows: the rows read as
    /// messages by `messages`, the orders read out of them and written as
    /// batches of order rows under `Order.field()`. The source is consumed.
    #[napi]
    pub fn orders_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let reader = self
            .inner
            .orders_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, OrderData::NAME))
    }

    /// `executions` over a stream of batches of message rows, written as
    /// batches of execution rows under `Execution.field()`. The source is
    /// consumed.
    #[napi]
    pub fn executions_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let reader = self
            .inner
            .executions_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, ExecutionData::NAME))
    }

    /// `trades` over a stream of batches of message rows, written as
    /// batches of trade rows under `Trade.field()`. The source is consumed.
    #[napi]
    pub fn trades_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let reader = self
            .inner
            .trades_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, TradeData::NAME))
    }

    /// `quotes` over a stream of batches of message rows, written as
    /// batches of quote rows under `Quote.field()`. The source is consumed.
    #[napi]
    pub fn quotes_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let reader = self
            .inner
            .quotes_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, QuoteData::NAME))
    }

    /// `books` over a stream of batches of message rows, written as batches
    /// of book rows under `Book.field(depth)`. What `books` refuses is
    /// refused here, before a row is read. The source is consumed.
    #[napi]
    pub fn books_arrow_reader(
        &self,
        source: &mut JsBatchReader,
        depth: f64,
        snapshot_ns: Either<BigInt, f64>,
    ) -> Result<JsBatchReader> {
        let depth = exact_u32(depth, "depth")?;
        let snapshot_ns = nanoseconds(snapshot_ns, "snapshotNs")?;
        let reader = self
            .inner
            .books_arrow_reader(source.take()?, depth, snapshot_ns)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, BookData::NAME))
    }
}

#[napi]
impl JsFixMsg {
    /// The new order single an order states: `35=D`, exactly.
    ///
    /// The order's `ClOrdID(11)` - the name it goes by, else its chain - the
    /// venue's `OrderID(37)` and the `OrigClOrdID(41)` where it goes by
    /// them, the instrument and its market, the order type its prices
    /// imply, the quantity, the price, the stop price, how long it stands,
    /// when it expires, and the instant as `TransactTime(60)` and
    /// `SendingTime(52)`. The message names the order as its one source.
    /// An order going by no client identifier and naming no chain is
    /// refused.
    #[napi(factory)]
    pub fn from_order(codec: &JsFixCodec, order: &JsOrder) -> Result<Self> {
        CoreFixMsg::from_order(&codec.inner, &order.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The execution report an execution states: `35=8`, `150=F`, exactly.
    ///
    /// The execution's own `ExecID(17)`, the order it fills where it goes by
    /// its names, the venue's match where it names one, the order's status
    /// as far as a fill can say it, the instrument, its market as
    /// `LastMkt(30)`, the side, the price and quantity that traded as
    /// `LastPx(31)` and `LastQty(32)`, the currency and unit, and the
    /// instant. An execution going by no identifier of its own is refused.
    #[napi(factory)]
    pub fn from_execution(codec: &JsFixCodec, execution: &JsExecution) -> Result<Self> {
        CoreFixMsg::from_execution(&codec.inner, &execution.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Refused: no one message states a quote, and the crate does not
    /// guess. Throws the core's sentence.
    #[napi(factory)]
    pub fn from_quote(codec: &JsFixCodec, quote: &JsQuote) -> Result<Self> {
        CoreFixMsg::from_quote(&codec.inner, &quote.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Refused: no one message states a trade, and the crate does not
    /// guess. Throws the core's sentence.
    #[napi(factory)]
    pub fn from_trade(codec: &JsFixCodec, trade: &JsTrade) -> Result<Self> {
        CoreFixMsg::from_trade(&codec.inner, &trade.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Refused: no one message states a book, and the crate does not
    /// guess. Throws the core's sentence.
    #[napi(factory)]
    pub fn from_book(codec: &JsFixCodec, book: &JsBook) -> Result<Self> {
        CoreFixMsg::from_book(&codec.inner, &book.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }
}
