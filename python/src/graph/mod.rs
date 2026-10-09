//! Native Python view of the graph vocabulary: the typed leaves - an order,
//! a quote or an execution, undated ([`operation::PyOrder`] ..) or dated
//! ([`operation::PyOrderEvent`] ..), a composite trade
//! ([`trade::PyTradeEvent`]), a book and its snapshot control
//! ([`book::PyBookEvent`], [`book::PySnapshotEvent`]) - and
//! [`market_data::PyMarketData`], the one value over every leaf.
//!
//! Nothing here resolves, folds, merges or validates a fact: a named fact
//! is resolved by the column enums' own `of_name`, checked by its column's
//! own field and stated by its column's own `record`; every other
//! constructor redirects to the core door named beside it, and a getter to
//! the trait accessor the fact answers. A value entering the boundary
//! crosses through [`crate::scalar`] once, never a second parser here.
//!
//! The fact getters and the verbs every leaf shares are written once, as
//! the segment macros below, and applied per class through
//! [`graph_methods!`], which folds the segments a class names into its one
//! `#[pymethods]` block - the crate builds `PyO3` without
//! `multiple-pymethods`, so a class has exactly one.

use std::collections::BTreeMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use yggdryl::Uuid as CoreUuid;
use yggdryl::graph::{
    ElementColumn, Event, EventColumn, FxRates, MarketColumn, MarketKind, Operation,
    OperationColumn, OperationEvent, OperationKind as CoreOperationKind,
};
use yggdryl::{Decimal, Scalar};

use crate::scalar::{PyScalar, from_py};
use crate::value_error;

/// Folds the segment macros a class names, in order, into its one
/// `#[pymethods]` block: `graph_methods!(PyX, "X"; [segment, ..]; { own
/// methods })`. Each segment appends its methods to the body and hands the
/// rest of the list back here; the empty list emits the block.
macro_rules! graph_methods {
    ($class:ident, $name:literal; []; { $($body:tt)* }) => {
        #[::pyo3::pymethods]
        impl $class {
            $($body)*
        }
    };
    ($class:ident, $name:literal; [$next:ident $(, $rest:ident)*]; { $($body:tt)* }) => {
        $next!($class, $name; [$($rest),*]; { $($body)* });
    };
}

/// The six facts [`yggdryl::graph::Element`] answers.
macro_rules! element_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// The element's own identity, as the uuid `Scalar` it is.
            #[getter]
            fn uuid(&self) -> $crate::scalar::PyScalar {
                $crate::graph::uuid_scalar(::yggdryl::graph::Element::get_uuid(&self.inner))
            }

            /// The identity every statement of one element shares: derived
            /// from the cross code, the element's own where it names none.
            #[getter]
            fn crossuuid(&self) -> $crate::scalar::PyScalar {
                $crate::graph::uuid_scalar(::yggdryl::graph::Element::get_crossuuid(&self.inner))
            }

            /// The cross code: the identifier every statement of one element
            /// shares, stored as `{kind}:{side}:{base}` - the
            /// `MarketDataKind` code, the `Side` code of a sided kind - an
            /// order or an execution - (`0` for any other, a quote among
            /// them) and the identifier itself, so a buy order `ORD-1` is
            /// `10:1:ORD-1`, a quote `14:0:Q-1` and a book `3:0:AAPL` -
            /// empty where it names none.
            #[getter]
            fn crosscode(&self) -> &str {
                ::yggdryl::graph::Element::get_crosscode(&self.inner)
            }

            /// The XXH3-64 code the element's content digests to.
            #[getter]
            fn hashcode(&self) -> u64 {
                ::yggdryl::graph::Element::get_hashcode(&self.inner)
            }

            /// The XXH3-64 of the cross code, zero where it names none.
            #[getter]
            fn crosshashcode(&self) -> u64 {
                ::yggdryl::graph::Element::get_crosshashcode(&self.inner)
            }

            /// The sorted identities of the elements this one was read from:
            /// provenance, never its chain. Empty for one built directly.
            #[getter]
            fn srcuuids(&self) -> Vec<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Element::get_srcuuids(&self.inner)
                    .iter()
                    .copied()
                    .map($crate::graph::uuid_scalar)
                    .collect()
            }
        });
    };
}

