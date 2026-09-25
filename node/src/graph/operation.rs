//! Native Node.js view of the operation leaves - [`CoreOrder`],
//! [`CoreQuote`] and [`CoreExecution`] undated, [`CoreOrderEvent`],
//! [`CoreQuoteEvent`] and [`CoreExecutionEvent`] dated - and of the two
//! typed values they carry, [`CoreLane`] and [`CoreBookRef`].

use napi::bindgen_prelude::{BigInt, Either, Null, Result};
use napi_derive::napi;
use yggdryl::graph::operation_column::{lane_datatype, lane_fact, lane_of};
use yggdryl::graph::{
    BookRef as CoreBookRef, Execution as CoreExecution, ExecutionEvent as CoreExecutionEvent,
    ExecutionKind, Lane as CoreLane, MdUpdateAction as CoreMdUpdateAction, Order as CoreOrder,
    OrderEvent as CoreOrderEvent, OrderKind, Quote as CoreQuote, QuoteEvent as CoreQuoteEvent,
    QuoteKind,
};
use yggdryl::{DataType, Decimal18, Field, Scalar};

use super::{decimal_text, instant_of, stated_operation};
use crate::napi_error;
use crate::text::codec::JsScalar;

/// An undated order: the element, market and operation facts of one order
/// with no instant. Immutable: every verb answers a new value.
#[napi(js_name = "Order")]
#[derive(Clone)]
pub struct JsOrder {
    pub(crate) inner: CoreOrder,
}

/// An undated quote: the element, market and operation facts of one quote
/// with no instant. Immutable: every verb answers a new value.
#[napi(js_name = "Quote")]
#[derive(Clone)]
pub struct JsQuote {
    pub(crate) inner: CoreQuote,
}

/// An undated execution: the element, market and operation facts of one
/// execution with no instant. Immutable: every verb answers a new value.
#[napi(js_name = "Execution")]
#[derive(Clone)]
pub struct JsExecution {
    pub(crate) inner: CoreExecution,
}

/// A dated order: one order at one instant, with the book-control facts of
/// a market-data entry where it is one. Immutable: every verb answers a new
/// value.
#[napi(js_name = "OrderEvent")]
#[derive(Clone)]
pub struct JsOrderEvent {
    pub(crate) inner: CoreOrderEvent,
}

/// A dated quote: one quote at one instant, with the book-control facts of
/// a market-data entry where it is one. Immutable: every verb answers a new
/// value.
#[napi(js_name = "QuoteEvent")]
#[derive(Clone)]
pub struct JsQuoteEvent {
    pub(crate) inner: CoreQuoteEvent,
}

/// A dated execution: one execution at one instant, with the book-control
/// facts of a market-data entry where it is one. Immutable: every verb
/// answers a new value.
#[napi(js_name = "ExecutionEvent")]
#[derive(Clone)]
pub struct JsExecutionEvent {
    pub(crate) inner: CoreExecutionEvent,
}

/// One undated operation class over the core alias `$core` of kind `$kind`,
/// dated into `$event`: its constructor, `kind`, `at` and the shared
/// segments.
macro_rules! operation_element_class {
    ($class:ident, $name:literal, $core:ty, $kind:ty, $event:ident) => {
        impl $class {
            /// Wrap a value the core built.
            pub(crate) const fn from_core(inner: $core) -> Self {
                Self { inner }
            }
        }

        #[napi]
        impl $class {
            /// Build the element from its named facts, one record `Scalar`
            /// keyed by column name - the market and operation columns and
            /// the element's own `crosscode` and `srcuuids` - each checked by
            /// its column's field and stated through its column, then
            /// finalized. A `null` fact clears; a derived identity or any
            /// other event column is refused by name.
            #[napi(constructor)]
            pub fn new(facts: Option<&JsScalar>) -> Result<Self> {
                let mut element = stated_operation::<$kind>($name, 0, facts, true)?.into_element();
                ::yggdryl::graph::Element::finalize(&mut element);
                Ok(Self::from_core(element))
            }

            /// Which operation this is: `"order"`, `"quote"` or
            /// `"execution"`.
            #[napi(getter)]
            pub fn kind(&self) -> &'static str {
                self.inner.kind().as_str()
            }

            /// This element dated at `unix` nanoseconds since the epoch, and
            /// finalized.
            #[napi]
            pub fn at(&self, unix: Either<BigInt, f64>) -> Result<$event> {
                let unix = instant_of(unix, "unix")?;
                Ok($event::from_core(self.inner.clone().at(unix)))
            }
        }

        element_getters!($class);
        market_getters!($class);
        operation_getters!($class);
        common_verbs!($class);
        element_repr!($class, $name);
    };
}

