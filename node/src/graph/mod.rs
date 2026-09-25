//! Native Node.js view of the graph vocabulary: the typed leaves - an order,
//! a quote or an execution, undated ([`operation::JsOrder`] ..) or dated
//! ([`operation::JsOrderEvent`] ..), a composite trade
//! ([`trade::JsTradeEvent`]), a book, its sides and its snapshot control
//! ([`book::JsBookEvent`], [`book::JsBookSide`], [`book::JsSnapshotEvent`])
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
//! `number`, a decimal as its text, a code as the text it is, an identifier
//! map as a `Record<string, string>`, a lane as a [`operation::JsLane`].
//!
//! The fact getters and the verbs every leaf shares are written once, as the
//! macros below, each emitting its own `#[napi] impl` block for the class it
//! is applied to; NAPI merges every block of one class into its prototype.

use std::collections::BTreeMap;

use napi::bindgen_prelude::{
    BigInt, ClassInstance, Either, Either11, FromNapiValue, Null, Result, Unknown,
};
use napi_derive::napi;
use yggdryl::graph::{
    Event, EventColumn, MarketColumn, MarketData as CoreMarketData, Operation, OperationColumn,
    OperationEvent, OperationKind as CoreOperationKind,
};
use yggdryl::idmap::IdMap as CoreIdMap;
use yggdryl::securityid::SecurityIds as CoreSecurityIds;
use yggdryl::{Decimal18, Scalar, graph};

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
            /// shares, empty where it names none.
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

