//! Native Node.js view of the graph vocabulary: the typed leaves - an order,
//! a quote or an execution, undated ([`operation::JsOrder`] ..) or dated
//! ([`operation::JsOrderEvent`] ..), a composite trade
//! ([`trade::JsTradeEvent`]), a book, its sides and its snapshot control
//! ([`book::JsBookEvent`], [`book::JsSnapshotEvent`])
//! - and [`market_data::JsMarketData`], the one value over every leaf.
//!
//! Nothing here resolves, folds, merges or validates a fact: a named fact is
//! resolved by the column enums' own `of_name`, checked by its column's own
//! field and stated by its column's own `record`; every other constructor
//! redirects to the core door named beside it, and a getter to the trait
//! accessor the fact answers. A value enters as a [`JsScalar`], already
//! crossed once through the crate's JavaScript-to-`Scalar` boundary, never
//! reparsed here.
//!
//! Facts cross as the plain values the rest of the addon uses: a UUID as its
//! hyphenated text, an instant and a hash as a `bigint`, `seqnum` as a
//! `number`, a decimal as its text, a code as the text it is, a set of
//! identifiers as `Identifiers`, the enum facts - `state`, `side`,
//! `marketdatakind` - as the member's stored name.
//!
//! The fact getters and the verbs every leaf shares are written once, as the
//! macros below, each emitting its own `#[napi] impl` block for the class it
//! is applied to; NAPI merges every block of one class into its prototype.

use napi::bindgen_prelude::{
    BigInt, ClassInstance, Either, Either11, FromNapiValue, Null, Result, Unknown,
};
use napi_derive::napi;
use yggdryl::graph::{
    ElementColumn, Event, EventColumn, MarketColumn, MarketData as CoreMarketData, Operation,
    OperationColumn, OperationEvent, OperationKind as CoreOperationKind,
};
use yggdryl::{Decimal, Identifiers, Scalar, graph};

use crate::fix::JsFixMsg;
use crate::text::codec::JsScalar;
use crate::{exact_i64, napi_error};

/// The six facts [`yggdryl::graph::Element`] answers.
macro_rules! element_getters {
    ($class:ident) => {
        #[napi]
        impl $class {
            /// The element's own identity, as its hyphenated text.
            #[napi(getter)]
            pub fn curruuid(&self) -> String {
                ::yggdryl::graph::Element::get_curruuid(&self.inner).to_string()
            }

            /// The identity every statement of one element shares: derived
            /// from the cross code, the element's own where it names none.
            #[napi(getter)]
            pub fn crossuuid(&self) -> String {
                ::yggdryl::graph::Element::get_crossuuid(&self.inner).to_string()
            }

            /// The cross code: the identifier every statement of one element
            /// shares, stored as `{kind}:{side}:{base}` - the
            /// `MarketDataKind` code, the `Side` code of a sided kind (`0`
            /// for any other) and the identifier itself, so a buy order
            /// `ORD-1` is `10:1:ORD-1` and a book `3:0:AAPL` - empty where
            /// it names none.
            #[napi(getter)]
            pub fn crosscode(&self) -> String {
                ::yggdryl::graph::Element::get_crosscode(&self.inner).to_owned()
            }

            /// The XXH3-64 code the element's content digests to.
            #[napi(getter)]
            pub fn currhashcode(&self) -> ::napi::bindgen_prelude::BigInt {
                ::napi::bindgen_prelude::BigInt::from(::yggdryl::graph::Element::get_currhashcode(
                    &self.inner,
                ))
            }

            /// The XXH3-64 of the cross code, `0n` where it names none.
            #[napi(getter)]
            pub fn crosshashcode(&self) -> ::napi::bindgen_prelude::BigInt {
                ::napi::bindgen_prelude::BigInt::from(::yggdryl::graph::Element::get_crosshashcode(
                    &self.inner,
                ))
            }

            /// The sorted identities of the elements this one was read from:
            /// provenance, never its chain. Empty for one built directly.
            #[napi(getter)]
            pub fn srcuuids(&self) -> Vec<String> {
                ::yggdryl::graph::Element::get_srcuuids(&self.inner)
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            }
        }
    };
}

