//! Native Python view of the market's products: what the market did, read
//! out of what a venue said.
//!
//! Five values, their streams and the two readers beside them, mirroring
//! the message and its stream in [`crate::fix`]. A product is a frozen
//! snapshot of the core value: every fact the graph vocabulary answers
//! crosses exactly as [`PyMarketEventData`] crosses it - a `uuid` `Scalar`
//! for an identity, an `int` of nanoseconds for an instant, a decimal
//! `Scalar` for a number, a code `Scalar` for a currency, a side, a state
//! or an instrument identifier - and the product's own facts cross as the
//! plain values they are. A bare-noun reading the core's trait provides -
//! what the stated facts imply, read on every call - is a property; a
//! reading that takes an argument is a method. Nothing here reads a
//! protocol: the doors that read products out of messages are on
//! [`FixCodec`](crate::fix::PyFixCodec), the row doors here are the core's
//! `Product::into_row` and `Product::from_row`, and [`PyBookIterator`] is
//! the core's reader over any stream of products.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Mutex;

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBool;

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{
    Book, BookData, BookIterator, Execution, ExecutionData, Level, Order, OrderData, Party,
    Pricing, Product, Quote, QuoteData, Statement, Symbol, Trade, TradeData,
};
use yggdryl::{Decimal18, Scalar, Side};

use crate::fix::{Failed, Pulled, PyMarketEventData, code_scalar, decimal_scalar, named_rows};
use crate::types::field::{PyField, core_field_from_value};
use crate::types::scalar::{PyScalar, as_py, from_py};
use crate::value_error;

/// An optional number as the decimal `Scalar` it is, or `None`.
fn opt_decimal(held: Option<Decimal18>) -> Option<PyScalar> {
    held.map(decimal_scalar)
}

/// An optional code as the code `Scalar` its datatype is, or `None`.
fn opt_code<C>(held: Option<&C>) -> Option<PyScalar>
where
    C: Clone,
    Scalar: From<C>,
{
    held.map(code_scalar)
}

/// A day count since the epoch as the `datetime.date` it names, or `None`.
fn epoch_date(py: Python<'_>, days: Option<i32>) -> PyResult<Option<Py<PyAny>>> {
    days.map(|held| as_py(py, &Scalar::date32(held)))
        .transpose()
}

/// One lane as Python reads it: `(px, qty)`, both decimals.
fn lane_tuple(lane: Option<(Decimal18, Decimal18)>) -> Option<(PyScalar, PyScalar)> {
    lane.map(|(px, qty)| (decimal_scalar(px), decimal_scalar(qty)))
}

/// One level of a ladder as Python reads it: `(px, qty, count)`.
fn level_tuple(level: &Level) -> (PyScalar, PyScalar, u64) {
    (
        decimal_scalar(level.px),
        decimal_scalar(level.qty),
        level.count,
    )
}

/// One party as Python reads it: `(role, id, source)`.
fn party_tuple(party: &Party) -> (String, String, String) {
    (party.role.clone(), party.id.clone(), party.source.clone())
}

/// The side one spelling names, as `Side.read` reads it - a FIX code, the
/// specification's name or the stored value - refused as `ValueError`.
fn side_of(spelling: &str) -> PyResult<Side> {
    Side::read(spelling).map_err(value_error)
}

/// The depth a book is read to, refusing zero by name: a book is a ladder
/// to a declared depth, and no ladder is no book.
fn book_depth(depth: u32) -> PyResult<NonZeroU32> {
    NonZeroU32::new(depth).ok_or_else(|| {
        PyValueError::new_err(
            "depth: expected a depth of at least one level, got zero: a book is a ladder to a \
             declared depth, and no ladder is no book",
        )
    })
}

/// What an order prices, spelled as Python reads it.
const fn pricing_name(pricing: Pricing) -> &'static str {
    match pricing {
        Pricing::Market => "market",
        Pricing::Limit => "limit",
        Pricing::Stop => "stop",
        Pricing::StopLimit => "stoplimit",
    }
}