/// One dated operation class over the core alias `$core` of kind `$kind`,
/// undated into `$element`: its constructor, `kind`, the book control,
/// `intoElement` and the shared segments.
macro_rules! operation_event_class {
    ($class:ident, $name:literal, $core:ty, $kind:ty, $element:ident) => {
        impl $class {
            /// Wrap a value the core built.
            pub(crate) const fn from_core(inner: $core) -> Self {
                Self { inner }
            }
        }

        #[napi]
        impl $class {
            /// Build the event at `currunix` nanoseconds since the epoch from
            /// its named facts, one record `Scalar` keyed by column name -
            /// the event, market and operation columns - each checked by its
            /// column's field and stated through its column, with `book`'s
            /// control facts, then finalized. A `null` fact clears; a
            /// derived identity, or `currunix` again, is refused by name.
            #[napi(constructor)]
            pub fn new(
                currunix: Either<BigInt, f64>,
                facts: Option<&JsScalar>,
                book: Option<&JsBookRef>,
            ) -> Result<Self> {
                let currunix = instant_of(currunix, "currunix")?;
                let mut event = stated_operation::<$kind>($name, currunix, facts, false)?;
                if let Some(book) = book {
                    event.set_book(Some(book.inner.clone()));
                }
                ::yggdryl::graph::Element::finalize(&mut event);
                Ok(Self::from_core(event))
            }

            /// Which operation this is: `"order"`, `"quote"` or
            /// `"execution"`.
            #[napi(getter)]
            pub fn kind(&self) -> &'static str {
                self.inner.kind().as_str()
            }

            /// The book-control facts, where this is a market-data entry.
            #[napi(getter)]
            pub fn book(&self) -> Option<JsBookRef> {
                self.inner.book().cloned().map(JsBookRef::from_core)
            }

            /// The update action the entry states, where it states one.
            #[napi(getter)]
            pub fn action(&self) -> Option<&'static str> {
                self.inner.action().map(CoreMdUpdateAction::as_str)
            }

            /// The book scope the entry states, empty where none.
            #[napi(getter)]
            pub fn scope(&self) -> String {
                self.inner.scope().to_owned()
            }

            /// Whether this is part of a FIX full-snapshot replacement.
            #[napi(getter)]
            pub fn is_full_snapshot(&self) -> bool {
                self.inner.is_full_snapshot()
            }

            /// This event with `book`'s control facts, refinalized.
            #[napi]
            pub fn with_book(&self, book: &JsBookRef) -> Self {
                let mut event = self.inner.clone().with_book(book.inner.clone());
                ::yggdryl::graph::Element::finalize(&mut event);
                Self::from_core(event)
            }

            /// This event without its clocks and book control, finalized as
            /// the undated element it then is.
            #[napi]
            pub fn into_element(&self) -> $element {
                let mut element = self.inner.clone().into_element();
                ::yggdryl::graph::Element::finalize(&mut element);
                $element::from_core(element)
            }
        }

        element_getters!($class);
        event_getters!($class);
        market_getters!($class);
        operation_getters!($class);
        common_verbs!($class);
        event_verbs!($class, $name);
    };
}