/// The facts [`yggdryl::graph::Event`] adds: the clocks, the state, the
/// place in the chain, and whether the observation reports an execution.
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

            /// How many elements came before this one in its chain.
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

            /// The latest execution instant the lifecycle reached, where
            /// known.
            #[napi(getter)]
            pub fn execunix(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_execunix(&self.inner)
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
            pub fn exprtime(&self) -> Option<::napi::bindgen_prelude::BigInt> {
                ::yggdryl::graph::Event::get_exprtime(&self.inner)
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

/// The nineteen facts [`yggdryl::graph::Market`] answers.
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

            /// The unit the quantity is counted in, as spelled; empty where
            /// none.
            #[napi(getter)]
            pub fn unit(&self) -> String {
                ::yggdryl::graph::Market::get_unit(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// The side, as the `side` code it is; `UNKNOWN` where none.
            #[napi(getter)]
            pub fn side(&self) -> String {
                ::yggdryl::graph::Market::get_side(&self.inner)
                    .as_str()
                    .to_owned()
            }

            /// The instrument's identifiers, one code under each source -
            /// `ISIN`, `CUSIP`, `FIGI` - in source order.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn securityids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::securityids_record(::yggdryl::graph::Market::get_securityids(
                    &self.inner,
                ))
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

/// The eight facts [`yggdryl::graph::Operation`] adds.
macro_rules! operation_getters {
    ($class:ident) => {
        #[napi]
        impl $class {
            /// The stable integer market-operation category, or `null`.
            #[napi(getter)]
            pub fn marketoperationid(&self) -> Option<i32> {
                ::yggdryl::graph::Operation::get_marketoperationid(&self.inner)
            }

            /// How long this stands, as the stored code; `null` where
            /// unstated.
            #[napi(getter)]
            pub fn tif(&self) -> Option<String> {
                ::yggdryl::graph::Operation::get_tif(&self.inner)
                    .map(|held| held.as_str().to_owned())
            }

            /// Whether the instrument trades, or `null` where the market said
            /// nothing either way - which is not `false`.
            #[napi(getter)]
            pub fn tradable(&self) -> Option<bool> {
                ::yggdryl::graph::Operation::get_tradable(&self.inner)
            }

            /// The accounts the operation is for, in key order.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn accountids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::idmap_record(::yggdryl::graph::Operation::get_accountids(
                    &self.inner,
                ))
            }

            /// The users the operation is by, in key order.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn userids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::idmap_record(::yggdryl::graph::Operation::get_userids(&self.inner))
            }

            /// The names the operation goes by, in key order.
            #[napi(getter, ts_return_type = "Record<string, string>")]
            pub fn altids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::idmap_record(::yggdryl::graph::Operation::get_altids(&self.inner))
            }

            /// The bid lane a quote states; `null` where none.
            #[napi(getter)]
            pub fn bid(&self) -> Option<$crate::graph::JsLane> {
                ::yggdryl::graph::Operation::get_bid(&self.inner)
                    .cloned()
                    .map($crate::graph::JsLane::from_core)
            }

            /// The ask lane, shaped as the bid; `null` where none.
            #[napi(getter)]
            pub fn ask(&self) -> Option<$crate::graph::JsLane> {
                ::yggdryl::graph::Operation::get_ask(&self.inner)
                    .cloned()
                    .map($crate::graph::JsLane::from_core)
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
            /// it cannot follow it or following changes nothing.
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
            /// This event stated as another statement of `live`, taking the
            /// place `live` holds in its chain.
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
mod iterator;
pub(crate) mod market_data;
mod operation;
mod trade;

pub use book::{
    JsBookEvent, JsBookIterator, JsBookSide, JsSnapshotEvent, JsSnapshotPartition,
    SnapshotPartitionInput,
};
pub use iterator::JsEventIterator;
pub use market_data::{JsMarketData, JsMarketDataRowIterator};
pub use operation::{
    BookRefInput, JsBookRef, JsExecution, JsExecutionEvent, JsLane, JsOrder, JsOrderEvent, JsQuote,
    JsQuoteEvent, LaneInput,
};
pub use trade::JsTradeEvent;

/// Any value a market stream carries: a `MarketData` or one of its ten
/// leaves, read back to the native value it holds by [`market_data_of`].
pub(crate) type AnyMarketData<'a> = Either11<
    ClassInstance<'a, JsMarketData>,
    ClassInstance<'a, JsOrder>,
    ClassInstance<'a, JsQuote>,
    ClassInstance<'a, JsExecution>,
    ClassInstance<'a, JsBookSide>,
    ClassInstance<'a, JsOrderEvent>,
    ClassInstance<'a, JsQuoteEvent>,
    ClassInstance<'a, JsExecutionEvent>,
    ClassInstance<'a, JsTradeEvent>,
    ClassInstance<'a, JsBookEvent>,
    ClassInstance<'a, JsSnapshotEvent>,
>;

/// The `MarketData` `item` is - a `MarketData` itself or any leaf, wrapped
/// through the core's own `From`.
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
        Either11::K(leaf) => CoreMarketData::from(leaf.inner.clone()),
    }
}