/// One product class: the frozen snapshot, the sixteen event facts every
/// product answers, the market facts its row publishes, the row doors and
/// the Python protocols, with the product's own facts appended.
macro_rules! product {
    (
        $(#[$meta:meta])*
        $py:ident, $name:literal, $core:ty;
        facts { $( $(#[$fdoc:meta])* $fact:ident : $ret:ty = $get:ident via $cross:path; )* }
        own { $($own:tt)* }
    ) => {
        $(#[$meta])*
        #[pyclass(name = $name, module = "yggdryl._native", frozen, skip_from_py_object)]
        #[derive(Clone)]
        pub(crate) struct $py {
            inner: $core,
        }

        impl $py {
            /// Wrap a product the core read.
            pub(crate) const fn from_inner(inner: $core) -> Self {
                Self { inner }
            }

            /// Borrow the product the core holds.
            pub(crate) const fn as_inner(&self) -> &$core {
                &self.inner
            }
        }

        #[pymethods]
        impl $py {
            /// The product's identity: the `uuid` its instant and its hash
            /// code derive.
            #[getter]
            fn curruuid(&self) -> PyScalar {
                crate::fix::uuid_scalar(self.inner.get_curruuid())
            }

            /// The identity every statement of one chain shares: derived
            /// from the cross code, and the product's own where it names
            /// none.
            #[getter]
            fn crossuuid(&self) -> PyScalar {
                crate::fix::uuid_scalar(self.inner.get_crossuuid())
            }

            /// The identifier every statement of one chain shares, as
            /// spelled; empty where none is named.
            #[getter]
            fn crosscode(&self) -> &str {
                self.inner.get_crosscode()
            }

            /// The code the facts digest to.
            #[getter]
            fn currhashcode(&self) -> u64 {
                self.inner.get_currhashcode()
            }

            /// The XXH3-64 of the cross code, zero where none is named.
            #[getter]
            fn crosshashcode(&self) -> u64 {
                self.inner.get_crosshashcode()
            }

            /// The names the product goes by, each under the scheme that
            /// issued it.
            #[getter]
            fn identifiers(&self) -> BTreeMap<String, String> {
                self.inner.get_identifiers().clone()
            }

            /// The identities of the statements this one descends from,
            /// oldest first.
            #[getter]
            fn parentuuids(&self) -> Vec<PyScalar> {
                self.inner
                    .get_parentuuids()
                    .iter()
                    .copied()
                    .map(crate::fix::uuid_scalar)
                    .collect()
            }

            /// The identities of the messages this product was read from.
            /// Provenance, never lineage: no walk moves it, and a product's
            /// table joins the message table on it.
            #[getter]
            fn srcuuids(&self) -> Vec<PyScalar> {
                self.inner
                    .get_srcuuids()
                    .iter()
                    .copied()
                    .map(crate::fix::uuid_scalar)
                    .collect()
            }

            /// When the product was stated: nanoseconds since the Unix
            /// epoch, UTC.
            #[getter]
            fn currunix(&self) -> i64 {
                self.inner.get_currunix()
            }

            /// The state the product is in, as the `state` code it is.
            #[getter]
            fn state(&self) -> PyScalar {
                code_scalar(self.inner.get_state())
            }

            /// The product's place in its chain: how many came before it.
            #[getter]
            fn seqnum(&self) -> u64 {
                self.inner.get_seqnum()
            }

            /// When the chain was created, or `None`.
            #[getter]
            fn creaunix(&self) -> Option<i64> {
                self.inner.get_creaunix()
            }

            /// When the product stops being good, or `None`.
            #[getter]
            fn expirunix(&self) -> Option<i64> {
                self.inner.get_expirunix()
            }

            /// When the statement this one follows happened, or `None`.
            #[getter]
            fn prevunix(&self) -> Option<i64> {
                self.inner.get_prevunix()
            }

            /// The identity of the statement this one follows, or `None`.
            #[getter]
            fn prevuuid(&self) -> Option<PyScalar> {
                self.inner.get_prevuuid().map(crate::fix::uuid_scalar)
            }

            /// The grid instant a walk read the product as the snapshot
            /// of, or `None`.
            #[getter]
            fn snapunix(&self) -> Option<i64> {
                self.inner.get_snapunix()
            }

            /// Whether the product can still be followed: its state can
            /// still change and it is not past its expiration - none
            /// stated, or one after its own instant. The one reading of
            /// liveness: what a walk keeps a chain live by, and what a
            /// ladder rests a maker on.
            #[getter]
            fn is_alive(&self) -> bool {
                self.inner.is_alive()
            }

            $(
                $(#[$fdoc])*
                #[getter]
                fn $fact(&self) -> $ret {
                    $cross(self.inner.$get())
                }
            )*

            /// The event this product is: every fact the graph vocabulary
            /// answers, held still, as `FixMsg.event()` answers a
            /// message's.
            fn event(&self) -> PyMarketEventData {
                PyMarketEventData::from_inner(self.inner.event().clone())
            }

            /// This product as one row of `field()`: the sixteen event
            /// columns, the market columns it publishes, then its own, each
            /// cell the raw value its column types and null where the
            /// product states no fact.
            #[allow(clippy::wrong_self_convention)]
            fn into_row(&self) -> PyResult<PyScalar> {
                self.inner
                    .into_row()
                    .map(PyScalar::from_inner)
                    .map_err(value_error)
            }

            /// The product one row of `field` states: the inverse of
            /// `into_row`.
            ///
            /// `field` is anything `Field` accepts - the one `field()`
            /// builds, or the one read off a batch - and `row` anything the
            /// `Scalar` boundary reads as it: a native `Scalar`, a mapping
            /// of names, a sequence in the field's order. The row is
            /// canonicalized under the field first, so a code spelled as
            /// text and a number spelled at another scale read as what the
            /// column types; a row that does not fit is a `ValueError`.
            #[staticmethod]
            fn from_row(field: &Bound<'_, PyAny>, row: &Bound<'_, PyAny>) -> PyResult<Self> {
                let field = core_field_from_value(field)?;
                let row = named_rows(&field, from_py(row)?);
                <$core as Product>::from_row(&field, &row)
                    .map(Self::from_inner)
                    .map_err(value_error)
            }

            fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
                let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
                    return py.NotImplemented();
                };
                PyBool::new(py, self.inner == other.inner)
                    .to_owned()
                    .into_any()
                    .unbind()
            }

            /// Hashes by the code the facts digest to, which equal facts
            /// share.
            fn __hash__(&self) -> isize {
                crate::python_hash(self.inner.get_currhashcode())
            }

            fn __repr__(&self) -> String {
                format!(
                    "{}({}, currunix={}, state={:?}, crosscode={:?})",
                    $name,
                    self.inner.get_curruuid(),
                    self.inner.get_currunix(),
                    self.inner.get_state().as_str(),
                    self.inner.get_crosscode()
                )
            }

            fn __copy__(&self) -> Self {
                self.clone()
            }

            fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
                self.clone()
            }

            $($own)*
        }
    };
}

/// One product stream: the core iterator behind a lock, pulled one item
/// at a time, each core value crossed by `$cross`, raising where the
/// Python iterable behind it failed.
macro_rules! stream {
    (
        $(#[$meta:meta])*
        $py:ident, $name:literal, $item:ty, $core:ty, $cross:expr
    ) => {
        $(#[$meta])*
        #[pyclass(name = $name, module = "yggdryl._native")]
        pub(crate) struct $py {
            /// The stream, behind the lock a class shared between threads
            /// needs; never contended, because a cursor is advanced by one
            /// caller.
            inner: Mutex<Box<dyn Iterator<Item = yggdryl::Result<$core>> + Send>>,
            /// Where the Python source behind `inner` failed.
            failed: Failed,
        }

        impl $py {
            /// A stream over a core door fed by a Python iterable.
            pub(crate) fn pulling<I>(inner: I, failed: Failed) -> Self
            where
                I: Iterator<Item = yggdryl::Result<$core>> + Send + 'static,
            {
                Self {
                    inner: Mutex::new(Box::new(inner)),
                    failed,
                }
            }
        }

        #[pymethods]
        impl $py {
            #[classattr]
            const __hash__: Option<Py<PyAny>> = None;
            fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                slf
            }
            fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<$item>> {
                let next = self
                    .inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .next();
                match next {
                    Some(held) => {
                        let held = held.map_err(value_error)?;
                        ($cross)(py, held).map(Some)
                    }
                    None => match self.failed.take() {
                        Some(error) => Err(error),
                        None => Ok(None),
                    },
                }
            }
        }
    };
}

product! {
    /// One order's life: a statement read out of its placement or a report
    /// against it, chained under the identifier the venue gave it.
    ///
    /// A frozen snapshot, comparing by every fact and hashing by the code
    /// the facts digest to. The row after the sixteen event columns: px,
    /// avgpx, qty, cumqty, leavesqty, side, currency, unit, tif, tradable,
    /// symbolticker, the six instrument codes, and the order's own stoppx
    /// and ordtype. What the stated facts imply - the pricing, what is
    /// left, what filled, whether it rests - is read on every call.
    PyOrder, "Order", OrderData;
    facts {
        /// The limit price, as a decimal; zero where none is stated.
        px: PyScalar = get_px via decimal_scalar;
        /// The average price filled so far, or `None`.
        avgpx: Option<PyScalar> = get_avgpx via opt_decimal;
        /// The quantity ordered, as a decimal; zero where none is stated.
        qty: PyScalar = get_qty via decimal_scalar;
        /// The quantity filled so far, or `None`.
        cumqty: Option<PyScalar> = get_cumqty via opt_decimal;
        /// The quantity still open, or `None`.
        leavesqty: Option<PyScalar> = get_leavesqty via opt_decimal;
        /// The side, as the `side` code it is; `UNKNOWN` where none is
        /// stated.
        side: PyScalar = get_side via code_scalar;
        /// The currency, as the `currency` code it is; `XXX` where none is
        /// stated.
        currency: PyScalar = get_currency via code_scalar;
        /// The unit the quantity is counted in; empty where none is stated.
        unit: &str = get_unit via std::convert::identity;
        /// The time in force, as the wire spells it, or `None`.
        tif: Option<&str> = get_tif via std::convert::identity;
        /// Whether the order can trade, or `None` where nothing says.
        tradable: Option<bool> = get_tradable via std::convert::identity;
        /// The instrument's ticker, or `None`.
        symbolticker: Option<&str> = get_symbolticker via std::convert::identity;
        /// The instrument's ISIN, or `None`.
        isincode: Option<PyScalar> = get_isincode via opt_code;
        /// The instrument's CUSIP, or `None`.
        cusipcode: Option<PyScalar> = get_cusipcode via opt_code;
        /// The instrument's SEDOL, or `None`.
        sedolcode: Option<PyScalar> = get_sedolcode via opt_code;
        /// The instrument's Bloomberg identifier, or `None`.
        bloombergcode: Option<PyScalar> = get_bloombergcode via opt_code;
        /// The instrument's CFI classification, or `None`.
        cficode: Option<PyScalar> = get_cficode via opt_code;
        /// The market the order names, as an ISO 10383 MIC, or `None`.
        miccode: Option<PyScalar> = get_miccode via opt_code;
        /// The stop price, as a decimal, or `None` for an order without
        /// one.
        stoppx: Option<PyScalar> = get_stoppx via opt_decimal;
        /// The type the order states, as the wire spells FIX's
        /// `OrdType(40)` - `1` market, `2` limit, `3` stop, `4` stop
        /// limit - or `None` where it states none.
        ordtype: Option<&str> = get_ordtype via std::convert::identity;
        /// What the order is worth at its limit, `px * qty` as a decimal,
        /// or `None` where either is zero.
        notional: Option<PyScalar> = notional via opt_decimal;
        /// What is left to trade, as a decimal: what the order states is
        /// left; else what it ordered less what filled, floored at zero;
        /// else what it ordered.
        remaining: PyScalar = remaining via decimal_scalar;
        /// What traded, as a decimal: what the order states filled; else
        /// what it ordered less what is left, floored at zero; else zero.
        filled: PyScalar = filled via decimal_scalar;
        /// How much of what was ordered traded, `filled / qty` at eighteen
        /// places, or `None` where nothing was ordered.
        filled_ratio: Option<PyScalar> = filled_ratio via opt_decimal;
        /// Whether the order rests on a ladder right now: alive, on a side
        /// that takes a lane, a limit order priced above zero, with
        /// something left. A market order, a stop until it triggers, a
        /// cross, and a done order rest nothing.
        is_resting: bool = is_resting via std::convert::identity;
    }
    own {
        /// What the order prices - `"market"`, `"limit"`, `"stop"` or
        /// `"stoplimit"` - read off the type it states, or `None` where it
        /// states a type outside the four; where it states none, what the
        /// limit and the stop imply.
        #[getter]
        fn pricing(&self) -> Option<&'static str> {
            self.inner.pricing().map(pricing_name)
        }

        /// The row an order publishes: the sixteen event columns, the
        /// market columns, stoppx and ordtype, under a non-null Struct
        /// named `order`.
        #[staticmethod]
        fn field() -> PyResult<PyField> {
            OrderData::field().map(PyField::from_inner).map_err(value_error)
        }
    }
}

product! {
    /// One fill: an execution report stating a quantity that traded, in the
    /// chain of the order it fills.
    ///
    /// A frozen snapshot, comparing by every fact and hashing by the code
    /// the facts digest to. The row after the sixteen event columns: px,
    /// qty, side, currency, unit, symbolticker and the six instrument codes.
    /// Its state is the order's status after the fill, which is what
    /// `is_partial` and `completes` read.
    PyExecution, "Execution", ExecutionData;
    facts {
        /// The price that traded, as a decimal; zero where none is stated.
        px: PyScalar = get_px via decimal_scalar;
        /// The quantity that traded, as a decimal; zero where none is
        /// stated.
        qty: PyScalar = get_qty via decimal_scalar;
        /// The side, as the `side` code it is; `UNKNOWN` where none is
        /// stated.
        side: PyScalar = get_side via code_scalar;
        /// The currency, as the `currency` code it is; `XXX` where none is
        /// stated.
        currency: PyScalar = get_currency via code_scalar;
        /// The unit the quantity is counted in; empty where none is stated.
        unit: &str = get_unit via std::convert::identity;
        /// The instrument's ticker, or `None`.
        symbolticker: Option<&str> = get_symbolticker via std::convert::identity;
        /// The instrument's ISIN, or `None`.
        isincode: Option<PyScalar> = get_isincode via opt_code;
        /// The instrument's CUSIP, or `None`.
        cusipcode: Option<PyScalar> = get_cusipcode via opt_code;
        /// The instrument's SEDOL, or `None`.
        sedolcode: Option<PyScalar> = get_sedolcode via opt_code;
        /// The instrument's Bloomberg identifier, or `None`.
        bloombergcode: Option<PyScalar> = get_bloombergcode via opt_code;
        /// The instrument's CFI classification, or `None`.
        cficode: Option<PyScalar> = get_cficode via opt_code;
        /// The market the fill names, as an ISO 10383 MIC, or `None`.
        miccode: Option<PyScalar> = get_miccode via opt_code;
        /// What the fill was worth, `px * qty` as a decimal, or `None`
        /// where either is zero.
        notional: Option<PyScalar> = notional via opt_decimal;
        /// Whether the fill left its order open: the order's status after
        /// it is still live.
        is_partial: bool = is_partial via std::convert::identity;
        /// Whether the fill completed its order: the order's status after
        /// it is done.
        completes: bool = completes via std::convert::identity;
    }
    own {
        /// The row an execution publishes: the sixteen event columns and
        /// the market columns, under a non-null Struct named `execution`.
        #[staticmethod]
        fn field() -> PyResult<PyField> {
            ExecutionData::field().map(PyField::from_inner).map_err(value_error)
        }
    }
}

product! {
    /// One settled transaction: a report stating a quantity that traded,
    /// with its dates and its parties, the two sides' reports of one match
    /// in one chain.
    ///
    /// A frozen snapshot, comparing by every fact and hashing by the code
    /// the facts digest to. The row after the sixteen event columns: px,
    /// qty, side, currency, unit, symbolticker, the six instrument codes,
    /// and the trade's own tradedate, settldate and parties.
    PyTrade, "Trade", TradeData;
    facts {
        /// The price that traded, as a decimal; zero where none is stated.
        px: PyScalar = get_px via decimal_scalar;
        /// The quantity that traded, as a decimal; zero where none is
        /// stated.
        qty: PyScalar = get_qty via decimal_scalar;
        /// The side, as the `side` code it is; `UNKNOWN` where none is
        /// stated.
        side: PyScalar = get_side via code_scalar;
        /// The currency, as the `currency` code it is; `XXX` where none is
        /// stated.
        currency: PyScalar = get_currency via code_scalar;
        /// The unit the quantity is counted in; empty where none is stated.
        unit: &str = get_unit via std::convert::identity;
        /// The instrument's ticker, or `None`.
        symbolticker: Option<&str> = get_symbolticker via std::convert::identity;
        /// The instrument's ISIN, or `None`.
        isincode: Option<PyScalar> = get_isincode via opt_code;
        /// The instrument's CUSIP, or `None`.
        cusipcode: Option<PyScalar> = get_cusipcode via opt_code;
        /// The instrument's SEDOL, or `None`.
        sedolcode: Option<PyScalar> = get_sedolcode via opt_code;
        /// The instrument's Bloomberg identifier, or `None`.
        bloombergcode: Option<PyScalar> = get_bloombergcode via opt_code;
        /// The instrument's CFI classification, or `None`.
        cficode: Option<PyScalar> = get_cficode via opt_code;
        /// The market the trade names, as an ISO 10383 MIC, or `None`.
        miccode: Option<PyScalar> = get_miccode via opt_code;
        /// What the trade was worth, `px * qty` as a decimal, or `None`
        /// where either is zero.
        notional: Option<PyScalar> = notional via opt_decimal;
        /// How many days after the trade date it settles, where both are
        /// stated - `T+2` answers `2` - or `None`.
        settlement_days: Option<i32> = settlement_days via std::convert::identity;
    }
    own {
        /// The day the trade was done, as a `datetime.date`, or `None`.
        #[getter]
        fn tradedate(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
            epoch_date(py, self.inner.get_tradedate())
        }

        /// The day the trade settles, as a `datetime.date`, or `None`.
        #[getter]
        fn settldate(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
            epoch_date(py, self.inner.get_settldate())
        }

        /// The parties to the trade, each as `(role, id, source)` in the
        /// order the report states them; empty where it states none.
        #[getter]
        fn parties(&self) -> Vec<(String, String, String)> {
            self.inner.get_parties().iter().map(party_tuple).collect()
        }

        /// The first party whose role is `role`, exactly as the report
        /// spells it, as `(role, id, source)`; `None` where none is.
        fn party_by_role(&self, role: &str) -> Option<(String, String, String)> {
            self.inner.party_by_role(role).map(party_tuple)
        }

        /// The row a trade publishes: the sixteen event columns, the
        /// market columns, tradedate, settldate and parties, under a
        /// non-null Struct named `trade`.
        #[staticmethod]
        fn field() -> PyResult<PyField> {
            TradeData::field().map(PyField::from_inner).map_err(value_error)
        }
    }
}

product! {
    /// A price stated at an instant: a quote, its updates and the cancel
    /// that ends it, in one chain under its identifier.
    ///
    /// A frozen snapshot, comparing by every fact and hashing by the code
    /// the facts digest to. The row after the sixteen event columns: the
    /// bid lane's bidpx, bidqty, bidcurrency and bidunit, the ask lane's
    /// askpx, askqty, askcurrency and askunit, symbolticker and the six
    /// instrument codes. A lane is quoted where both its price and its
    /// size are stated above zero - a zero is a withdrawn side, FIX's own
    /// rule - and `bid`, `ask`, `mid` and `spread` read the lanes so.
    PyQuote, "Quote", QuoteData;
    facts {
        /// The bid lane's price, or `None`.
        bidpx: Option<PyScalar> = get_bidpx via opt_decimal;
        /// The bid lane's size, or `None`.
        bidqty: Option<PyScalar> = get_bidqty via opt_decimal;
        /// The currency the bid lane is quoted in, or `None`.
        bidcurrency: Option<PyScalar> = get_bidcurrency via opt_code;
        /// The unit the bid lane's size is counted in, or `None`.
        bidunit: Option<&str> = get_bidunit via std::convert::identity;
        /// The ask lane's price, or `None`.
        askpx: Option<PyScalar> = get_askpx via opt_decimal;
        /// The ask lane's size, or `None`.
        askqty: Option<PyScalar> = get_askqty via opt_decimal;
        /// The currency the ask lane is quoted in, or `None`.
        askcurrency: Option<PyScalar> = get_askcurrency via opt_code;
        /// The unit the ask lane's size is counted in, or `None`.
        askunit: Option<&str> = get_askunit via std::convert::identity;
        /// The instrument's ticker, or `None`.
        symbolticker: Option<&str> = get_symbolticker via std::convert::identity;
        /// The instrument's ISIN, or `None`.
        isincode: Option<PyScalar> = get_isincode via opt_code;
        /// The instrument's CUSIP, or `None`.
        cusipcode: Option<PyScalar> = get_cusipcode via opt_code;
        /// The instrument's SEDOL, or `None`.
        sedolcode: Option<PyScalar> = get_sedolcode via opt_code;
        /// The instrument's Bloomberg identifier, or `None`.
        bloombergcode: Option<PyScalar> = get_bloombergcode via opt_code;
        /// The instrument's CFI classification, or `None`.
        cficode: Option<PyScalar> = get_cficode via opt_code;
        /// The market the quote names, as an ISO 10383 MIC, or `None`.
        miccode: Option<PyScalar> = get_miccode via opt_code;
        /// The bid lane where it is quoted, as `(px, qty)`: what the
        /// quoter would pay, and for how much; `None` where a price or a
        /// size is missing or zero.
        bid: Option<(PyScalar, PyScalar)> = bid via lane_tuple;
        /// The ask lane where it is quoted, as `(px, qty)`: what the
        /// quoter would be paid, and for how much; `None` likewise.
        ask: Option<(PyScalar, PyScalar)> = ask via lane_tuple;
        /// Whether both lanes are quoted.
        is_two_sided: bool = is_two_sided via std::convert::identity;
        /// The middle of the two lanes, `(bidpx + askpx) / 2` as a
        /// decimal, or `None` on a one-sided or empty quote.
        mid: Option<PyScalar> = mid via opt_decimal;
        /// The distance between the two lanes, `askpx - bidpx` as a
        /// decimal - zero where they lock, negative where they cross - or
        /// `None` on a one-sided or empty quote.
        spread: Option<PyScalar> = spread via opt_decimal;
    }
    own {
        /// The lane `side` takes, as `(px, qty)`: the bid for a side that
        /// pays, the ask for one that is paid, `None` for a side that
        /// takes no lane or a lane not quoted. `side` is any spelling
        /// `Side.read` reads - `"Buy"`, `"1"`, `"BUY"` - and one it does
        /// not is a `ValueError`.
        fn lane(&self, side: &str) -> PyResult<Option<(PyScalar, PyScalar)>> {
            Ok(lane_tuple(self.inner.lane(&side_of(side)?)))
        }

        /// The row a quote publishes: the sixteen event columns and the
        /// two lanes' columns, under a non-null Struct named `quote`.
        #[staticmethod]
        fn field() -> PyResult<PyField> {
            QuoteData::field().map(PyField::from_inner).map_err(value_error)
        }
    }
}

product! {
    /// The ladder for one instrument at one instant, to a declared depth:
    /// every order and quote of the instrument live in the stream, summed
    /// and counted per price, what printed against it, the books of one
    /// symbol chained flat.
    ///
    /// A frozen snapshot, comparing by every fact and hashing by the code
    /// the facts digest to. A book is a market event whose `px` is the mid
    /// - zero where there is none - `qty` the size resting on both
    /// ladders, `bidpx`, `bidqty`, `askpx` and `askqty` the tops,
    /// `lastpx`, `lastqty`, `avgpx` and `cumqty` the prints since the
    /// chain began, and `tradable` the makers' stated fact. The row after
    /// the sixteen event columns: lastpx, lastqty, avgpx, cumqty,
    /// tradable, currency, unit, symbolticker, the six instrument codes,
    /// then the book's own bids and asks, each a fixed-size list of
    /// `depth` levels, and updates. Every other reading is the ladders',
    /// re-derived on every call, and a missing top is `None` on every
    /// reading that divides by it.
    PyBook, "Book", BookData;
    facts {
        /// The mid, as a decimal; zero where the book is one-sided or
        /// empty.
        px: PyScalar = get_px via decimal_scalar;
        /// The size resting on both ladders, summed, as a decimal.
        qty: PyScalar = get_qty via decimal_scalar;
        /// The best bid's price, or `None`.
        bidpx: Option<PyScalar> = get_bidpx via opt_decimal;
        /// The best bid's size, or `None`.
        bidqty: Option<PyScalar> = get_bidqty via opt_decimal;
        /// The best ask's price, or `None`.
        askpx: Option<PyScalar> = get_askpx via opt_decimal;
        /// The best ask's size, or `None`.
        askqty: Option<PyScalar> = get_askqty via opt_decimal;
        /// The price of the last print, or `None` where nothing printed.
        lastpx: Option<PyScalar> = get_lastpx via opt_decimal;
        /// The size of the last print, or `None`.
        lastqty: Option<PyScalar> = get_lastqty via opt_decimal;
        /// The average price of what printed since the chain began, or
        /// `None`.
        avgpx: Option<PyScalar> = get_avgpx via opt_decimal;
        /// What printed since the chain began, summed, or `None`.
        cumqty: Option<PyScalar> = get_cumqty via opt_decimal;
        /// Whether the instrument can trade, as the makers stated it, or
        /// `None` where nothing says or they disagree.
        tradable: Option<bool> = get_tradable via std::convert::identity;
        /// The currency, as the `currency` code it is; `XXX` where none is
        /// stated or the makers disagree.
        currency: PyScalar = get_currency via code_scalar;
        /// The unit the sizes are counted in; empty where none is stated.
        unit: &str = get_unit via std::convert::identity;
        /// The instrument's ticker, or `None`.
        symbolticker: Option<&str> = get_symbolticker via std::convert::identity;
        /// The instrument's ISIN, or `None`.
        isincode: Option<PyScalar> = get_isincode via opt_code;
        /// The instrument's CUSIP, or `None`.
        cusipcode: Option<PyScalar> = get_cusipcode via opt_code;
        /// The instrument's SEDOL, or `None`.
        sedolcode: Option<PyScalar> = get_sedolcode via opt_code;
        /// The instrument's Bloomberg identifier, or `None`.
        bloombergcode: Option<PyScalar> = get_bloombergcode via opt_code;
        /// The instrument's CFI classification, or `None`.
        cficode: Option<PyScalar> = get_cficode via opt_code;
        /// The market the book names, as an ISO 10383 MIC, or `None`.
        miccode: Option<PyScalar> = get_miccode via opt_code;
        /// How many statements have been applied to the book's chain since
        /// it began: every order, quote and print, one that moved nothing
        /// among them.
        updates: u64 = get_updates via std::convert::identity;
        /// The middle of the two tops, `(bidpx + askpx) / 2` as a decimal,
        /// or `None` on a one-sided or empty book.
        mid: Option<PyScalar> = mid via opt_decimal;
        /// The distance between the two tops, `askpx - bidpx` as a decimal
        /// - zero where they lock, negative where they cross - or `None`
        /// on a one-sided or empty book.
        spread: Option<PyScalar> = spread via opt_decimal;
        /// Whether both ladders rest something.
        is_two_sided: bool = is_two_sided via std::convert::identity;
        /// Whether the two tops are at one price: a spread of zero.
        is_locked: bool = is_locked via std::convert::identity;
        /// Whether the best bid is above the best ask: a negative spread.
        is_crossed: bool = is_crossed via std::convert::identity;
        /// The spread as a share of the mid, `spread * 10000 / mid` as a
        /// decimal, or `None` where there is no mid or it is zero.
        spread_bps: Option<PyScalar> = spread_bps via opt_decimal;
        /// The size-weighted mid, `(b * Qa + a * Qb) / (Qb + Qa)` over the
        /// tops, as a decimal, or `None` where either top is missing.
        microprice: Option<PyScalar> = microprice via opt_decimal;
        /// The imbalance at the tops, `(Qb - Qa) / (Qb + Qa)` in `[-1, 1]`,
        /// a missing top counting as no size, or `None` where both are
        /// missing.
        imbalance: Option<PyScalar> = imbalance via opt_decimal;
        /// The size resting on the bid ladder, summed, as a decimal.
        bid_size: PyScalar = bid_size via decimal_scalar;
        /// The size resting on the ask ladder, summed, as a decimal.
        ask_size: PyScalar = ask_size via decimal_scalar;
        /// The orders and quote lanes resting on the bid ladder, summed.
        bid_count: u64 = bid_count via std::convert::identity;
        /// The orders and quote lanes resting on the ask ladder, summed.
        ask_count: u64 = ask_count via std::convert::identity;
    }
    own {
        /// How many levels a side holds at most: the depth declared when
        /// the book was read.
        #[getter]
        fn depth(&self) -> u32 {
            self.inner.get_depth().get()
        }

        /// The bid side, best first, each level as `(px, qty, count)`: the
        /// price, the size resting there and how many orders and quote
        /// lanes rest there.
        #[getter]
        fn bids(&self) -> Vec<(PyScalar, PyScalar, u64)> {
            self.inner.get_bids().iter().map(level_tuple).collect()
        }

        /// The ask side, best first, each level as `(px, qty, count)`.
        #[getter]
        fn asks(&self) -> Vec<(PyScalar, PyScalar, u64)> {
            self.inner.get_asks().iter().map(level_tuple).collect()
        }

        /// The top of the bid ladder as `(px, qty, count)`, or `None`.
        #[getter]
        fn best_bid(&self) -> Option<(PyScalar, PyScalar, u64)> {
            self.inner.best_bid().map(level_tuple)
        }

        /// The top of the ask ladder as `(px, qty, count)`, or `None`.
        #[getter]
        fn best_ask(&self) -> Option<(PyScalar, PyScalar, u64)> {
            self.inner.best_ask().map(level_tuple)
        }

        /// The level at `index` of the ladder `side` takes, the top at
        /// zero, as `(px, qty, count)`; `None` past the ladder or for a
        /// side that takes no lane. `side` is any spelling `Side.read`
        /// reads, and one it does not is a `ValueError`.
        fn level(&self, side: &str, index: usize) -> PyResult<Option<(PyScalar, PyScalar, u64)>> {
            Ok(self.inner.level(&side_of(side)?, index).map(level_tuple))
        }

        /// The imbalance over the sizes summed across the first `levels`
        /// of each ladder, a shorter ladder contributing what it has, as
        /// a decimal; `None` where both sums are zero, which no levels
        /// always is.
        fn imbalance_to_depth(&self, levels: usize) -> Option<PyScalar> {
            opt_decimal(self.inner.imbalance_to_depth(levels))
        }

        /// The row a book of `depth` levels publishes: the sixteen event
        /// columns, the market columns, then bids and asks as fixed-size
        /// lists of `depth` levels and updates, under a non-null Struct
        /// named `book`. A depth of zero is a `ValueError` naming `depth`.
        #[staticmethod]
        fn field(depth: u32) -> PyResult<PyField> {
            BookData::field(book_depth(depth)?)
                .map(PyField::from_inner)
                .map_err(value_error)
        }
    }
}

/// The product one statement is, as the Python class of its arm.
fn statement_object(py: Python<'_>, statement: Statement) -> PyResult<Py<PyAny>> {
    Ok(match statement {
        Statement::Order(held) => Py::new(py, PyOrder::from_inner(held))?.into_any(),
        Statement::Quote(held) => Py::new(py, PyQuote::from_inner(held))?.into_any(),
        Statement::Execution(held) => Py::new(py, PyExecution::from_inner(held))?.into_any(),
        Statement::Trade(held) => Py::new(py, PyTrade::from_inner(held))?.into_any(),
    })
}

/// The core statement one product is, by its class; anything else is a
/// `TypeError` naming what it was.
fn statement_of(item: &Bound<'_, PyAny>) -> PyResult<Statement> {
    if let Ok(held) = item.extract::<PyRef<'_, PyOrder>>() {
        return Ok(Statement::Order(held.inner.clone()));
    }
    if let Ok(held) = item.extract::<PyRef<'_, PyQuote>>() {
        return Ok(Statement::Quote(held.inner.clone()));
    }
    if let Ok(held) = item.extract::<PyRef<'_, PyExecution>>() {
        return Ok(Statement::Execution(held.inner.clone()));
    }
    if let Ok(held) = item.extract::<PyRef<'_, PyTrade>>() {
        return Ok(Statement::Trade(held.inner.clone()));
    }
    Err(PyTypeError::new_err(format!(
        "a statement must be an Order, a Quote, an Execution or a Trade, not {}",
        item.get_type().name()?
    )))
}