/// The facts [`yggdryl::graph::Event`] adds: the clocks, the state, its
/// place among the events of its instant, and whether the observation
/// reports an execution.
macro_rules! event_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// When the operation happened - the transaction instant:
            /// nanoseconds since the Unix epoch, UTC.
            #[getter]
            fn transunix(&self) -> i64 {
                ::yggdryl::graph::Event::get_transunix(&self.inner)
            }

            /// The lifecycle state reached, as the `State` member it is.
            #[getter]
            fn state(&self, py: ::pyo3::Python<'_>) -> ::pyo3::PyResult<::pyo3::Py<::pyo3::PyAny>> {
                $crate::graph::member(py, *::yggdryl::graph::Event::get_state(&self.inner))
            }

            /// Its place among the events of its instant: zero for the
            /// first of each run its stream hands over at that instant, with
            /// no other instant between, one more for each next.
            #[getter]
            fn seqnum(&self) -> u64 {
                ::yggdryl::graph::Event::get_seqnum(&self.inner)
            }

            /// When this was created, where known.
            #[getter]
            fn creaunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_creaunix(&self.inner)
            }

            /// When the message crossed the wire - the technical clock -
            /// where stated.
            #[getter]
            fn sendunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_sendunix(&self.inner)
            }

            /// When this expires, where it has an expiry.
            #[getter]
            fn exprunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_exprunix(&self.inner)
            }

            /// When the element this one follows happened, where it follows
            /// one.
            #[getter]
            fn prevunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_prevunix(&self.inner)
            }

            /// The identity of the element this one follows, or `None`.
            #[getter]
            fn prevuuid(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Event::get_prevuuid(&self.inner).map($crate::graph::uuid_scalar)
            }

            /// The grid step a walk read this as the snapshot of, where one
            /// did.
            #[getter]
            fn snapunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_snapunix(&self.inner)
            }

            /// Whether this observation itself reports an execution.
            #[getter]
            fn is_execution(&self) -> bool {
                ::yggdryl::graph::Event::is_execution(&self.inner)
            }
        });
    };
}

