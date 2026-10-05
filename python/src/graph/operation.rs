//! Native Python view of the operation leaves - [`CoreOrder`],
//! [`CoreQuote`] and [`CoreExecution`] undated, [`CoreOrderEvent`],
//! [`CoreQuoteEvent`] and [`CoreExecutionEvent`] dated - and of the book
//! control a market-data entry carries, [`CoreBookRef`].

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict};

use yggdryl::graph::{
    BookRef as CoreBookRef, Element, Execution as CoreExecution,
    ExecutionEvent as CoreExecutionEvent, ExecutionKind, MdUpdateAction as CoreMdUpdateAction,
    Order as CoreOrder, OrderEvent as CoreOrderEvent, OrderKind, Quote as CoreQuote,
    QuoteEvent as CoreQuoteEvent, QuoteKind,
};
use yggdryl::{DataType, Decimal};

use super::{decimal_scalar, ellipsis, slot_repr, stated_operation};
use crate::scalar::{PyScalar, from_py};
use crate::value_error;

/// One undated operation class: `Order`, `Quote` or `Execution` over the
/// core alias `$core` of kind `$kind`, dated into `$event`.
macro_rules! operation_element_class {
    ($class:ident, $name:literal, $core:ty, $kind:ty, $event:ident) => {
        #[doc = concat!(
            "An undated ", $name, ": the element, market and operation facts ",
            "of one operation with no instant. Immutable: every verb answers ",
            "a new value."
        )]
        #[pyclass(name = $name, module = "yggdryl._native", frozen, skip_from_py_object)]
        #[derive(Clone)]
        pub(crate) struct $class {
            pub(crate) inner: $core,
        }

        impl $class {
            /// Wrap a value the core built.
            pub(crate) const fn from_core(inner: $core) -> Self {
                Self { inner }
            }
        }

        graph_methods!($class, $name; [
            element_getters, market_getters, operation_getters, kind_getters, common_verbs,
            element_repr
        ]; {
            /// Build the element from its named facts, keyed by column name
            /// - the market and operation columns and the element's own
            /// `crosscode` and `srcuuids` - each checked by its column's
            /// field and stated through its column, then finalized. A fact
            /// given as `...` is skipped; `None` clears. A derived identity
            /// or any other event column is refused by name.
            #[new]
            #[pyo3(signature = (**facts))]
            fn new(facts: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
                let mut element = stated_operation::<$kind>($name, 0, facts, true)?.into_element();
                element.finalize();
                Ok(Self::from_core(element))
            }

            /// Which operation this is: `"order"`, `"quote"` or
            /// `"execution"`.
            #[getter]
            fn kind(&self) -> &'static str {
                self.inner.kind().as_str()
            }

            /// This element dated at `unix` nanoseconds since the epoch, and
            /// finalized.
            fn at(&self, unix: i64) -> $event {
                $event::from_core(self.inner.clone().at(unix))
            }
        });
    };
}