/// The facts [`yggdryl::graph::Event`] adds: the clocks, the state, its
/// place among the events of its instant, and whether the observation
/// reports an execution.
macro_rules! event_getters {
    ($class:ident) => {
        #[napi]
        impl $class {
            /// When this happened: nanoseconds since the Unix epoch, UTC.
            #[napi(getter)]
            pub fn currunix(&self) -> ::napi::bindgen_prelude::BigInt {
                ::napi::bindgen_prelude::BigInt::from(::yggdryl::graph::Event::get_currunix(
                    &self.inner,
                ))
            }

            /// The lifecycle state reached, as the `state` code it is.
            #[napi(getter)]
            pub fn state(&self) -> String {
                ::yggdryl::graph::Event::get_state(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// Its place among the events of its instant: zero for the
            /// first of each run its stream hands over at that instant, with
            /// no other instant between, one more for each next.
            #[napi(getter)]
            pub fn seqnum(&self) -> ::napi::bindgen_prelude::Result<f64> {
                $crate::exact_f64(::yggdryl::graph::Event::get_seqnum(&self.inner), "seqnum")
            }

            /// When this was created, where known.
            #[napi(getter)]
            pub fn creaunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_creaunix(&self.inner)
                    .map(::napi::bindgen_prelude::BigInt::from)
            }

            /// When this was recorded, where stated.
            #[napi(getter)]
            pub fn recdunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_recdunix(&self.inner)
                    .map(::napi::bindgen_prelude::BigInt::from)
            }

            /// When this expires, where it has an expiry.
            #[napi(getter)]
            pub fn exprunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_exprunix(&self.inner)
                    .map(::napi::bindgen_prelude::BigInt::from)
            }

            /// When the element this one follows happened, where it follows
            /// one.
            #[napi(getter)]
            pub fn prevunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_prevunix(&self.inner)
                    .map(::napi::bindgen_prelude::BigInt::from)
            }

            /// The identity of the element this one follows, or `null`.
            #[napi(getter)]
            pub fn prevuuid(&self) -> Option<String> {
                ::yggdryl::graph::Event::get_prevuuid(&self.inner).map(|uuid| uuid.to_string())
            }

            /// The grid step a walk read this as the snapshot of, where one
            /// did.
            #[napi(getter)]
            pub fn snapunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_snapunix(&self.inner)
                    .map(::napi::bindgen_prelude::BigInt::from)
            }

            /// Whether this observation itself reports an execution.
            #[napi(getter)]
            pub fn is_execution(&self) -> bool {
                ::yggdryl::graph::Event::is_execution(&self.inner)
            }
        }
    };
}