/// The `MarketData` a JavaScript value holds, or an error naming what it is:
/// the door a single-item argument crosses.
pub(crate) fn market_data_from(value: Unknown<'_>) -> Result<CoreMarketData> {
    let kind = value.get_type()?.to_string().to_lowercase();
    AnyMarketData::from_unknown(value)
        .map(|item| market_data_of(&item))
        .map_err(|_| napi_error(format!("expected MarketData or a market leaf, got {kind}")))
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
pub(crate) fn decimal_text(held: Option<Decimal18>) -> Option<String> {
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

/// An identifier map - the accounts, the users, the names an operation goes
/// by - as the record JavaScript reads, each value under the key that
/// stated it, in key order.
pub(crate) fn idmap_record(ids: &CoreIdMap) -> BTreeMap<String, String> {
    ids.iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// The identifiers an instrument is stated under, one code under each
/// source - `ISIN`, `CUSIP`, `FIGI` - in source order.
pub(crate) fn securityids_record(ids: &CoreSecurityIds) -> BTreeMap<String, String> {
    ids.iter()
        .map(|id| (id.sectype().as_str().to_owned(), id.code().to_owned()))
        .collect()
}

/// The event columns an undated element states: the facts
/// [`yggdryl::graph::Element`] answers that `finalize` keeps, and no clock,
/// state or chain.
const ELEMENT_COLUMNS: [EventColumn; 2] = [EventColumn::CrossCode, EventColumn::SrcUuids];

/// The identity columns `finalize` derives from the stated facts, so no
/// caller states one: a value given would be overwritten, never kept.
const DERIVED_COLUMNS: [EventColumn; 4] = [
    EventColumn::CurrUuid,
    EventColumn::CrossUuid,
    EventColumn::CurrHashCode,
    EventColumn::CrossHashCode,
];

/// One named fact, resolved to the column that states it.
#[derive(Clone, Copy)]
enum Fact {
    Event(EventColumn),
    Market(MarketColumn),
    Operation(OperationColumn),
}

impl Fact {
    /// The column `name` is - folded, as the column enums read it - or an
    /// error naming the unknown fact.
    fn of_name(owner: &str, name: &str) -> Result<Self> {
        EventColumn::of_name(name)
            .map(Self::Event)
            .or_else(|| MarketColumn::of_name(name).map(Self::Market))
            .or_else(|| OperationColumn::of_name(name).map(Self::Operation))
            .ok_or_else(|| napi_error(format!("{owner} states no fact {name:?}")))
    }

    /// The refusal of a fact the leaf does not take from a caller: a
    /// derived identity, which `finalize` computes; `currunix` on an event,
    /// stated once as the constructor's first argument; and on an undated
    /// element every clock, state and chain fact.
    fn refuse_unstated(
        self,
        owner: &str,
        name: &str,
        undated: bool,
    ) -> std::result::Result<(), String> {
        let Self::Event(column) = self else {
            return Ok(());
        };
        if DERIVED_COLUMNS.contains(&column) {
            Err(format!(
                "{owner} states no fact {name:?}: an identity is derived, never stated"
            ))
        } else if undated && !ELEMENT_COLUMNS.contains(&column) {
            Err(format!(
                "{owner} states no fact {name:?}: an undated element has no clock, state or chain"
            ))
        } else if column == EventColumn::CurrUnix {
            Err(format!(
                "{owner} states currunix once, as its first argument"
            ))
        } else {
            Ok(())
        }
    }

    /// `value` checked by the column's own field - a null clears, so it
    /// crosses unchecked - and stated through the column's own `record`.
    fn state<E: Event + Operation>(self, leaf: &mut E, value: &Scalar) -> Result<()> {
        let field = match self {
            Self::Event(column) => column.field(),
            Self::Market(column) => column.field(),
            Self::Operation(column) => column.field(),
        }
        .map_err(napi_error)?;
        let checked = if matches!(value, Scalar::Null) {
            Scalar::Null
        } else {
            field.scalar(value.clone()).map_err(napi_error)?
        };
        match self {
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

/// The symbol of the one consolidated book a global walk emits:
/// `graph.GLOBAL_SYMBOL`'s native half.
#[napi(js_name = "_graphGlobalSymbolNative", skip_typescript)]
pub fn graph_global_symbol_native() -> &'static str {
    graph::GLOBAL_SYMBOL
}

/// The alternate-identifier key an entry's own `MDEntryID(278)` is held
/// under: `graph.ENTRY_ID`'s native half.
#[napi(js_name = "_graphEntryIdNative", skip_typescript)]
pub fn graph_entry_id_native() -> &'static str {
    graph::book::ENTRY_ID
}

/// The alternate-identifier key an entry's `MDEntryRefID(280)` is held
/// under: `graph.ENTRY_REF_ID`'s native half.
#[napi(js_name = "_graphEntryRefIdNative", skip_typescript)]
pub fn graph_entry_ref_id_native() -> &'static str {
    graph::book::ENTRY_REF_ID
}

/// The alternate-identifier keys an order's own identifiers may follow
/// across a lifecycle: `graph.FOLLOWED_ALTIDS`'s native half.
#[napi(js_name = "_graphFollowedAltidsNative", skip_typescript)]
pub fn graph_followed_altids_native() -> Vec<String> {
    graph::FOLLOWED_ALTIDS
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}