operation_element_class!(JsOrder, "Order", CoreOrder, OrderKind, JsOrderEvent);
operation_element_class!(JsQuote, "Quote", CoreQuote, QuoteKind, JsQuoteEvent);
operation_element_class!(
    JsExecution,
    "Execution",
    CoreExecution,
    ExecutionKind,
    JsExecutionEvent
);
operation_event_class!(
    JsOrderEvent,
    "OrderEvent",
    CoreOrderEvent,
    OrderKind,
    JsOrder
);
operation_event_class!(
    JsQuoteEvent,
    "QuoteEvent",
    CoreQuoteEvent,
    QuoteKind,
    JsQuote
);
operation_event_class!(
    JsExecutionEvent,
    "ExecutionEvent",
    CoreExecutionEvent,
    ExecutionKind,
    JsExecution
);

/// The action a spelling names, or an error listing every spelling.
fn md_update_action_of(text: &str) -> Result<CoreMdUpdateAction> {
    CoreMdUpdateAction::read(text).ok_or_else(|| {
        napi_error(format!(
            "unknown MdUpdateAction {text:?}; expected one of {:?}",
            CoreMdUpdateAction::ALL.map(CoreMdUpdateAction::as_str)
        ))
    })
}

/// A stated decimal slot, checked through the same `DataType::scalar` door
/// every other decimal slot in the graph binding crosses, so a value this
/// slot refuses is refused the same way theirs is.
fn stated_decimal(value: Scalar, name: &str) -> Result<Option<Decimal18>> {
    if matches!(value, Scalar::Null) {
        return Ok(None);
    }
    let checked = DataType::DECIMAL
        .nullable_field(name)
        .scalar(value)
        .map_err(napi_error)?;
    Decimal18::from_scalar(&checked)
        .map(Some)
        .ok_or_else(|| napi_error(format!("expected a decimal for {name}")))
}

/// One text or `null` slot of a JavaScript `toString`: `null` where the slot
/// states nothing, else its text quoted.
fn slot_text<T: std::fmt::Display>(slot: Option<T>) -> String {
    slot.map_or_else(
        || "null".to_owned(),
        |value| format!("{:?}", value.to_string()),
    )
}

/// The six slots a lane object states, each `undefined` or `null` where not
/// given: a lane has no third state to tell them apart by. Given, a slot is
/// widened through `Scalar.from` as a fact is - a decimal its text, a whole
/// number or a bigint; read back by `toJSON`, decimals and codes are their
/// text, as every graph getter answers them.
#[napi(object)]
#[derive(Clone, Default)]
pub struct LaneInput {
    /// The lane's price, as decimal text.
    #[napi(ts_type = "string | number | bigint | null")]
    pub price: Option<Either<String, Null>>,
    /// The spot part of an FX forward price, as decimal text.
    #[napi(ts_type = "string | number | bigint | null")]
    pub spotrate: Option<Either<String, Null>>,
    /// The forward points of an FX forward price, as decimal text.
    #[napi(ts_type = "string | number | bigint | null")]
    pub forwardpoints: Option<Either<String, Null>>,
    /// The currency, as its code text.
    #[napi(ts_type = "string | null")]
    pub currency: Option<Either<String, Null>>,
    /// The lane's quantity, as decimal text.
    #[napi(ts_type = "string | number | bigint | null")]
    pub quantity: Option<Either<String, Null>>,
    /// The unit the quantity is counted in, as spelled.
    #[napi(ts_type = "string | null")]
    pub unit: Option<Either<String, Null>>,
}

/// One lane of a quote: what a party is willing to pay or be paid, in the
/// currency and unit it states, with the FX parts of its price where it
/// quotes a forward. Every slot is what the lane states; a lane states
/// nothing of a slot it leaves `null`.
#[napi(js_name = "Lane")]
#[derive(Clone)]
pub struct JsLane {
    pub(crate) inner: CoreLane,
}

impl JsLane {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreLane) -> Self {
        Self { inner }
    }
}