/// One product crossed as its own class: what a one-product stream does
/// with what the core answers.
fn own_class<C, P>(cross: fn(C) -> P) -> impl Fn(Python<'_>, C) -> PyResult<P> {
    move |_py, held| Ok(cross(held))
}

stream! {
    /// A stream of orders, one at a time: what `FixCodec.orders` answers.
    ///
    /// Nothing is collected: the core walk is the stream, and the Python
    /// iterable of messages behind it is pulled one item at a time. A
    /// message the walk refuses raises `ValueError` where it is met and the
    /// stream goes on past it; an item that is not a `FixMsg`, or a failure
    /// of the iterable itself, raises as itself and ends it.
    PyOrders, "Orders", PyOrder, OrderData, own_class(PyOrder::from_inner)
}

stream! {
    /// A stream of executions, one at a time: what `FixCodec.executions`
    /// answers, pulled and raising as `Orders` does.
    PyExecutions, "Executions", PyExecution, ExecutionData, own_class(PyExecution::from_inner)
}

stream! {
    /// A stream of trades, one at a time: what `FixCodec.trades` answers,
    /// pulled and raising as `Orders` does.
    PyTrades, "Trades", PyTrade, TradeData, own_class(PyTrade::from_inner)
}

stream! {
    /// A stream of quotes, one at a time: what `FixCodec.quotes` answers,
    /// pulled and raising as `Orders` does.
    PyQuotes, "Quotes", PyQuote, QuoteData, own_class(PyQuote::from_inner)
}