/// One dated operation class: `OrderEvent`, `QuoteEvent` or
/// `ExecutionEvent` over the core alias `$core` of kind `$kind`, undated
/// into `$element`.
macro_rules! operation_event_class {
    ($class:ident, $name:literal, $core:ty, $kind:ty, $element:ident) => {
        #[doc = concat!(
            "A dated ", $name, ": one operation at one instant, with the ",
            "book-control facts of a market-data entry where it is one. ",
            "Immutable: every verb answers a new value."
        )]
        #[pyclass(name = $name, module = "yggdryl._native", frozen, skip_from_py_object)]
        #[derive(Clone)]
        pub(crate) struct $class {
            pub(crate) inner: $core,
        }

        impl $class {
            /// Wrap a value the core built.
            pub(crate) const fn from_core(inner: $core) -> Self {
                Self { inner }
            }
        }

        graph_methods!($class, $name; [
            element_getters, event_getters, market_getters, operation_getters,
            kind_getters, common_verbs, event_verbs
        ]; {
            /// Build the event at `currunix` nanoseconds since the epoch from
            /// its named facts, keyed by column name - the event, market and
            /// operation columns - each checked by its column's field and
            /// stated through its column, with `book`'s control facts, then
            /// finalized. A fact given as `...` is skipped; `None` clears. A
            /// derived identity, or `currunix` again, is refused by name.
            #[new]
            #[pyo3(signature = (currunix, book=ellipsis(), **facts))]
            fn new(
                py: Python<'_>,
                currunix: i64,
                book: Py<PyAny>,
                facts: Option<&Bound<'_, PyDict>>,
            ) -> PyResult<Self> {
                let mut event = stated_operation::<$kind>($name, currunix, facts, false)?;
                let book = book.bind(py);
                if !book.is(py.Ellipsis()) && !book.is_none() {
                    let book = book.extract::<PyRef<'_, PyBookRef>>().map_err(|_| {
                        PyTypeError::new_err(concat!("expected a BookRef for ", $name, ".book"))
                    })?;
                    event.set_book(Some(book.inner.clone()));
                }
                event.finalize();
                Ok(Self::from_core(event))
            }

            /// Which operation this is: `"order"`, `"quote"` or
            /// `"execution"`.
            #[getter]
            fn kind(&self) -> &'static str {
                self.inner.kind().as_str()
            }

            /// The book-control facts, where this is a market-data entry.
            #[getter]
            fn book(&self) -> Option<PyBookRef> {
                self.inner.book().cloned().map(PyBookRef::from_core)
            }

            /// The update action the entry states, where it states one.
            #[getter]
            fn action(&self) -> Option<&'static str> {
                self.inner.action().map(CoreMdUpdateAction::as_str)
            }

            /// The book scope the entry states, empty where none.
            #[getter]
            fn scope(&self) -> &str {
                self.inner.scope()
            }

            /// Whether this is part of a FIX full-snapshot replacement.
            #[getter]
            fn is_full_snapshot(&self) -> bool {
                self.inner.is_full_snapshot()
            }

            /// This event with `book`'s control facts, refinalized.
            fn with_book(&self, book: PyRef<'_, PyBookRef>) -> Self {
                let mut event = self.inner.clone().with_book(book.inner.clone());
                event.finalize();
                Self::from_core(event)
            }

            /// This event without its clocks and book control, finalized
            /// as the undated element it then is.
            #[allow(clippy::wrong_self_convention)] // The core's own name, over `&self`: frozen, never moved.
            fn into_element(&self) -> $element {
                let mut element = self.inner.clone().into_element();
                element.finalize();
                $element::from_core(element)
            }
        });
    };
}

operation_element_class!(PyOrder, "Order", CoreOrder, OrderKind, PyOrderEvent);
operation_element_class!(PyQuote, "Quote", CoreQuote, QuoteKind, PyQuoteEvent);
operation_element_class!(
    PyExecution,
    "Execution",
    CoreExecution,
    ExecutionKind,
    PyExecutionEvent
);
operation_event_class!(
    PyOrderEvent,
    "OrderEvent",
    CoreOrderEvent,
    OrderKind,
    PyOrder
);
operation_event_class!(
    PyQuoteEvent,
    "QuoteEvent",
    CoreQuoteEvent,
    QuoteKind,
    PyQuote
);
operation_event_class!(
    PyExecutionEvent,
    "ExecutionEvent",
    CoreExecutionEvent,
    ExecutionKind,
    PyExecution
);

/// A value given at a `BookRef` keyword: `None` where it was skipped
/// (`Ellipsis`) or stated as Python `None`, else the bound value.
fn stated<'py>(py: Python<'py>, value: &Py<PyAny>) -> Option<Bound<'py, PyAny>> {
    let bound = value.bind(py).clone();
    if bound.is(py.Ellipsis()) || bound.is_none() {
        None
    } else {
        Some(bound)
    }
}

/// A stated slot read as a decimal, checked through the same
/// `DataType::scalar` door every other decimal slot in the graph binding
/// crosses, so a float, an out-of-range value or any other value this slot
/// refuses is refused the same way theirs is.
fn stated_decimal(py: Python<'_>, name: &str, value: &Py<PyAny>) -> PyResult<Option<Decimal>> {
    let Some(bound) = stated(py, value) else {
        return Ok(None);
    };
    let scalar = from_py(&bound)?;
    let checked = DataType::Decimal
        .nullable_field(name)
        .scalar(scalar)
        .map_err(value_error)?;
    Decimal::from_scalar(&checked)
        .map(Some)
        .ok_or_else(|| PyTypeError::new_err(format!("expected a decimal for {name}")))
}

/// The typed book-control facts a market-data entry carries: what a book
/// reads to place the operation.
#[pyclass(
    name = "BookRef",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyBookRef {
    pub(crate) inner: CoreBookRef,
}