/// The twenty-seven facts [`yggdryl::graph::Market`] answers.
macro_rules! market_getters {
    ($class:ident) => {
        #[napi]
        impl $class {
            /// The price stated, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn price(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_price(&self.inner))
            }

            /// The currency, as the `ccy` code it is; `XXX` where none.
            #[napi(getter)]
            pub fn currency(&self) -> String {
                ::yggdryl::graph::Market::get_currency(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// The quantity stated, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn quantity(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_quantity(&self.inner))
            }

            /// The stop price the order triggers at, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn stoppx(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_stoppx(&self.inner))
            }

            /// The part of the quantity shown to the market - an iceberg's peak, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn displayqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_displayqty(&self.inner))
            }

            /// The part of the quantity kept from the market - an iceberg's reserve, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn hiddenqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_hiddenqty(&self.inner))
            }

            /// How much was canceled, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn cxlqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_cxlqty(&self.inner))
            }

            /// The unit the quantity is counted in, as spelled; empty where
            /// none.
            #[napi(getter)]
            pub fn unit(&self) -> String {
                ::yggdryl::graph::Market::get_unit(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// The type of its kind this is, as the `marketdatatype` member's
            /// stored name; `UKNW` where none, never `null`.
            #[napi(getter)]
            pub fn marketdatatype(&self) -> String {
                ::yggdryl::graph::Market::get_marketdatatype(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// The side, as the `side` member's four-letter code; `UKNW` where
            /// none, never `null`.
            #[napi(getter)]
            pub fn side(&self) -> String {
                ::yggdryl::graph::Market::get_side(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// The instrument's security identifiers, each a code under a key
            /// - `isin`, `derived:cusip`, `ullink:isin` - a map keyed
            /// `src:type`, the type alone for the base source, in key order.
            #[napi(getter, ts_return_type = "Identifiers")]
            pub fn securityids(&self) -> $crate::identifier::JsIdentifiers {
                $crate::identifier::JsIdentifiers::from_core(
                    ::yggdryl::graph::Market::get_securityids(&self.inner),
                )
            }

            /// The instrument's ISIN, borrowed from `securityids`; `null`
            /// where it states none.
            #[napi(getter)]
            pub fn isincode(&self) -> Option<String> {
                ::yggdryl::graph::Market::get_isincode(&self.inner).map(ToOwned::to_owned)
            }

            /// The instrument's classification; `null` where none.
            #[napi(getter)]
            pub fn cficode(&self) -> Option<String> {
                ::yggdryl::graph::Market::get_cficode(&self.inner)
                    .map(|held| held.as_str().to_owned())
            }

            /// The market, as an ISO 10383 MIC; `null` where none.
            #[napi(getter)]
            pub fn miccode(&self) -> Option<String> {
                ::yggdryl::graph::Market::get_miccode(&self.inner)
                    .map(|held| held.as_str().to_owned())
            }

            /// When this last executed: the latest execution instant its
            /// lifecycle reached, nanoseconds since the Unix epoch, UTC,
            /// where known - a market fact, never an event's.
            #[napi(getter)]
            pub fn execunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Market::get_execunix(&self.inner)
                    .map(::napi::bindgen_prelude::BigInt::from)
            }

            /// The price last traded at; `null` where none.
            #[napi(getter)]
            pub fn lastpx(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_lastpx(&self.inner))
            }

            /// The quantity last traded; `null` where none.
            #[napi(getter)]
            pub fn lastqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_lastqty(&self.inner))
            }

            /// The price averaged; `null` where none.
            #[napi(getter)]
            pub fn avgpx(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_avgpx(&self.inner))
            }

            /// How much is done; `null` where none.
            #[napi(getter)]
            pub fn cumqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_cumqty(&self.inner))
            }

            /// How much is still open; `null` where none.
            #[napi(getter)]
            pub fn leavesqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_leavesqty(&self.inner))
            }

            /// The price the step before this one settled on; `null` where
            /// none.
            #[napi(getter)]
            pub fn prevpx(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_prevpx(&self.inner))
            }

            /// The quantity the step before this one settled on; `null`
            /// where none.
            #[napi(getter)]
            pub fn prevqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_prevqty(&self.inner))
            }

            /// The spot part of an FX price; `null` where none.
            #[napi(getter)]
            pub fn spotrate(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_spotrate(&self.inner))
            }

            /// The forward points of an FX price; `null` where none.
            #[napi(getter)]
            pub fn forwardpoints(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_forwardpoints(
                    &self.inner,
                ))
            }

            /// The best bid price stated, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn bidpx(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_bidpx(&self.inner))
            }

            /// The quantity at the best bid, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn bidqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_bidqty(&self.inner))
            }

            /// The currency the bid is stated in, as the `ccy` code it is; `null`
            /// where none.
            #[napi(getter)]
            pub fn bidccy(&self) -> Option<String> {
                ::yggdryl::graph::Market::get_bidccy(&self.inner)
                    .map(|held| held.as_str().to_owned())
            }

            /// The best ask price stated, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn askpx(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_askpx(&self.inner))
            }

            /// The quantity at the best ask, as decimal text; `null` where none.
            #[napi(getter)]
            pub fn askqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Market::get_askqty(&self.inner))
            }

            /// The currency the ask is stated in, as the `ccy` code it is; `null`
            /// where none.
            #[napi(getter)]
            pub fn askccy(&self) -> Option<String> {
                ::yggdryl::graph::Market::get_askccy(&self.inner)
                    .map(|held| held.as_str().to_owned())
            }

            /// The rates an amount in `currency` is divided by to state it in
            /// each target currency, keyed by the target's `ccy` code, each
            /// rate as decimal text; empty where none.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn fxrates(&self) -> ::std::collections::BTreeMap<String, String> {
                ::yggdryl::graph::Market::get_fxrates(&self.inner)
                    .iter()
                    .map(|(target, rate)| (target.as_str().to_owned(), rate.to_string()))
                    .collect()
            }

            /// The ticker a person knows the instrument by; `null` where
            /// none.
            #[napi(getter)]
            pub fn ticker(&self) -> Option<String> {
                ::yggdryl::graph::Market::get_ticker(&self.inner).map(ToOwned::to_owned)
            }

            /// Free-form facts beside the typed ones, in key order; empty
            /// where none.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn metadata(&self) -> ::std::collections::BTreeMap<String, String> {
                ::yggdryl::graph::Market::get_metadata(&self.inner)
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect()
            }
        }
    };
}