/// The facts [`yggdryl::graph::Market`] answers.
macro_rules! market_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// The price stated, as a decimal; `None` where none.
            #[getter]
            fn price(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_price(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The currency, as the `ccy` code it is; `XXX` where none.
            #[getter]
            fn currency(&self) -> $crate::scalar::PyScalar {
                $crate::graph::code_scalar(::yggdryl::graph::Market::get_currency(&self.inner))
            }

            /// The currency the instrument originates in - the one it was
            /// issued in - as the `ccy` code it is, where the element states
            /// it or a registry filled it; `None` where neither did, never
            /// the currency.
            #[getter]
            fn origccy(&self) -> Option<$crate::scalar::PyScalar> {
                let held = ::yggdryl::graph::Market::get_origccy(&self.inner);
                (!held.is_none()).then(|| $crate::graph::code_scalar(held))
            }

            /// The currency an amount converts from: `origccy` where held,
            /// else `currency` - never `XXX` where a currency is stated.
            #[getter]
            fn origin_currency(&self) -> $crate::scalar::PyScalar {
                $crate::graph::code_scalar(::yggdryl::graph::Market::origin_currency(&self.inner))
            }

            /// The quantity stated, as a decimal; `None` where none.
            #[getter]
            fn quantity(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_quantity(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The stop price the order triggers at, as a decimal; `None` where none.
            #[getter]
            fn stoppx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_stoppx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The part of the quantity shown to the market - an iceberg's peak, as a decimal; `None` where none.
            #[getter]
            fn displayqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_displayqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The part of the quantity kept from the market - an iceberg's reserve, as a decimal; `None` where none.
            #[getter]
            fn hiddenqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_hiddenqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// How much was canceled, as a decimal; `None` where none.
            #[getter]
            fn cxlqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_cxlqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The unit the quantity is counted in, as spelled; empty where
            /// none.
            #[getter]
            fn unit(&self) -> &str {
                ::yggdryl::graph::Market::get_unit(&self.inner).as_str()
            }

            /// The side, as the `Side` member it is; `Side.UKNW` where
            /// none is stated, never `None`.
            #[getter]
            fn side(&self, py: ::pyo3::Python<'_>) -> ::pyo3::PyResult<::pyo3::Py<::pyo3::PyAny>> {
                $crate::graph::member(py, ::yggdryl::graph::Market::get_side(&self.inner))
            }

            /// The type of its kind this is, as the `MarketDataType` member;
            /// `MarketDataType.UKNW` where none is stated, never `None`.
            #[getter]
            fn marketdatatype(&self, py: ::pyo3::Python<'_>) -> ::pyo3::PyResult<::pyo3::Py<::pyo3::PyAny>> {
                $crate::graph::member(py, ::yggdryl::graph::Market::get_marketdatatype(&self.inner))
            }

            /// The instrument's identifiers, each a source, a type and a code
            /// - `isin`, `cusip`, `figi` - a map keyed `src:type`, in key
            /// order.
            #[getter]
            fn securityids(&self) -> $crate::identifier::PyIdentifiers {
                $crate::identifier::PyIdentifiers::from_core(::yggdryl::graph::Market::get_securityids(&self.inner))
            }

            /// The ISIN the instrument is stated under - the `isin` entry of
            /// `securityids` - as text; `None` where none.
            #[getter]
            fn isincode(&self) -> Option<&str> {
                ::yggdryl::graph::Market::get_isincode(&self.inner)
            }

            /// The rates an amount in `currency` is divided by to state it in
            /// another currency, each decimal under its target currency's
            /// code, in currency order; empty where none.
            #[getter]
            fn fxrates(&self) -> ::std::collections::BTreeMap<String, $crate::scalar::PyScalar> {
                $crate::graph::fxrates_dict(::yggdryl::graph::Market::get_fxrates(&self.inner))
            }

            /// The best bid price stated, as a decimal; `None` where none.
            #[getter]
            fn bidpx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_bidpx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The quantity at the best bid, as a decimal; `None` where none.
            #[getter]
            fn bidqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_bidqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The currency of the bid, as the `ccy` code it is; `None` where
            /// none.
            #[getter]
            fn bidccy(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_bidccy(&self.inner).map($crate::graph::code_scalar)
            }

            /// The best ask price stated, as a decimal; `None` where none.
            #[getter]
            fn askpx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_askpx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The quantity at the best ask, as a decimal; `None` where none.
            #[getter]
            fn askqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_askqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The currency of the ask, as the `ccy` code it is; `None` where
            /// none.
            #[getter]
            fn askccy(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_askccy(&self.inner).map($crate::graph::code_scalar)
            }

            /// The instrument's classification; `None` where none.
            #[getter]
            fn cficode(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_cficode(&self.inner).map($crate::graph::code_scalar)
            }

            /// The market, as an ISO 10383 MIC; `None` where none.
            #[getter]
            fn miccode(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_miccode(&self.inner).map($crate::graph::code_scalar)
            }

            /// When this last executed: the latest execution instant its
            /// lifecycle reached, nanoseconds since the Unix epoch, UTC,
            /// where known - a market fact, never an event's.
            #[getter]
            fn execunix(&self) -> Option<i64> {
                ::yggdryl::graph::Market::get_execunix(&self.inner)
            }

            /// The price last traded at; `None` where none.
            #[getter]
            fn lastpx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_lastpx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The quantity last traded; `None` where none.
            #[getter]
            fn lastqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_lastqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The price averaged; `None` where none.
            #[getter]
            fn avgpx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_avgpx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// How much is done; `None` where none.
            #[getter]
            fn cumqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_cumqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// How much is still open; `None` where none.
            #[getter]
            fn leavesqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_leavesqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The price the step before this one settled on; `None` where
            /// none.
            #[getter]
            fn prevpx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_prevpx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The quantity the step before this one settled on; `None`
            /// where none.
            #[getter]
            fn prevqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_prevqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The spot part of an FX price; `None` where none.
            #[getter]
            fn spotrate(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_spotrate(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The forward points of an FX price; `None` where none.
            #[getter]
            fn forwardpoints(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_forwardpoints(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The ticker a person knows the instrument by; `None` where none.
            #[getter]
            fn ticker(&self) -> Option<&str> {
                ::yggdryl::graph::Market::get_ticker(&self.inner)
            }

            /// The strike price of the option the element is about, as a
            /// decimal; `None` where none.
            #[getter]
            fn strikepx(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_strikepx(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// Free-form facts beside the typed ones, in key order; empty
            /// where none.
            #[getter]
            fn metadata(&self) -> ::std::collections::BTreeMap<String, String> {
                ::yggdryl::graph::Market::get_metadata(&self.inner)
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect()
            }
        });
    };
}

/// The facts [`yggdryl::graph::Operation`] adds.
macro_rules! operation_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// The quantity ordered, as a decimal; `None` where none.
            #[getter]
            fn ordqty(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Operation::get_ordqty(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// How long this stands, as the `TimeInForce` member; `None`
            /// where unstated.
            #[getter]
            fn timeinforce(&self, py: ::pyo3::Python<'_>) -> ::pyo3::PyResult<::std::option::Option<::pyo3::Py<::pyo3::PyAny>>> {
                ::yggdryl::graph::Operation::get_timeinforce(&self.inner)
                    .map(|held| $crate::graph::member(py, *held))
                    .transpose()
            }

            /// Whether the instrument trades, or `None` where the market said
            /// nothing either way - which is not `False`.
            #[getter]
            fn tradable(&self) -> Option<bool> {
                ::yggdryl::graph::Operation::get_tradable(&self.inner)
            }

            /// The names the operation goes by - its order, client order and
            /// execution ids with the parents a chain gave them - a map keyed
            /// `src:type`, in key order.
            #[getter]
            fn identifiers(&self) -> $crate::identifier::PyIdentifiers {
                $crate::identifier::PyIdentifiers::from_core(::yggdryl::graph::Operation::get_identifiers(&self.inner))
            }

            /// The parties the operation names, each typed by its role -
            /// `customeraccount`, `executingtrader` - from its source, a map
            /// keyed `src:type`.
            #[getter]
            fn partyids(&self) -> $crate::identifier::PyIdentifiers {
                $crate::identifier::PyIdentifiers::from_core(::yggdryl::graph::Operation::get_partyids(&self.inner))
            }
        });
    };
}

/// The market data kind of a leaf: the `MarketDataKind` member its
/// [`LeafKind::marketdatakind`] files it under.
macro_rules! kind_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// The kind of market data this is, as the `MarketDataKind`
            /// member its leaf is filed under: `ORDR`, `QUOT`, `EXEC`,
            /// `TRAD` or `BOOK` - and, for a FIX message held whole, the
            /// category its dictionary files it under.
            #[getter]
            fn marketdatakind(&self, py: ::pyo3::Python<'_>) -> ::pyo3::PyResult<::pyo3::Py<::pyo3::PyAny>> {
                $crate::graph::member(
                    py,
                    $crate::graph::LeafKind::marketdatakind(&self.inner),
                )
            }
        });
    };
}