impl PyBookRef {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreBookRef) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyBookRef {
    /// Build a book-control value from its five slots, each defaulting to
    /// `Ellipsis` (not given). `action` is read through
    /// [`CoreMdUpdateAction`]'s `FromStr`, refusing text that names no
    /// spelling.
    #[pyo3(signature = (
        action=ellipsis(),
        scope=ellipsis(),
        position=ellipsis(),
        entry_px=ellipsis(),
        entry_size=ellipsis()
    ))]
    #[new]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `Py`.
    fn new(
        py: Python<'_>,
        action: Py<PyAny>,
        scope: Py<PyAny>,
        position: Py<PyAny>,
        entry_px: Py<PyAny>,
        entry_size: Py<PyAny>,
    ) -> PyResult<Self> {
        let action = stated(py, &action)
            .map(|value| value.extract::<String>())
            .transpose()?
            .map(|text| text.parse::<CoreMdUpdateAction>().map_err(value_error))
            .transpose()?;
        let scope = stated(py, &scope)
            .map(|value| value.extract::<String>())
            .transpose()?
            .map(Into::into);
        let position = stated(py, &position)
            .map(|value| value.extract::<u32>())
            .transpose()?;
        let entry_px = stated_decimal(py, "entry_px", &entry_px)?;
        let entry_size = stated_decimal(py, "entry_size", &entry_size)?;
        Ok(Self::from_core(CoreBookRef {
            action,
            scope,
            position,
            entry_px,
            entry_size,
        }))
    }

    /// Rebuild a book-control value pickle carried.
    #[staticmethod]
    fn _from_pickle(
        action: Option<&str>,
        scope: Option<String>,
        position: Option<u32>,
        entry_px: Option<PyRef<'_, PyScalar>>,
        entry_size: Option<PyRef<'_, PyScalar>>,
    ) -> PyResult<Self> {
        let action = action
            .map(|text| text.parse::<CoreMdUpdateAction>().map_err(value_error))
            .transpose()?;
        let entry_px = entry_px
            .map(|value| {
                Decimal::from_scalar(&value.inner)
                    .ok_or_else(|| PyTypeError::new_err("expected a decimal for entry_px"))
            })
            .transpose()?;
        let entry_size = entry_size
            .map(|value| {
                Decimal::from_scalar(&value.inner)
                    .ok_or_else(|| PyTypeError::new_err("expected a decimal for entry_size"))
            })
            .transpose()?;
        Ok(Self::from_core(CoreBookRef {
            action,
            scope: scope.map(Into::into),
            position,
            entry_px,
            entry_size,
        }))
    }

    /// The update action, where this control states one.
    #[getter]
    fn action(&self) -> Option<&'static str> {
        self.inner.action.map(CoreMdUpdateAction::as_str)
    }

    /// The book scope this control belongs to, where stated.
    #[getter]
    fn scope(&self) -> Option<&str> {
        self.inner.scope.as_deref()
    }

    /// The entry's position in its level, where stated.
    #[getter]
    const fn position(&self) -> Option<u32> {
        self.inner.position
    }

    /// The price this control states, as a decimal; `None` where it states
    /// none.
    #[getter]
    fn entry_px(&self) -> Option<PyScalar> {
        self.inner.entry_px.map(decimal_scalar)
    }

    /// The size this control states, as a decimal; `None` where it states
    /// none.
    #[getter]
    fn entry_size(&self) -> Option<PyScalar> {
        self.inner.entry_size.map(decimal_scalar)
    }

    /// Whether any control fact is stated.
    fn is_stated(&self) -> bool {
        self.inner.is_stated()
    }

    /// Whether the stated action removes a range of positions.
    fn is_range_delete(&self) -> bool {
        self.inner
            .action
            .is_some_and(CoreMdUpdateAction::is_range_delete)
    }

    /// Whether the stated action is a partial update of a live entry.
    fn is_partial(&self) -> bool {
        self.inner
            .action
            .is_some_and(CoreMdUpdateAction::is_partial)
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

    /// Hashes over the control's own fields.
    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __repr__(&self) -> String {
        format!(
            "BookRef(action={}, scope={}, position={}, entry_px={}, entry_size={})",
            slot_repr(self.action()),
            slot_repr(self.inner.scope.as_deref()),
            self.inner
                .position
                .map_or_else(|| "None".to_owned(), |position| position.to_string()),
            slot_repr(self.inner.entry_px),
            slot_repr(self.inner.entry_size),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    #[allow(clippy::type_complexity)]
    fn __reduce__(
        &self,
        py: Python<'_>,
    ) -> PyResult<(
        Py<PyAny>,
        (
            Option<&'static str>,
            Option<String>,
            Option<u32>,
            Option<PyScalar>,
            Option<PyScalar>,
        ),
    )> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (
                self.action(),
                self.inner.scope.as_ref().map(ToString::to_string),
                self.inner.position,
                self.entry_px(),
                self.entry_size(),
            ),
        ))
    }
}