/// The three facts [`yggdryl::graph::Operation`] adds.
macro_rules! operation_getters {
    ($class:ident) => {
        #[napi]
        impl $class {
            /// How long this stands, as the `timeinforce` member's stored
            /// name; `null` where unstated.
            #[napi(getter)]
            pub fn timeinforce(&self) -> Option<String> {
                ::yggdryl::graph::Operation::get_timeinforce(&self.inner)
                    .map(|held| held.as_str().to_owned())
            }

            /// The quantity the order asked for, as decimal text; `null` where
            /// unstated.
            #[napi(getter)]
            pub fn ordqty(&self) -> Option<String> {
                $crate::graph::decimal_text(::yggdryl::graph::Operation::get_ordqty(&self.inner))
            }

            /// Whether the instrument trades, or `null` where the market said
            /// nothing either way - which is not `false`.
            #[napi(getter)]
            pub fn tradable(&self) -> Option<bool> {
                ::yggdryl::graph::Operation::get_tradable(&self.inner)
            }

            /// The names the operation goes by - `clordid`, `orderid` - with
            /// the parents a chain gave them (`origclordid`,
            /// `parentorderid`, `origorderid`); a map keyed `src:type`, the
            /// type alone for the base source, in key order.
            #[napi(getter, ts_return_type = "Identifiers")]
            pub fn identifiers(&self) -> $crate::identifier::JsIdentifiers {
                $crate::identifier::JsIdentifiers::from_core(
                    ::yggdryl::graph::Operation::get_identifiers(&self.inner),
                )
            }

            /// The parties the operation names, each typed by its role -
            /// `executingtrader`, `clientid` - from its source, a map keyed
            /// `src:type`.
            #[napi(getter, ts_return_type = "Identifiers")]
            pub fn partyids(&self) -> $crate::identifier::JsIdentifiers {
                $crate::identifier::JsIdentifiers::from_core(
                    ::yggdryl::graph::Operation::get_partyids(&self.inner),
                )
            }
        }
    };
}

/// The `marketdatakind` of a leaf class, the category its `$kind` stands
/// under, as the member's stored name: `ORDR`, `QUOT`, `EXEC`, `TRAD` or
/// `BOOK`.
macro_rules! marketdatakind_getter {
    ($class:ident, $kind:ident) => {
        #[napi]
        impl $class {
            /// The market data category this leaf stands under, as the
            /// `marketdatakind` member's stored name.
            #[napi(getter)]
            pub fn marketdatakind(&self) -> &'static str {
                ::yggdryl::graph::MarketKind::$kind
                    .marketdatakind()
                    .as_str()
            }
        }
    };
}