/// The slots `input` states, in `names` order, each `Null` where it states
/// none: `input` is one record `Scalar` keyed by slot name - the loader
/// widened the caller's object through `Scalar.from`, the one door every
/// graph value crosses - and a key naming no slot is refused by name.
fn named_slots<const N: usize>(
    owner: &str,
    input: Option<&JsScalar>,
    names: [&str; N],
) -> Result<[Scalar; N]> {
    let mut slots = std::array::from_fn(|_| Scalar::Null);
    let Some(input) = input else {
        return Ok(slots);
    };
    let record = match &input.inner {
        Scalar::Null => return Ok(slots),
        record => record.as_struct().ok_or_else(|| {
            napi_error(format!(
                "{owner} takes an object keyed by slot name, got {}",
                record.kind()
            ))
        })?,
    };
    for (name, value) in record {
        let index = names
            .iter()
            .position(|slot| *slot == name.as_str())
            .ok_or_else(|| {
                napi_error(format!(
                    "{owner} has no slot {:?}; expected one of {names:?}",
                    name.as_str()
                ))
            })?;
        slots[index] = value.clone();
    }
    Ok(slots)
}

/// A stated text slot of `owner`, or `None` where it states none.
fn text_slot(owner: &str, name: &str, value: &Scalar) -> Result<Option<String>> {
    match value {
        Scalar::Null => Ok(None),
        value => value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| {
                napi_error(format!("{owner}.{name} must be text, got {}", value.kind()))
            }),
    }
}

/// A slot the lane states, as the object slot `toJSON` writes: `null` where
/// it states none.
fn slot_value(value: Option<String>) -> Either<String, Null> {
    value.map_or(Either::B(Null), Either::A)
}

#[napi]
impl JsLane {
    /// Build a lane from its six slots, one record `Scalar` keyed by slot
    /// name. The row crosses the boundary once, through the lane struct's
    /// own field - `lane_datatype`'s `scalar` - and is read back with
    /// `lane_of`, so every slot is validated exactly as a stored lane is.
    #[napi(constructor, ts_args_type = "input?: LaneInput | null")]
    pub fn new(input: Option<&JsScalar>) -> Result<Self> {
        let raw = Scalar::from_sequence(named_slots(
            "Lane",
            input,
            [
                "price",
                "spotrate",
                "forwardpoints",
                "currency",
                "quantity",
                "unit",
            ],
        )?);
        let dtype = lane_datatype().map_err(napi_error)?;
        let field = Field::new("lane", dtype, true);
        let checked = field.scalar(raw).map_err(napi_error)?;
        Ok(Self::from_core(lane_of(&checked).unwrap_or_default()))
    }

    /// The price this lane states, as decimal text; `null` where it states
    /// none.
    #[napi(getter)]
    pub fn price(&self) -> Option<String> {
        decimal_text(self.inner.price)
    }

    /// The spot part of an FX forward price; `null` where the lane states
    /// none.
    #[napi(getter)]
    pub fn spotrate(&self) -> Option<String> {
        decimal_text(self.inner.spotrate)
    }

    /// The forward points of an FX forward price; `null` where the lane
    /// states none.
    #[napi(getter)]
    pub fn forwardpoints(&self) -> Option<String> {
        decimal_text(self.inner.forwardpoints)
    }

    /// The currency, as the `ccy` code it is; `null` where the lane states
    /// none.
    #[napi(getter)]
    pub fn currency(&self) -> Option<String> {
        self.inner
            .currency
            .as_ref()
            .map(|held| held.as_str().to_owned())
    }

    /// The quantity this lane states, as decimal text; `null` where it
    /// states none.
    #[napi(getter)]
    pub fn quantity(&self) -> Option<String> {
        decimal_text(self.inner.quantity)
    }

    /// The unit the quantity is counted in, as spelled; `null` where the
    /// lane states none.
    #[napi(getter)]
    pub fn unit(&self) -> Option<String> {
        self.inner
            .unit
            .as_ref()
            .map(|held| held.as_str().to_owned())
    }

    /// Whether the lane states any slot.
    #[napi]
    pub fn is_stated(&self) -> bool {
        self.inner.is_stated()
    }