/// The verbs every leaf and `MarketData` share: following, merging, the
/// order, equality, the hash, copies and pickle - which carries the value's
/// one-row `MarketData.arrow_reader` IPC stream and rebuilds it through
/// `MarketData.from_arrow_reader`, exact because the row states every
/// identity.
macro_rules! common_verbs {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// This value stated as the one after `previous`, or `None` where
            /// it cannot follow it or following changes nothing. It takes
            /// every `metadata` key of its chain it lacks and, where it names
            /// identifiers, every `identifiers` type but `mdentryrefid` and every
            /// party id, its own values standing.
            fn with_previous(&self, previous: &Self) -> Option<Self> {
                ::yggdryl::graph::Element::with_previous(self.inner.clone(), &previous.inner)
                    .map(Self::from_core)
            }

            /// This value with another statement of `other` folded in, or
            /// `None` for another element or a fold that changes nothing.
            fn merge_with(&self, other: &Self) -> Option<Self> {
                ::yggdryl::graph::Element::merge_with(self.inner.clone(), &other.inner)
                    .map(Self::from_core)
            }

            /// Whether this value comes after `other` in its order.
            fn is_after(&self, other: &Self) -> bool {
                ::yggdryl::graph::Element::is_after(&self.inner, &other.inner)
            }

            /// Whether this value comes before `other` in its order.
            fn is_before(&self, other: &Self) -> bool {
                ::yggdryl::graph::Element::is_before(&self.inner, &other.inner)
            }

            fn __eq__(
                &self,
                py: ::pyo3::Python<'_>,
                other: &::pyo3::Bound<'_, ::pyo3::PyAny>,
            ) -> ::pyo3::Py<::pyo3::PyAny> {
                use ::pyo3::types::PyAnyMethods as _;
                let Ok(other) = other.extract::<::pyo3::PyRef<'_, Self>>() else {
                    return py.NotImplemented();
                };
                ::pyo3::types::PyBool::new(py, self.inner == other.inner)
                    .to_owned()
                    .into_any()
                    .unbind()
            }

            /// Hashes by the code the content digests to, which equal values
            /// share.
            fn __hash__(&self) -> isize {
                $crate::python_hash(::yggdryl::graph::Element::get_hashcode(&self.inner))
            }

            fn __copy__(&self) -> Self {
                self.clone()
            }

            fn __deepcopy__(&self, _memo: &::pyo3::Bound<'_, ::pyo3::PyAny>) -> Self {
                self.clone()
            }

            /// Rebuild a value pickle carried: the IPC stream of its one
            /// `MarketData` row.
            #[staticmethod]
            fn _from_pickle(stream: &[u8]) -> ::pyo3::PyResult<Self> {
                let data = $crate::graph::market_data::from_ipc(stream)?;
                data.try_into().map(Self::from_core).map_err($crate::value_error)
            }

            fn __reduce__<'py>(
                &self,
                py: ::pyo3::Python<'py>,
            ) -> ::pyo3::PyResult<(
                ::pyo3::Bound<'py, ::pyo3::PyAny>,
                ::pyo3::Bound<'py, ::pyo3::types::PyTuple>,
            )> {
                use ::pyo3::types::PyAnyMethods as _;
                // A FIX message held whole pickles as itself: its row would
                // split into the leaves it reports.
                if let Some(message) = $crate::graph::LeafKind::held_fix(&self.inner) {
                    let whole = ::pyo3::Py::new(py, $crate::fix::PyFixMsg::from_inner(message.clone()))?;
                    return Ok((
                        py.get_type::<Self>().into_any(),
                        ::pyo3::types::PyTuple::new(py, [whole])?,
                    ));
                }
                let stream = $crate::graph::market_data::into_ipc(
                    ::yggdryl::graph::MarketData::from(self.inner.clone()),
                )?;
                Ok((
                    py.get_type::<Self>().getattr("_from_pickle")?,
                    ::pyo3::types::PyTuple::new(py, [::pyo3::types::PyBytes::new(py, &stream)])?,
                ))
            }
        });
    };
}