/// The verbs every leaf and `MarketData` share: following, merging, the
/// order, equality, the hash, a clone and `toJSON`/`fromJSON`, which carry
/// the value's one-row `MarketData.arrowReader` IPC stream as base64 text
/// and rebuild it through `MarketData.fromArrowReader` - exact, because the
/// row states every identity.
macro_rules! common_verbs {
    ($class:ident) => {
        #[napi]
        impl $class {
            /// This value stated as the one after `previous`, or `null` where
            /// it cannot follow it or following changes nothing. It takes
            /// every `metadata` key of its chain it lacks and, where it names
            /// identifiers, every `identifiers` type but `mdentryrefid` and every
            /// party id, its own values standing, and the parents each
            /// identifier it states takes from its chain (`orderid` A then B
            /// is `parentorderid` A).
            #[napi]
            pub fn with_previous(&self, previous: &$class) -> Option<$class> {
                ::yggdryl::graph::Element::with_previous(self.inner.clone(), &previous.inner)
                    .map(Self::from_core)
            }

            /// This value with another statement of `other` folded in, or
            /// `null` for another element or a fold that changes nothing.
            #[napi]
            pub fn merge_with(&self, other: &$class) -> Option<$class> {
                ::yggdryl::graph::Element::merge_with(self.inner.clone(), &other.inner)
                    .map(Self::from_core)
            }

            /// Whether this value comes after `other` in its order.
            #[napi]
            pub fn is_after(&self, other: &$class) -> bool {
                ::yggdryl::graph::Element::is_after(&self.inner, &other.inner)
            }

            /// Whether this value comes before `other` in its order.
            #[napi]
            pub fn is_before(&self, other: &$class) -> bool {
                ::yggdryl::graph::Element::is_before(&self.inner, &other.inner)
            }

            /// Whether this value states the same facts as `other`.
            #[napi]
            pub fn equals(&self, other: &$class) -> bool {
                self.inner == other.inner
            }

            /// The code the content digests to, which equal values share.
            #[napi]
            pub fn stable_hash(&self) -> ::napi::bindgen_prelude::BigInt {
                ::napi::bindgen_prelude::BigInt::from(::yggdryl::graph::Element::get_currhashcode(
                    &self.inner,
                ))
            }

            /// A cheap native clone.
            #[napi(js_name = "clone")]
            pub fn clone_js(&self) -> Self {
                self.clone()
            }

            /// The value's one `MarketData` row, as the base64 text of its
            /// Arrow IPC stream, so it survives `JSON.stringify`.
            #[napi(js_name = "toJSON")]
            pub fn to_json(&self) -> ::napi::bindgen_prelude::Result<String> {
                $crate::graph::market_data::into_json(::yggdryl::graph::MarketData::from(
                    self.inner.clone(),
                ))
            }

            /// Rebuild a value `toJSON` wrote.
            #[napi(factory, js_name = "fromJSON")]
            pub fn from_json(text: String) -> ::napi::bindgen_prelude::Result<Self> {
                let data = $crate::graph::market_data::from_json(&text)?;
                data.try_into()
                    .map(Self::from_core)
                    .map_err($crate::napi_error)
            }
        }
    };
}

/// The `toString` of an undated leaf: its name, its identity and its cross
/// code.
macro_rules! element_repr {
    ($class:ident, $name:literal) => {
        #[napi]
        impl $class {
            /// `<Class>(<curruuid>, crosscode=..)`.
            #[napi(js_name = "toString")]
            pub fn js_string(&self) -> String {
                format!(
                    "{}({}, crosscode={:?})",
                    $name,
                    ::yggdryl::graph::Element::get_curruuid(&self.inner),
                    ::yggdryl::graph::Element::get_crosscode(&self.inner),
                )
            }
        }
    };
}

/// What a dated leaf adds to [`common_verbs!`]: `restating`, and a
/// `toString` naming its instant.
macro_rules! event_verbs {
    ($class:ident, $name:literal) => {
        #[napi]
        impl $class {
            /// This event stated as another statement of `live`, taking
            /// live's predecessor, place and snapshot.
            #[napi]
            pub fn restating(&self, live: &$class) -> $class {
                Self::from_core(::yggdryl::graph::Event::restating(
                    self.inner.clone(),
                    &live.inner,
                ))
            }

            /// `<Class>(<curruuid>, currunix=.., crosscode=..)`.
            #[napi(js_name = "toString")]
            pub fn js_string(&self) -> String {
                format!(
                    "{}({}, currunix={}, crosscode={:?})",
                    $name,
                    ::yggdryl::graph::Element::get_curruuid(&self.inner),
                    ::yggdryl::graph::Event::get_currunix(&self.inner),
                    ::yggdryl::graph::Element::get_crosscode(&self.inner),
                )
            }
        }
    };
}

mod book;
mod candle;
mod iterator;
pub(crate) mod market_data;
mod operation;
mod trade;

pub use book::{BookLimit, JsBookEvent, JsBookIterator, JsSnapshotEvent};
pub use candle::{CandleReading, JsCandle, JsCandleIterator, JsCandleOptions};
pub use iterator::JsEventIterator;
pub use market_data::{JsMarketData, JsMarketDataRowIterator};
pub use operation::{
    BookRefInput, JsBookRef, JsExecution, JsExecutionEvent, JsOrder, JsOrderEvent, JsQuote,
    JsQuoteEvent,
};
pub use trade::JsTradeEvent;

/// Any value a market stream carries: a `MarketData` or one of its nine
/// leaves, read back to the native value it holds by [`market_data_of`].
pub(crate) type AnyMarketData<'a> = Either11<
    ClassInstance<'a, JsMarketData>,
    ClassInstance<'a, JsOrder>,
    ClassInstance<'a, JsQuote>,
    ClassInstance<'a, JsExecution>,
    ClassInstance<'a, JsOrderEvent>,
    ClassInstance<'a, JsQuoteEvent>,
    ClassInstance<'a, JsExecutionEvent>,
    ClassInstance<'a, JsTradeEvent>,
    ClassInstance<'a, JsBookEvent>,
    ClassInstance<'a, JsSnapshotEvent>,
    ClassInstance<'a, JsFixMsg>,