    /// Whether this lane states the same slots as `other`.
    #[napi]
    pub fn equals(&self, other: &JsLane) -> bool {
        self.inner == other.inner
    }

    /// The lane's own stable hash: the six slots it states, digested as
    /// `lane_fact` renders them.
    #[napi]
    pub fn stable_hash(&self) -> BigInt {
        BigInt::from(lane_fact(&self.inner).stable_hash())
    }

    /// A cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// `Lane(price=.., spotrate=.., forwardpoints=.., currency=.., quantity=.., unit=..)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "Lane(price={}, spotrate={}, forwardpoints={}, currency={}, quantity={}, unit={})",
            slot_text(self.inner.price),
            slot_text(self.inner.spotrate),
            slot_text(self.inner.forwardpoints),
            slot_text(self.inner.currency.as_ref().map(yggdryl::Ccy::as_str)),
            slot_text(self.inner.quantity),
            slot_text(self.inner.unit.as_ref().map(yggdryl::Unit::as_str)),
        )
    }

    /// The lane's own six slots, so it survives `JSON.stringify` and is
    /// what `new Lane(...)` reads back.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> LaneInput {
        LaneInput {
            price: Some(slot_value(self.price())),
            spotrate: Some(slot_value(self.spotrate())),
            forwardpoints: Some(slot_value(self.forwardpoints())),
            currency: Some(slot_value(self.currency())),
            quantity: Some(slot_value(self.quantity())),
            unit: Some(slot_value(self.unit())),
        }
    }
}

/// The five slots a book-control object states, each `undefined` or `null`
/// where not given; given, a slot is widened through `Scalar.from` as a
/// fact is, so a decimal is its text, a whole number or a bigint.
#[napi(object)]
#[derive(Clone, Default)]
pub struct BookRefInput {
    /// The update action, read through `MdUpdateAction::read`, refusing text
    /// that names no spelling.
    #[napi(ts_type = "string | null")]
    pub action: Option<Either<String, Null>>,
    /// The book scope this control belongs to.
    #[napi(ts_type = "string | null")]
    pub scope: Option<Either<String, Null>>,
    /// `MDEntryPositionNo(290)`: the entry's position in its level.
    #[napi(ts_type = "number | null")]
    pub position: Option<Either<f64, Null>>,
    /// The price this control states, as decimal text.
    #[napi(ts_type = "string | number | bigint | null")]
    pub entry_px: Option<Either<String, Null>>,
    /// The size this control states, as decimal text.
    #[napi(ts_type = "string | number | bigint | null")]
    pub entry_size: Option<Either<String, Null>>,
}

/// The typed book-control facts a market-data entry carries: what a book
/// reads to place the operation.
#[napi(js_name = "BookRef")]
#[derive(Clone)]
pub struct JsBookRef {
    pub(crate) inner: CoreBookRef,
}

impl JsBookRef {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreBookRef) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsBookRef {
    /// Build a book-control value from its five slots, one record `Scalar`
    /// keyed by slot name: `action` read through the `MdUpdateAction`
    /// vocabulary, `position` an unsigned 32-bit integer, the two decimals
    /// through the decimal field's own scalar door.
    #[napi(constructor, ts_args_type = "input?: BookRefInput | null")]
    pub fn new(input: Option<&JsScalar>) -> Result<Self> {
        let [action, scope, position, entry_px, entry_size] = named_slots(
            "BookRef",
            input,
            ["action", "scope", "position", "entryPx", "entrySize"],
        )?;
        let action = text_slot("BookRef", "action", &action)?
            .as_deref()
            .map(md_update_action_of)
            .transpose()?;
        let position = match position {
            Scalar::Null => None,
            value => Some(
                value
                    .as_i128()
                    .and_then(|position| u32::try_from(position).ok())
                    .ok_or_else(|| {
                        napi_error(format!(
                            "BookRef.position must be an unsigned 32-bit integer, got {}",
                            value.kind()
                        ))
                    })?,
            ),
        };
        Ok(Self::from_core(CoreBookRef {
            action,
            scope: text_slot("BookRef", "scope", &scope)?.map(Into::into),
            position,
            entry_px: stated_decimal(entry_px, "entry_px")?,
            entry_size: stated_decimal(entry_size, "entry_size")?,
        }))
    }

