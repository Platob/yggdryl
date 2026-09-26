//! Native Python view of the graph vocabulary: the typed leaves - an order,
//! a quote or an execution, undated ([`operation::PyOrder`] ..) or dated
//! ([`operation::PyOrderEvent`] ..), a composite trade
//! ([`trade::PyTradeEvent`]), a book, its sides and its snapshot control
//! ([`book::PyBookEvent`], [`book::PyBookSide`], [`book::PySnapshotEvent`])
//! - and [`market_data::PyMarketData`], the one value over every leaf.
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
use pyo3::types::{PyDict, PyTuple};

use yggdryl::Uuid as CoreUuid;
use yggdryl::graph::{
    Event, EventColumn, MarketColumn, Operation, OperationColumn, OperationEvent,
    OperationKind as CoreOperationKind,
};
use yggdryl::idmap::IdMap as CoreIdMap;
use yggdryl::securityid::SecurityIds as CoreSecurityIds;
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
            fn curruuid(&self) -> $crate::scalar::PyScalar {
                $crate::graph::uuid_scalar(::yggdryl::graph::Element::get_curruuid(&self.inner))
            }

            /// The identity every statement of one element shares: derived
            /// from the cross code, the element's own where it names none.
            #[getter]
            fn crossuuid(&self) -> $crate::scalar::PyScalar {
                $crate::graph::uuid_scalar(::yggdryl::graph::Element::get_crossuuid(&self.inner))
            }

            /// The cross code: the identifier every statement of one element
            /// shares, empty where it names none.
            #[getter]
            fn crosscode(&self) -> &str {
                ::yggdryl::graph::Element::get_crosscode(&self.inner)
            }

            /// The XXH3-64 code the element's content digests to.
            #[getter]
            fn currhashcode(&self) -> u64 {
                ::yggdryl::graph::Element::get_currhashcode(&self.inner)
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

/// The facts [`yggdryl::graph::Event`] adds: the clocks, the state, the
/// place in the chain, and whether the observation reports an execution.
macro_rules! event_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// When this happened: nanoseconds since the Unix epoch, UTC.
            #[getter]
            fn currunix(&self) -> i64 {
                ::yggdryl::graph::Event::get_currunix(&self.inner)
            }

            /// The lifecycle state reached, as the `state` code it is.
            #[getter]
            fn state(&self) -> $crate::scalar::PyScalar {
                $crate::graph::code_scalar(::yggdryl::graph::Event::get_state(&self.inner))
            }

            /// How many elements came before this one in its chain.
            #[getter]
            fn seqnum(&self) -> u64 {
                ::yggdryl::graph::Event::get_seqnum(&self.inner)
            }

            /// When this was created, where known.
            #[getter]
            fn creaunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_creaunix(&self.inner)
            }

            /// The latest execution instant the lifecycle reached, where
            /// known.
            #[getter]
            fn execunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_execunix(&self.inner)
            }

            /// When this was recorded, where stated.
            #[getter]
            fn recdunix(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_recdunix(&self.inner)
            }

            /// When this expires, where it has an expiry.
            #[getter]
            fn exprtime(&self) -> Option<i64> {
                ::yggdryl::graph::Event::get_exprtime(&self.inner)
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

/// The nineteen facts [`yggdryl::graph::Market`] answers.
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

            /// The quantity stated, as a decimal; `None` where none.
            #[getter]
            fn quantity(&self) -> Option<$crate::scalar::PyScalar> {
                ::yggdryl::graph::Market::get_quantity(&self.inner).map($crate::graph::decimal_scalar)
            }

            /// The unit the quantity is counted in, as spelled; empty where
            /// none.
            #[getter]
            fn unit(&self) -> &str {
                ::yggdryl::graph::Market::get_unit(&self.inner).as_str()
            }

            /// The side, as the `side` code it is; `UNKNOWN` where none.
            #[getter]
            fn side(&self) -> $crate::scalar::PyScalar {
                $crate::scalar::PyScalar::from_inner(::yggdryl::Scalar::from(
                    ::yggdryl::graph::Market::get_side(&self.inner),
                ))
            }

            /// The instrument's identifiers, one code under each source -
            /// `ISIN`, `CUSIP`, `FIGI` - in source order.
            #[getter]
            fn securityids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::securityids_dict(::yggdryl::graph::Market::get_securityids(&self.inner))
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

/// The eight facts [`yggdryl::graph::Operation`] adds.
macro_rules! operation_getters {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// The stable integer market-operation category, or `None`.
            #[getter]
            fn marketoperationid(&self) -> Option<i32> {
                ::yggdryl::graph::Operation::get_marketoperationid(&self.inner)
            }

            /// How long this stands, as the stored code; `None` where
            /// unstated.
            #[getter]
            fn tif(&self) -> Option<&str> {
                ::yggdryl::graph::Operation::get_tif(&self.inner).map(::yggdryl::TimeInForce::as_str)
            }

            /// Whether the instrument trades, or `None` where the market said
            /// nothing either way - which is not `False`.
            #[getter]
            fn tradable(&self) -> Option<bool> {
                ::yggdryl::graph::Operation::get_tradable(&self.inner)
            }

            /// The accounts the operation is for, in key order.
            #[getter]
            fn accountids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::idmap_dict(::yggdryl::graph::Operation::get_accountids(&self.inner))
            }

            /// The users the operation is by, in key order.
            #[getter]
            fn userids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::idmap_dict(::yggdryl::graph::Operation::get_userids(&self.inner))
            }

            /// The names the operation goes by, in key order.
            #[getter]
            fn altids(&self) -> ::std::collections::BTreeMap<String, String> {
                $crate::graph::idmap_dict(::yggdryl::graph::Operation::get_altids(&self.inner))
            }

            /// The bid lane a quote states; `None` where none.
            #[getter]
            fn bid(&self) -> Option<$crate::graph::operation::PyLane> {
                ::yggdryl::graph::Operation::get_bid(&self.inner)
                    .cloned()
                    .map($crate::graph::operation::PyLane::from_core)
            }

            /// The ask lane, shaped as the bid; `None` where none.
            #[getter]
            fn ask(&self) -> Option<$crate::graph::operation::PyLane> {
                ::yggdryl::graph::Operation::get_ask(&self.inner)
                    .cloned()
                    .map($crate::graph::operation::PyLane::from_core)
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
            /// it cannot follow it or following changes nothing.
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
                $crate::python_hash(::yggdryl::graph::Element::get_currhashcode(&self.inner))
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
                (::pyo3::Bound<'py, ::pyo3::types::PyBytes>,),
            )> {
                use ::pyo3::types::PyAnyMethods as _;
                let stream = $crate::graph::market_data::into_ipc(
                    ::yggdryl::graph::MarketData::from(self.inner.clone()),
                )?;
                Ok((
                    py.get_type::<Self>().getattr("_from_pickle")?,
                    (::pyo3::types::PyBytes::new(py, &stream),),
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
                    ::yggdryl::graph::Element::get_curruuid(&self.inner),
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
            /// This event stated as another statement of `live`, taking the
            /// place `live` holds in its chain.
            fn restating(&self, live: &Self) -> Self {
                Self::from_core(::yggdryl::graph::Event::restating(self.inner.clone(), &live.inner))
            }

            fn __repr__(&self) -> String {
                format!(
                    "{}({}, currunix={}, crosscode={:?})",
                    $name,
                    ::yggdryl::graph::Element::get_curruuid(&self.inner),
                    ::yggdryl::graph::Event::get_currunix(&self.inner),
                    ::yggdryl::graph::Element::get_crosscode(&self.inner),
                )
            }
        });
    };
}