stream! {
    /// A stream of books, one at a time: what `FixCodec.books` answers,
    /// pulled and raising as `Orders` does.
    PyBooks, "Books", PyBook, BookData, own_class(PyBook::from_inner)
}

stream! {
    /// A mixed stream of statements, one at a time: what
    /// `FixCodec.statements` answers, each item an `Order`, a `Quote` or an
    /// `Execution` by what the message stated, in instant order, pulled
    /// and raising as `Orders` does.
    PyStatements, "Statements", Py<PyAny>, Statement, statement_object
}

/// The instrument a statement is about, as the one text every book of it
/// is keyed by.
///
/// `Symbol(text)` is the caller's own spelling, trimmed, blank text being
/// `Symbol.GLOBAL`, the symbol of no instrument; `Symbol.of(product)` is
/// the rule a statement is keyed by - its ISIN, else its ticker, else its
/// CUSIP, its SEDOL, its Bloomberg identifier, else the global symbol.
/// Immutable: compares and orders by its text, hashes stably, copies and
/// pickles as itself.
#[pyclass(
    name = "Symbol",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySymbol {
    inner: Symbol,
}

#[pymethods]
impl PySymbol {
    /// A symbol spelled by the caller, trimmed; blank text is the global
    /// symbol.
    #[new]
    fn new(text: &str) -> Self {
        Self {
            inner: Symbol::new(text),
        }
    }