>;

/// The `MarketData` `item` is - a `MarketData` itself, any leaf or a
/// `FixMsg` held whole, wrapped through the core's own `From`.
pub(crate) fn market_data_of(item: &AnyMarketData<'_>) -> CoreMarketData {
    match item {
        Either11::A(data) => data.inner.clone(),
        Either11::B(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::C(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::D(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::E(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::F(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::G(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::H(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::I(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::J(leaf) => CoreMarketData::from(leaf.inner.clone()),
        Either11::K(message) => CoreMarketData::from(message.as_core().clone()),
    }
}

/// The `MarketData` a JavaScript value holds, or an error naming what it is:
/// the door a single-item argument crosses.
pub(crate) fn market_data_from(value: Unknown<'_>) -> Result<CoreMarketData> {
    let kind = value.get_type()?.to_string().to_lowercase();
    AnyMarketData::from_unknown(value)
        .map(|item| market_data_of(&item))
        .map_err(|_| {
            napi_error(format!(
                "expected MarketData, a market leaf or a FixMsg, got {kind}"
            ))
        })
}

/// One instant or grid step a caller stated, as a `bigint` or a whole
/// `number` of at most 2^53, exactly.
pub(crate) fn instant_of(value: Either<BigInt, f64>, name: &str) -> Result<i64> {
    match value {
        Either::A(value) => match value.get_i64() {
            (value, true) => Ok(value),
            (_, false) => Err(napi_error(format!(
                "{name} must fit in a signed 64-bit integer"
            ))),
        },
        Either::B(value) => exact_i64(value, name),
    }
}

/// One of the market's numbers, exact, as decimal text; `None` where the
/// market states none.
pub(crate) fn decimal_text(held: Option<Decimal>) -> Option<String> {
    held.map(|value| value.to_string())
}

/// A stated object-field slot, an owned string.
///
/// A `#[napi(object)]` struct field typed plain `Option<String>` reads
/// `undefined` (the key omitted) as `None` but refuses a literal `null`, so
/// every optional slot of the graph's input objects is typed
/// `Either<T, Null>` and collapsed back here, the one place that answers
/// "not given" for the two spellings that mean it.
pub(crate) fn optional<T>(value: Option<Either<T, Null>>) -> Option<T> {
    value.and_then(|value| match value {
        Either::A(held) => Some(held),
        Either::B(Null) => None,
    })
}

/// One named fact, resolved to the column that states it.
#[derive(Clone, Copy)]
enum Fact {
    Element(ElementColumn),
    Event(EventColumn),
    Market(MarketColumn),
    Operation(OperationColumn),
}

impl Fact {
    /// The column `name` is - folded, as the column enums read it - or an
    /// error naming the unknown fact.
    fn of_name(owner: &str, name: &str) -> Result<Self> {
        ElementColumn::of_name(name)
            .map(Self::Element)
            .or_else(|| EventColumn::of_name(name).map(Self::Event))
            .or_else(|| MarketColumn::of_name(name).map(Self::Market))
            .or_else(|| OperationColumn::of_name(name).map(Self::Operation))
            .ok_or_else(|| napi_error(format!("{owner} states no fact {name:?}")))
    }

    /// Whether the column is one of the three identifier maps, which a `Map`
    /// from each key's text to its value, an `Identifiers`, or an array of
    /// `Identifier` objects - each the one-entry map of its key - states as
    /// the map `Identifiers::from_scalar` reads it into.
    const fn is_identifier_map(self) -> bool {
        matches!(
            self,
            Self::Market(MarketColumn::SecurityIds)
                | Self::Operation(OperationColumn::Identifiers | OperationColumn::PartyIds)
        )
    }

    /// The map `value` states where this is an identifier map - a map from
    /// each key's text to its value or a sequence of them, as
    /// `Identifiers::from_scalar` reads it, refusing a key that reads as no
    /// key, a value its type refuses and two values under one key - `None`
    /// for any other column, whose value crosses as it is.
    fn identifier_map_of(self, value: &Scalar) -> Result<Option<Scalar>> {
        if !self.is_identifier_map() {
            return Ok(None);
        }
        Identifiers::from_scalar(value)
            .map(|ids| Some(ids.into_scalar()))
            .map_err(napi_error)
    }

    /// The refusal of a fact the leaf does not take from a caller: an
    /// identity, which `finalize` derives; the category, which the leaf is;
    /// `currunix` on an event, stated once as the constructor's first
    /// argument; and on an undated element every event fact.
    fn refuse_unstated(
        self,
        owner: &str,
        name: &str,
        undated: bool,
    ) -> std::result::Result<(), String> {
        match self {
            Self::Element(_)
                if !matches!(
                    self,
                    Self::Element(ElementColumn::CrossCode | ElementColumn::SrcUuids)
                ) =>
            {
                Err(format!(
                    "{owner} states no fact {name:?}: an identity is derived, never stated"
                ))
            }
            Self::Market(MarketColumn::MarketDataKind) => Err(format!(
                "{owner} states no fact {name:?}: the category is the leaf's own"
            )),
            Self::Event(_) if undated => Err(format!(
                "{owner} states no fact {name:?}: an undated element has no clock, state or chain"
            )),
            Self::Event(EventColumn::CurrUnix) => Err(format!(
                "{owner} states currunix once, as its first argument"
            )),
            Self::Element(_) | Self::Operation(_) | Self::Market(_) | Self::Event(_) => Ok(()),
        }
    }

    /// `value` checked by the column's own field - a null clears, so it
    /// crosses unchecked - and stated through the column's own `record`.
    fn state<E: Event + Operation>(self, leaf: &mut E, value: &Scalar) -> Result<()> {
        let field = match self {
            Self::Element(column) => column.field(),
            Self::Event(column) => column.field(),
            Self::Market(column) => column.field(),
            Self::Operation(column) => column.field(),
        }
        .map_err(napi_error)?;
        let checked = if matches!(value, Scalar::Null) {
            Scalar::Null
        } else {
            let stated = self
                .identifier_map_of(value)?
                .unwrap_or_else(|| value.clone());
            field.scalar(stated).map_err(napi_error)?
        };
        match self {
            Self::Element(column) => column.record(leaf, &checked),
            Self::Event(column) => column.record(leaf, &checked),
            Self::Market(column) => column.record(leaf, &checked),
            Self::Operation(column) => column.record(leaf, &checked),
        }
        Ok(())
    }
}

/// An operation of kind `K` at `currunix` with every named fact in `facts`
/// stated through its column, not yet finalized.
///
/// `facts` is one record `Scalar` keyed by column name - the loader drops a
/// key whose value is `undefined` before widening the object, so a fact not
/// given never reaches here, and a `null` one arrives as a null that clears.
/// `undated` refuses the event facts an undated element does not state,
/// naming the fact.
pub(crate) fn stated_operation<K: CoreOperationKind>(
    owner: &str,
    currunix: i64,
    facts: Option<&JsScalar>,
    undated: bool,
) -> Result<OperationEvent<K>> {
    let mut leaf = OperationEvent::<K>::at(currunix);
    let Some(facts) = facts else {
        return Ok(leaf);
    };
    let record = match &facts.inner {
        Scalar::Null => return Ok(leaf),
        record => record.as_struct().ok_or_else(|| {
            napi_error(format!(
                "{owner} facts must be a record keyed by column name, got {}",
                record.kind()
            ))
        })?,
    };
    for (name, value) in record {
        let fact = Fact::of_name(owner, name)?;
        fact.refuse_unstated(owner, name, undated)
            .map_err(napi_error)?;
        fact.state(&mut leaf, value)?;
    }
    Ok(leaf)
}

/// The identifier type an entry's own `MDEntryID(278)` is held under:
/// `graph.ENTRY_ID`'s native half.
#[napi(js_name = "_graphEntryIdNative", skip_typescript)]
pub fn graph_entry_id_native() -> &'static str {
    graph::book::ENTRY_ID.as_str()
}

/// The identifier type an entry's `MDEntryRefID(280)` is held under:
/// `graph.ENTRY_REF_ID`'s native half.
#[napi(js_name = "_graphEntryRefIdNative", skip_typescript)]
pub fn graph_entry_ref_id_native() -> &'static str {
    graph::book::ENTRY_REF_ID.as_str()
}