pub(crate) mod book;
pub(crate) mod iterator;
pub(crate) mod market_data;
pub(crate) mod operation;
pub(crate) mod trade;

/// A native identity as the uuid `Scalar` it is: the binding has no `Uuid`
/// class of its own, and `as_py()` answers the hyphenated text.
pub(crate) fn uuid_scalar(uuid: CoreUuid) -> PyScalar {
    PyScalar::from_inner(Scalar::Uuid(uuid))
}

/// A native code - a currency, a side, a state, an identifier - as the
/// code `Scalar` its datatype is.
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

/// An identifier map - the accounts, the users, the names an operation
/// goes by - as the `dict` Python reads, each value under the key that
/// stated it, in key order.
pub(crate) fn idmap_dict(ids: &CoreIdMap) -> BTreeMap<String, String> {
    ids.iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// The identifiers an instrument is stated under, one code under each
/// source - `ISIN`, `CUSIP`, `FIGI` - in source order.
pub(crate) fn securityids_dict(ids: &CoreSecurityIds) -> BTreeMap<String, String> {
    ids.iter()
        .map(|id| (id.sectype().as_str().to_owned(), id.code().to_owned()))
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
    /// The column `name` is - folded, as the column enums read it - or a
    /// `ValueError` naming the unknown fact.
    fn of_name(owner: &str, name: &str) -> PyResult<Self> {
        EventColumn::of_name(name)
            .map(Self::Event)
            .or_else(|| MarketColumn::of_name(name).map(Self::Market))
            .or_else(|| OperationColumn::of_name(name).map(Self::Operation))
            .ok_or_else(|| PyValueError::new_err(format!("{owner} states no fact {name:?}")))
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
    fn state<E: Event + Operation>(self, leaf: &mut E, value: &Scalar) -> PyResult<()> {
        let field = match self {
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
            Self::Event(column) => column.record(leaf, &checked),
            Self::Market(column) => column.record(leaf, &checked),
            Self::Operation(column) => column.record(leaf, &checked),
        }
        Ok(())
    }
}

/// An operation of kind `K` at `currunix` with every named fact in `facts`
/// stated through its column, not yet finalized. A value given as the
/// literal `Ellipsis` is skipped and `None` clears; `undated` refuses the
/// event facts an undated element does not state, naming the fact.
pub(crate) fn stated_operation<K: CoreOperationKind>(
    owner: &str,
    currunix: i64,
    facts: Option<&Bound<'_, PyDict>>,
    undated: bool,
) -> PyResult<OperationEvent<K>> {
    let mut leaf = OperationEvent::<K>::at(currunix);
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
        } else {
            from_py(&value)?
        };
        fact.state(&mut leaf, &scalar)?;
    }
    Ok(leaf)
}