    /// The symbol of no instrument, spelled `GLOBAL`: what a statement
    /// naming none keys, and the one symbol a global book is read under.
    #[classattr]
    #[allow(non_snake_case)]
    fn GLOBAL() -> Self {
        Self {
            inner: Symbol::GLOBAL,
        }
    }

    /// The symbol a product names: its ISIN, else the ticker it is known
    /// by, else its CUSIP, else its SEDOL, else its Bloomberg identifier,
    /// else `Symbol.GLOBAL`; a blank ticker names nothing. `product` is an
    /// `Order`, a `Quote`, an `Execution`, a `Trade` or a `Book`, and
    /// anything else is a `TypeError`.
    #[staticmethod]
    fn of(product: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner = if let Ok(held) = product.extract::<PyRef<'_, PyOrder>>() {
            Symbol::of(held.as_inner())
        } else if let Ok(held) = product.extract::<PyRef<'_, PyQuote>>() {
            Symbol::of(held.as_inner())
        } else if let Ok(held) = product.extract::<PyRef<'_, PyExecution>>() {
            Symbol::of(held.as_inner())
        } else if let Ok(held) = product.extract::<PyRef<'_, PyTrade>>() {
            Symbol::of(held.as_inner())
        } else if let Ok(held) = product.extract::<PyRef<'_, PyBook>>() {
            Symbol::of(held.as_inner())
        } else {
            return Err(PyTypeError::new_err(format!(
                "Symbol.of takes an Order, a Quote, an Execution, a Trade or a Book, not {}",
                product.get_type().name()?
            )));
        };
        Ok(Self { inner })
    }

    /// The text the symbol is.
    #[getter]
    fn text(&self) -> &str {
        self.inner.as_str()
    }

    /// Whether this is the symbol of no instrument.
    #[getter]
    fn is_global(&self) -> bool {
        self.inner.is_global()
    }

    fn __str__(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("Symbol({:?})", self.inner.as_str())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    /// Hashes by the XXH3-64 of the text, which equal symbols share.
    fn __hash__(&self) -> isize {
        crate::python_hash(yggdryl::xxhash::xxh3(self.inner.as_str().as_bytes()))
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        let class = py.get_type::<Self>().into_any().unbind();
        (class, (self.inner.as_str().to_owned(),))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One book per symbol per instant, read out of any stream of products:
/// the core's `BookIterator` over a Python iterable of `Order`, `Quote`,
/// `Execution` and `Trade` values.
///
/// The statements are pulled one item at a time and never collected, and
/// they must arrive in instant order - what a `FixCodec` door answers and
/// what a table sorted by `currunix` reads back as - because a ladder
/// cannot be rewound: a statement before the open instant is refused as
/// `ValueError` naming `currunix` where it is met, moves nothing, and the
/// stream goes on. The orders and quotes rest, the executions and trades
/// print - a stream carrying both the execution and the trade of one fill
/// counts it twice, so a caller feeds one of the two - and the book of
/// every symbol an instant touched is read once the stream moves past
/// it, to `depth` levels a side, in symbol order. `snapshot_ns` of zero
/// reads one book per instant; a positive step is a grid, the book of a
/// step its closing state, dated at the last instant that moved the
/// symbol and stamped with the step. `symbol` keys every statement under
/// one symbol instead of the one it names - `Symbol.GLOBAL` is the global
/// book of the whole stream. A depth of zero is a `ValueError` naming
/// `depth` and a negative step one naming `snapshot_ns`, before an item
/// is pulled; an item that is not a product raises `TypeError` as itself
/// and ends the stream, after the books of the instant already open; a
/// failure of the iterable raises as itself, likewise.
#[pyclass(name = "BookIterator", module = "yggdryl._native")]
pub(crate) struct PyBookIterator {
    /// The reader, behind the lock a class shared between threads needs;
    /// never contended, because a cursor is advanced by one caller.
    inner: Mutex<Box<dyn Iterator<Item = yggdryl::Result<BookData>> + Send>>,
    /// Where the Python iterable behind `inner` failed.
    failed: Failed,
    depth: NonZeroU32,
    snapshot_ns: i64,
    symbol: Option<PySymbol>,
}

#[pymethods]
impl PyBookIterator {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    #[new]
    #[pyo3(signature = (statements, depth, *, snapshot_ns = 0, symbol = None))]
    fn new(
        statements: &Bound<'_, PyAny>,
        depth: u32,
        snapshot_ns: i64,
        symbol: Option<PyRef<'_, PySymbol>>,
    ) -> PyResult<Self> {
        let depth = book_depth(depth)?;
        if snapshot_ns < 0 {
            return Err(PyValueError::new_err(format!(
                "snapshot_ns: expected a grid step in nanoseconds, or zero for one book per \
                 instant, got {snapshot_ns}"
            )));
        }
        let symbol = symbol.map(|held| held.clone());
        let pulled = Pulled::new(statements, statement_of)?;
        let failed = pulled.failed.clone();
        let mut books =
            BookIterator::new(pulled.map(Ok), depth, true).with_snapshot_ns(snapshot_ns);
        if let Some(symbol) = &symbol {
            books = books.with_symbol(symbol.inner.clone());
        }
        Ok(Self {
            inner: Mutex::new(Box::new(books)),
            failed,
            depth,
            snapshot_ns,
            symbol,
        })
    }

    /// How many levels a side every book is read to.
    #[getter]
    fn depth(&self) -> u32 {
        self.depth.get()
    }

    /// The grid step in nanoseconds, or zero for one book per instant.
    #[getter]
    fn snapshot_ns(&self) -> i64 {
        self.snapshot_ns
    }

    /// The one symbol every statement is keyed under, or `None` where each
    /// is keyed under the one it names.
    #[getter]
    fn symbol(&self) -> Option<PySymbol> {
        self.symbol.clone()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<PyBook>> {
        let next = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next();
        match next {
            Some(held) => held.map(PyBook::from_inner).map(Some).map_err(value_error),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