/// The `repr` of an undated leaf: its name, its identity and its cross code.
macro_rules! element_repr {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            fn __repr__(&self) -> String {
                format!(
                    "{}({}, crosscode={:?})",
                    $name,
                    ::yggdryl::graph::Element::get_uuid(&self.inner),
                    ::yggdryl::graph::Element::get_crosscode(&self.inner),
                )
            }
        });
    };
}

/// What a dated leaf adds to [`common_verbs!`]: `restating`, and a `repr`
/// naming its instant.
macro_rules! event_verbs {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// This event stated as another statement of `live`, taking
            /// live's predecessor, place and snapshot.
            fn restating(&self, live: &Self) -> Self {
                Self::from_core(::yggdryl::graph::Event::restating(self.inner.clone(), &live.inner))
            }

            fn __repr__(&self) -> String {
                format!(
                    "{}({}, transunix={}, crosscode={:?})",
                    $name,
                    ::yggdryl::graph::Element::get_uuid(&self.inner),
                    ::yggdryl::graph::Event::get_transunix(&self.inner),
                    ::yggdryl::graph::Element::get_crosscode(&self.inner),
                )
            }
        });
    };
}

pub(crate) mod book;
pub(crate) mod candle;
pub(crate) mod iterator;
pub(crate) mod market_data;
pub(crate) mod operation;
pub(crate) mod trade;

/// A native identity as the uuid `Scalar` it is: the binding has no `Uuid`
/// class of its own, and `as_py()` answers the hyphenated text.
pub(crate) fn uuid_scalar(uuid: CoreUuid) -> PyScalar {
    PyScalar::from_inner(Scalar::Uuid(uuid))
}