/// Register every graph class and the module's constants.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<operation::PyLane>()?;
    module.add_class::<operation::PyBookRef>()?;
    module.add_class::<operation::PyOrder>()?;
    module.add_class::<operation::PyQuote>()?;
    module.add_class::<operation::PyExecution>()?;
    module.add_class::<operation::PyOrderEvent>()?;
    module.add_class::<operation::PyQuoteEvent>()?;
    module.add_class::<operation::PyExecutionEvent>()?;
    module.add_class::<trade::PyTradeEvent>()?;
    module.add_class::<book::PySnapshotPartition>()?;
    module.add_class::<book::PyBookSide>()?;
    module.add_class::<book::PyBookEvent>()?;
    module.add_class::<book::PySnapshotEvent>()?;
    module.add_class::<book::PyBookIterator>()?;
    module.add_class::<market_data::PyMarketData>()?;
    module.add_class::<market_data::PyMarketDataRowIterator>()?;
    module.add_class::<iterator::PyEventIterator>()?;
    // The symbol of the one consolidated book a global walk emits.
    module.add("GLOBAL_SYMBOL", yggdryl::graph::GLOBAL_SYMBOL)?;
    // The alternate-identifier keys a market-data entry's own identifiers
    // are held under.
    module.add("ENTRY_ID", yggdryl::graph::book::ENTRY_ID)?;
    module.add("ENTRY_REF_ID", yggdryl::graph::book::ENTRY_REF_ID)?;
    // The alternate-identifier keys an order's own identifiers may follow
    // across a lifecycle: a tuple, so no caller can change the module
    // constant for every importer.
    module.add(
        "FOLLOWED_ALTIDS",
        PyTuple::new(module.py(), yggdryl::graph::FOLLOWED_ALTIDS)?,
    )?;
    Ok(())
}