    /// The update action, where this control states one.
    #[napi(getter)]
    pub fn action(&self) -> Option<&'static str> {
        self.inner.action.map(CoreMdUpdateAction::as_str)
    }

    /// The book scope this control belongs to, where stated.
    #[napi(getter)]
    pub fn scope(&self) -> Option<String> {
        self.inner.scope.as_ref().map(ToString::to_string)
    }

    /// The entry's position in its level, where stated.
    #[napi(getter)]
    pub fn position(&self) -> Option<u32> {
        self.inner.position
    }

    /// The price this control states, as decimal text; `null` where it
    /// states none.
    #[napi(getter)]
    pub fn entry_px(&self) -> Option<String> {
        decimal_text(self.inner.entry_px)
    }

    /// The size this control states, as decimal text; `null` where it
    /// states none.
    #[napi(getter)]
    pub fn entry_size(&self) -> Option<String> {
        decimal_text(self.inner.entry_size)
    }

    /// Whether any control fact is stated.
    #[napi]
    pub fn is_stated(&self) -> bool {
        self.inner.is_stated()
    }

    /// Whether the stated action removes a range of positions.
    #[napi]
    pub fn is_range_delete(&self) -> bool {
        self.inner
            .action
            .is_some_and(CoreMdUpdateAction::is_range_delete)
    }

    /// Whether the stated action is a partial update of a live entry.
    #[napi]
    pub fn is_partial(&self) -> bool {
        self.inner
            .action
            .is_some_and(CoreMdUpdateAction::is_partial)
    }

    /// Whether this control states the same fields as `other`.
    #[napi]
    pub fn equals(&self, other: &JsBookRef) -> bool {
        self.inner == other.inner
    }

    /// The control's own stable hash: its five slots, digested as one
    /// record; equal controls share it.
    #[napi]
    pub fn stable_hash(&self) -> BigInt {
        BigInt::from(book_ref_hash(&self.inner))
    }

    /// A cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// `BookRef(action=.., scope=.., position=.., entryPx=.., entrySize=..)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "BookRef(action={}, scope={}, position={}, entryPx={}, entrySize={})",
            slot_text(self.action()),
            slot_text(self.inner.scope.as_deref()),
            self.inner
                .position
                .map_or_else(|| "null".to_owned(), |position| position.to_string()),
            slot_text(self.inner.entry_px),
            slot_text(self.inner.entry_size),
        )
    }

    /// The control's own five slots, so it survives `JSON.stringify` and is
    /// what `new BookRef(...)` reads back.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> BookRefInput {
        BookRefInput {
            action: Some(slot_value(self.action().map(ToOwned::to_owned))),
            scope: Some(slot_value(self.scope())),
            position: Some(
                self.inner
                    .position
                    .map_or(Either::B(Null), |position| Either::A(f64::from(position))),
            ),
            entry_px: Some(slot_value(self.entry_px())),
            entry_size: Some(slot_value(self.entry_size())),
        }
    }
}

/// The stable hash of a book control: its five slots as one record
/// `Scalar` - a slot it states nothing in a null - digested by the crate's
/// one `stable_hash`, so equal controls hash alike in either language.
pub(crate) fn book_ref_hash(book: &CoreBookRef) -> u64 {
    Scalar::from_sequence([
        book.action
            .map_or(Scalar::Null, |action| Scalar::from(action.as_str())),
        book.scope.clone().map_or(Scalar::Null, Scalar::from),
        book.position.map_or(Scalar::Null, Scalar::from),
        book.entry_px.map_or(Scalar::Null, Scalar::from),
        book.entry_size.map_or(Scalar::Null, Scalar::from),
    ])
    .stable_hash()
}