/// A native enum fact - a state, a side, a market data kind - as the member
/// of its Python enum, through the one `Scalar.as_py` door.
pub(crate) fn member<E>(py: Python<'_>, held: E) -> PyResult<Py<PyAny>>
where
    Scalar: From<E>,
{
    crate::scalar::as_py(py, &Scalar::from(held))
}

/// Which leaf a native value is, answered by the leaf types themselves: what
/// [`kind_getters!`] reads a leaf's `MarketDataKind` through.
pub(crate) trait LeafKind {
    /// The leaf kind this value is.
    fn market_kind(&self) -> MarketKind;

    /// The `MarketDataKind` this value is filed under.
    fn marketdatakind(&self) -> yggdryl::MarketDataKind {
        self.market_kind().marketdatakind()
    }

    /// The FIX message this value holds whole, where it is one: what its
    /// pickle carries instead of the rows the message splits into.
    fn held_fix(&self) -> Option<&yggdryl::FixMsg> {
        None
    }
}

impl<K: CoreOperationKind> LeafKind for yggdryl::graph::OperationElement<K> {
    fn market_kind(&self) -> MarketKind {
        self.kind()
    }
}

impl<K: CoreOperationKind> LeafKind for OperationEvent<K> {
    fn market_kind(&self) -> MarketKind {
        self.kind()
    }
}

impl LeafKind for yggdryl::graph::TradeEvent {
    fn market_kind(&self) -> MarketKind {
        MarketKind::TradeEvent
    }
}

impl LeafKind for yggdryl::graph::BookEvent {
    fn market_kind(&self) -> MarketKind {
        MarketKind::BookEvent
    }
}

impl LeafKind for yggdryl::graph::SnapshotEvent {
    fn market_kind(&self) -> MarketKind {
        MarketKind::SnapshotEvent
    }
}

impl LeafKind for yggdryl::graph::MarketData {
    fn market_kind(&self) -> MarketKind {
        self.kind()
    }

    fn marketdatakind(&self) -> yggdryl::MarketDataKind {
        yggdryl::graph::MarketData::marketdatakind(self)
    }

    fn held_fix(&self) -> Option<&yggdryl::FixMsg> {
        self.as_message::<yggdryl::FixMsg>()
    }
}

/// A native code - a currency, an identifier - as the code `Scalar` its
/// datatype is.
pub(crate) fn code_scalar<C>(code: &C) -> PyScalar
where
    C: Clone,
    Scalar: From<C>,
{
    PyScalar::from_inner(Scalar::from(code.clone()))
}

/// One of the market's numbers, exact, as the decimal `Scalar` it is.
pub(crate) fn decimal_scalar(held: Decimal) -> PyScalar {
    PyScalar::from_inner(Scalar::from(held))
}

/// The rates an element states, each decimal `Scalar` under its target
/// currency's code, in currency order.
pub(crate) fn fxrates_dict(rates: &FxRates) -> BTreeMap<String, PyScalar> {
    rates
        .iter()
        .map(|(target, rate)| (target.as_str().to_owned(), decimal_scalar(*rate)))
        .collect()
}

/// One optional slot of a `repr`, as Python spells it: `None` where the
/// slot states nothing, else its text quoted.
pub(crate) fn slot_repr<T: std::fmt::Display>(slot: Option<T>) -> String {
    slot.map_or_else(
        || "None".to_owned(),
        |value| format!("{:?}", value.to_string()),
    )
}

/// The literal `Ellipsis` object, as a signature default: this project's
/// spelling for a keyword argument that was not given. A bare `py.Ellipsis()`
/// does not work as a `#[pyo3(signature = ...)]` default expression, so this
/// is the one indirection every per-slot constructor's signature calls.
#[allow(clippy::redundant_closure_for_method_calls)] // `Python::Ellipsis` alone fails HRTB inference.
pub(crate) fn ellipsis() -> Py<PyAny> {
    Python::attach(|py| py.Ellipsis())
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
    fn of_name(owner: &str, name: &str) -> PyResult<Self> {
        ElementColumn::of_name(name)
            .map(Self::Element)
            .or_else(|| EventColumn::of_name(name).map(Self::Event))
            .or_else(|| MarketColumn::of_name(name).map(Self::Market))
            .or_else(|| OperationColumn::of_name(name).map(Self::Operation))
            .ok_or_else(|| PyValueError::new_err(format!("{owner} states no fact {name:?}")))
    }

    /// Whether the column is one of the three identifier maps, which a
    /// `dict` from each key's text to its value, an `Identifiers`, or a
    /// sequence of `Identifier` objects - each the one-entry map of its key
    /// - states as the map `Identifiers::from_scalar` reads it into.
    const fn is_identifier_map(self) -> bool {
        matches!(
            self,
            Self::Market(MarketColumn::SecurityIds)
                | Self::Operation(OperationColumn::Identifiers | OperationColumn::PartyIds)
        )
    }

    /// The refusal of a fact the leaf does not take from a caller: an
    /// identity, which `finalize` derives; the category, which the leaf is;
    /// `transunix` on an event, stated once as the constructor's first
    /// argument; and on an undated element every event fact.
    fn refuse_unstated(
        self,
        owner: &str,
        name: &str,
        undated: bool,
    ) -> std::result::Result<(), String> {
        match self {
            Self::Market(MarketColumn::MarketDataKind) => Err(format!(
                "{owner} states no fact {name:?}: the category is the leaf's own"
            )),
            Self::Element(ElementColumn::CrossCode | ElementColumn::SrcUuids)
            | Self::Operation(_)
            | Self::Market(_) => Ok(()),
            Self::Element(_) => Err(format!(
                "{owner} states no fact {name:?}: an identity is derived, never stated"
            )),
            Self::Event(_) if undated => Err(format!(
                "{owner} states no fact {name:?}: an undated element has no clock, state or chain"
            )),
            Self::Event(EventColumn::TransUnix) => Err(format!(
                "{owner} states transunix once, as its first argument"
            )),
            Self::Event(_) => Ok(()),
        }
    }

    /// `value` checked by the column's own field - a null clears, so it
    /// crosses unchecked - and stated through the column's own `record`.
    fn state<E: Event + Operation>(self, leaf: &mut E, value: &Scalar) -> PyResult<()> {
        let field = match self {
            Self::Element(column) => column.field(),
            Self::Event(column) => column.field(),
            Self::Market(column) => column.field(),
            Self::Operation(column) => column.field(),
        }
        .map_err(value_error)?;
        let checked = if matches!(value, Scalar::Null) {
            Scalar::Null
        } else {
            field.scalar(value.clone()).map_err(value_error)?
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

/// An operation of kind `K` at `transunix` with every named fact in `facts`
/// stated through its column, not yet finalized. A value given as the
/// literal `Ellipsis` is skipped and `None` clears; `undated` refuses the
/// event facts an undated element does not state, naming the fact.
pub(crate) fn stated_operation<K: CoreOperationKind>(
    owner: &str,
    transunix: i64,
    facts: Option<&Bound<'_, PyDict>>,
    undated: bool,
) -> PyResult<OperationEvent<K>> {
    let mut leaf = OperationEvent::<K>::at(transunix);
    let Some(facts) = facts else {
        return Ok(leaf);
    };
    let py = facts.py();
    for (key, value) in facts.iter() {
        if value.is(py.Ellipsis()) {
            continue;
        }
        let name: String = key.extract()?;
        let fact = Fact::of_name(owner, &name)?;
        fact.refuse_unstated(owner, &name, undated)
            .map_err(PyValueError::new_err)?;
        let scalar = if value.is_none() {
            Scalar::Null
        } else if fact.is_identifier_map() {
            ::yggdryl::Identifiers::from_scalar(&from_py(&value)?)
                .map_err(value_error)?
                .into_scalar()
        } else {
            from_py(&value)?
        };
        fact.state(&mut leaf, &scalar)?;
    }
    Ok(leaf)
}

/// Register every graph class and the module's constants.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<operation::PyBookRef>()?;
    module.add_class::<operation::PyOrder>()?;
    module.add_class::<operation::PyQuote>()?;
    module.add_class::<operation::PyExecution>()?;
    module.add_class::<operation::PyOrderEvent>()?;
    module.add_class::<operation::PyQuoteEvent>()?;
    module.add_class::<operation::PyExecutionEvent>()?;
    module.add_class::<trade::PyTradeEvent>()?;
    module.add_class::<book::PyBookEvent>()?;
    module.add_class::<book::PySnapshotEvent>()?;
    module.add_class::<book::PyBookIterator>()?;
    module.add_class::<market_data::PyMarketData>()?;
    module.add_class::<market_data::PyMarketDataRowIterator>()?;
    module.add_class::<iterator::PyEventIterator>()?;
    module.add_class::<candle::PyCandle>()?;
    module.add_class::<candle::PyCandleOptions>()?;
    module.add_class::<candle::PyCandleIterator>()?;
    module.add_function(wrap_pyfunction!(candle::candles, module)?)?;
    // The identifier types a market-data entry's own identifiers are held
    // under.
    module.add("ENTRY_ID", yggdryl::graph::book::ENTRY_ID.as_str())?;
    module.add("ENTRY_REF_ID", yggdryl::graph::book::ENTRY_REF_ID.as_str())?;
    Ok(())
}
