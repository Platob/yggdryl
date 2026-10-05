//! Native Node.js view of [`CoreBookEvent`], [`CoreSnapshotEvent`] and the
//! [`CoreBookIterator`] walk.

use napi::bindgen_prelude::{
    Array, BigInt, ClassInstance, Either, Either3, Env, Function, Null, Result, Unknown,
};
use napi_derive::napi;
use yggdryl::Side as CoreSide;
use yggdryl::graph::{
    BookEvent as CoreBookEvent, BookIterator as CoreBookIterator, Event, Market,
    MarketData as CoreMarketData, SnapshotEvent as CoreSnapshotEvent,
};

use super::market_data::JsMarketData;
use super::operation::JsBookRef;
use super::{AnyMarketData, decimal_text, market_data_from, market_data_of};
use crate::expression::{JsFilter, JsTerm, filter_from_input};
use crate::{
    Failed, Pulled, exact_i64, exact_i64_input, exact_u64, javascript_failure, napi_error, or_null,
};

/// One price level of a book's side, as the plain object JavaScript reads.
#[napi(object, object_from_js = false)]
pub struct BookLimit {
    /// The limit's price as decimal text; `null` on the one limit folding
    /// every entry that states no price.
    #[napi(ts_type = "string | null")]
    pub price: Either<String, Null>,
    /// The exact sum of the quantities its entries state, as decimal text;
    /// an entry stating none adds nothing.
    pub quantity: String,
    /// Its entries' `curruuid`s in live order: position, then arrival.
    pub uuids: Vec<String>,
    /// Whether the level can trade: any of its entries does not state
    /// `tradable = false`.
    pub tradable: bool,
}

/// A count of limits, checked once: a whole number of at most 2^53.
fn levels_of(levels: f64) -> Result<usize> {
    Ok(usize::try_from(exact_u64(levels, "levels")?).unwrap_or(usize::MAX))
}

/// The side `side` names: a spelling read through the core `Side`
/// vocabulary, or a `Side` code - what `Side.BUYS` holds - read as the code
/// a `side` column stores.
fn side_of(side: Either<String, f64>) -> Result<CoreSide> {
    match side {
        Either::A(text) => CoreSide::read(&text),
        Either::B(code) => CoreSide::read_code(exact_i64(code, "side")?),
    }
    .map_err(napi_error)
}

/// The entries `entries` answers, each a `MarketData`.
fn market_data_list<'a>(entries: impl Iterator<Item = &'a CoreMarketData>) -> Vec<JsMarketData> {
    entries.cloned().map(JsMarketData::from_core).collect()
}

/// One coherent view of a market at one exact nanosecond instant: on a
/// complete book every live entry of both sides and the price levels of
/// each, and on every book the deltas applied since the book before it and
/// the top of book it settled on. A walk emits a book whole only at a
/// snapshot tick and every other book as its deltas alone, which
/// `withPrevious` over the complete book before it rebuilds. Immutable:
/// `withOperations` and every verb answer a new book.
#[napi(js_name = "BookEvent")]
#[derive(Clone)]
pub struct JsBookEvent {
    pub(crate) inner: CoreBookEvent,
}

impl JsBookEvent {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreBookEvent) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsBookEvent {
    /// An empty book of the ticker `symbol` at `currunix` nanoseconds since
    /// the epoch, keyed by that ticker; an empty `symbol` keys the book
    /// `XX0000000000`, the ISIN that states none, and states no ticker.
    #[napi(constructor)]
    pub fn new(currunix: Either<BigInt, f64>, symbol: String) -> Result<Self> {
        let currunix = exact_i64_input(currunix, "currunix")?;
        Ok(Self::from_core(CoreBookEvent::new(currunix, symbol)))
    }

    /// An empty book keyed `key` at `currunix` nanoseconds since the epoch:
    /// `key` is its crosscode - an instrument's ISIN, a ticker, or
    /// `XX0000000000` - and the book states neither a ticker nor an ISIN.
    /// The empty base a code's first book, stating its deltas alone,
    /// rebuilds over with `withPrevious`.
    #[napi(factory)]
    pub fn keyed(currunix: Either<BigInt, f64>, key: String) -> Result<Self> {
        let currunix = exact_i64_input(currunix, "currunix")?;
        Ok(Self::from_core(CoreBookEvent::keyed(currunix, key)))
    }

    /// Whether the book holds its sides - every entry alive on it - rather
    /// than only the deltas it applied since the book before it: a book a
    /// caller builds, one a walk emits at a snapshot tick, and one rebuilt
    /// by `withPrevious` are complete.
    #[napi(getter)]
    pub fn is_complete(&self) -> bool {
        self.inner.is_complete()
    }

    /// Every entry alive on the book, each once and a `MarketData`: the bid
    /// side's, best price first and every entry stating no price last, then
    /// the ask side's the same way but those resting on the bid too - a
    /// two-sided quote is one entry, listed with the bids. Empty on a book
    /// stating its deltas alone.
    #[napi]
    pub fn alive(&self) -> Vec<JsMarketData> {
        market_data_list(self.inner.alive())
    }

    /// The entries alive on the side `side` names - read through the `Side`
    /// vocabulary - each a `MarketData`, best price first and every entry
    /// stating no price last - a two-sided quote on both sides. Empty for a
    /// side that is neither a bid nor an ask, or on a book stating its
    /// deltas alone.
    #[napi]
    pub fn alive_on(&self, side: Either<String, f64>) -> Result<Vec<JsMarketData>> {
        Ok(market_data_list(self.inner.alive_on(side_of(side)?)))
    }

    /// The orders and quotes applied since the book before this one, each a
    /// `MarketData`, in the order applied across both sides: what a book
    /// stating its deltas alone states, and what `withPrevious` replays
    /// over the book before it.
    #[napi]
    pub fn deltas(&self) -> Vec<JsMarketData> {
        market_data_list(self.inner.deltas())
    }

    /// One limit per price level of the side `side` names - read through
    /// the `Side` vocabulary - best first and the one unpriced limit last,
    /// each naming its entries' `curruuid`s in position order; empty for a
    /// side that is neither a bid nor an ask, and on a book stating its
    /// deltas alone.
    #[napi]
    pub fn limits(&self, side: Either<String, f64>) -> Result<Vec<BookLimit>> {
        Ok(self
            .inner
            .limits(side_of(side)?)
            .map(|limit| BookLimit {
                price: or_null(decimal_text(limit.price)),
                quantity: limit.quantity.to_string(),
                uuids: limit.uuids.iter().map(ToString::to_string).collect(),
                tradable: limit.tradable,
            })
            .collect())
    }

    /// The best tradable price on the side `side` names, as decimal text:
    /// the first priced level that can trade; `null` where none can.
    #[napi]
    pub fn best_price(&self, side: Either<String, f64>) -> Result<Option<String>> {
        Ok(decimal_text(self.inner.best_price(side_of(side)?)))
    }

    /// The exact aggregate quantity at `bestPrice(side)`, as decimal text,
    /// an entry stating none adding nothing; `null` where `bestPrice` is.
    #[napi]
    pub fn best_quantity(&self, side: Either<String, f64>) -> Result<Option<String>> {
        Ok(decimal_text(self.inner.best_quantity(side_of(side)?)))
    }

    /// The exact quantity resting on the first `levels` limits of the side
    /// `side` names, the unpriced one counted where reached, as decimal
    /// text: `'0'` for an empty side or no level, `null` past what a decimal
    /// holds or for a side that is neither a bid nor an ask.
    #[napi]
    pub fn depth(&self, side: Either<String, f64>, levels: f64) -> Result<Option<String>> {
        Ok(decimal_text(
            self.inner.depth(side_of(side)?, levels_of(levels)?),
        ))
    }

    /// Whether the best tradable bid is strictly above the best tradable
    /// ask.
    #[napi(getter)]
    pub fn is_crossed(&self) -> bool {
        self.inner.is_crossed()
    }

    /// Whether both best tradable prices are stated and equal.
    #[napi(getter)]
    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    /// The best tradable ask less the best tradable bid, negative when
    /// crossed, as decimal text; `null` where a side states no best.
    #[napi(getter)]
    pub fn spread(&self) -> Option<String> {
        decimal_text(self.inner.spread())
    }

    /// `(bid - ask) / (bid + ask)` over the two sides' `depth(levels)`, as
    /// decimal text: `'1'` bid-only, `'-1'` ask-only; `null` when the total
    /// is zero - both sides empty, or no level.
    #[napi]
    pub fn imbalance(&self, levels: f64) -> Result<Option<String>> {
        Ok(decimal_text(self.inner.imbalance(levels_of(levels)?)))
    }

    /// The arithmetic midpoint of a coherent two-sided best bid and offer,
    /// as decimal text; `null` where there is none.
    #[napi(getter)]
    pub fn bbo_midpoint(&self) -> Option<String> {
        decimal_text(self.inner.bbo_midpoint())
    }

    /// The two-value median of the best bid and ask aggregate quantities,
    /// as decimal text; `null` where there is none.
    #[napi(getter)]
    pub fn median_quantity(&self) -> Option<String> {
        decimal_text(self.inner.median_quantity())
    }

    /// This book with every operation of one atomic group applied: each an
    /// order or quote event, a snapshot control, or a `MarketData` holding
    /// one, folded, and an execution event recorded among the deltas,
    /// moving no side, since a fill moves a book through its order's or
    /// quote's report; a trade event is pruned and changes nothing. A book
    /// stating its deltas alone is refused at `$.alive`.
    #[napi(
        ts_args_type = "operations: Array<MarketData | Order | Quote | Execution | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent>"
    )]
    pub fn with_operations(&self, operations: Array<'_>) -> Result<Self> {
        let mut items = Vec::with_capacity(operations.len() as usize);
        for index in 0..operations.len() {
            let item = operations
                .get::<Unknown<'_>>(index)?
                .ok_or_else(|| napi_error(format!("operations[{index}]: missing")))?;
            items.push(
                market_data_from(item).map_err(|error| {
                    napi_error(format!("operations[{index}]: {}", error.reason))
                })?,
            );
        }
        let mut book = self.inner.clone();
        book.add_operations(items).map_err(napi_error)?;
        Ok(Self::from_core(book))
    }
}

element_getters!(JsBookEvent);
event_getters!(JsBookEvent);
market_getters!(JsBookEvent);
marketdatakind_getter!(JsBookEvent, BookEvent);
common_verbs!(JsBookEvent);
event_verbs!(JsBookEvent, "BookEvent");

/// A dated market element, whichever leaf holds it: what a snapshot
/// control copies.
trait EventMarket: Event + Market {}
impl<T: Event + Market + ?Sized> EventMarket for T {}

/// The dated market element `data` holds - any of the six dated leaves - or
/// `None` for an undated one.
fn event_market_of(data: &CoreMarketData) -> Option<&dyn EventMarket> {
    match data {
        CoreMarketData::OrderEvent(leaf) => Some(leaf),
        CoreMarketData::QuoteEvent(leaf) => Some(leaf),
        CoreMarketData::ExecutionEvent(leaf) => Some(leaf),
        CoreMarketData::TradeEvent(leaf) => Some(leaf),
        CoreMarketData::BookEvent(leaf) => Some(leaf.as_ref()),
        CoreMarketData::SnapshotEvent(leaf) => Some(leaf),
        _ => None,
    }
}

/// The full-snapshot control an empty FIX `W` is: an event stating the scope
/// it replaces and no entry of its own.
#[napi(js_name = "SnapshotEvent")]
#[derive(Clone)]
pub struct JsSnapshotEvent {
    pub(crate) inner: CoreSnapshotEvent,
}

impl JsSnapshotEvent {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreSnapshotEvent) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsSnapshotEvent {
    /// The snapshot control over `event` - any dated leaf, whose event and
    /// market facts are copied - replacing `scope`.
    #[napi(
        factory,
        ts_args_type = "event: MarketData | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent, scope?: string | null"
    )]
    pub fn snapshot(event: Unknown<'_>, scope: Option<String>) -> Result<Self> {
        let data = market_data_from(event)?;
        let event = event_market_of(&data).ok_or_else(|| {
            napi_error(format!(
                "expected a dated leaf to snapshot, got {}",
                data.kind().as_str()
            ))
        })?;
        Ok(Self::from_core(CoreSnapshotEvent::snapshot(
            event,
            scope.map(Into::into),
        )))
    }

    /// The control's book facts: always a full snapshot, with its scope.
    #[napi(getter)]
    pub fn book(&self) -> JsBookRef {
        JsBookRef::from_core(self.inner.book().clone())
    }
}

element_getters!(JsSnapshotEvent);
event_getters!(JsSnapshotEvent);
market_getters!(JsSnapshotEvent);
marketdatakind_getter!(JsSnapshotEvent, SnapshotEvent);
common_verbs!(JsSnapshotEvent);
event_verbs!(JsSnapshotEvent, "SnapshotEvent");

/// The core stage's source: the caller's items as `MarketData`, a
/// JavaScript failure crossing as one typed sentinel.
type BookSource = Box<dyn Iterator<Item = yggdryl::Result<CoreMarketData>> + Send>;

/// Books from a sorted stream of operations, one per book key and effective
/// timestamp, pulling its items lazily from the caller's iterable. Yields
/// `BookEvent`.
#[napi(js_name = "BookIterator")]
pub struct JsBookIterator {
    inner: CoreBookIterator<BookSource>,
    failed: Failed,
}

#[napi]
impl JsBookIterator {
    /// Opens a book walk over the items `pull` hands over - any leaf or
    /// `MarketData`, sorted by their own event order; `snapshotMillis === 0`
    /// disables grid snapshots, so a book is emitted whole only at a full
    /// refresh.
    ///
    /// The walk folds orders, quotes and snapshot controls, records an
    /// execution among its instant's deltas and prunes every other input
    /// where it is pulled. `filter` - a `Filter`, a `Term` or the text of a
    /// predicate over the `marketdata` row - narrows it further, bound once
    /// here; it never admits a pruned kind.
    /// Not given, every booked input is kept.
    #[napi(factory, js_name = "_bookIteratorNative", skip_typescript)]
    pub fn new_native(
        env: Env,
        pull: Function<'_, (), Option<AnyMarketData<'static>>>,
        snapshot_millis: f64,
        filter: Option<Either3<ClassInstance<'_, JsFilter>, ClassInstance<'_, JsTerm>, String>>,
    ) -> Result<Self> {
        let snapshot_millis = exact_u64(snapshot_millis, "snapshotMillis")?;
        let filter = filter.map(filter_from_input).transpose()?;
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        // The core stage takes a typed error, not a native one, so a
        // JavaScript failure - which the loader throws on into the pull -
        // crosses it as one sentinel `Err`, and the stage ends there as it
        // would on a refusal of its own. `failed` is peeked, never taken, so
        // it still holds the failure for `next` to throw once the stage
        // surfaces the sentinel in its place; the loader throws the
        // JavaScript original instead. Yielded exactly once: `Chain` stops
        // calling a side once it answers `None`.
        let source: BookSource = {
            let sentinel_failed = failed.clone();
            let mut yielded = false;
            Box::new(
                pulled
                    .map(|item| Ok(market_data_of(&item)))
                    .chain(std::iter::from_fn(move || {
                        if yielded {
                            return None;
                        }
                        yielded = true;
                        sentinel_failed
                            .peek()
                            .map(|error| Err(javascript_failure(error)))
                    })),
            )
        };
        let mut inner = CoreBookIterator::new(source, snapshot_millis).map_err(napi_error)?;
        if let Some(filter) = filter {
            inner = inner.with_filter(filter).map_err(napi_error)?;
        }
        Ok(Self { inner, failed })
    }

    /// Advance the walk: the next book, or `null` at its end. The loader
    /// wraps this into the iterator protocol.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<BookEvent>")]
    pub fn next(&mut self) -> Result<Option<JsBookEvent>> {
        match self.inner.next() {
            Some(Ok(book)) => Ok(Some(JsBookEvent::from_core(book))),
            // A core refusal (a bad item) or the sentinel standing in for a
            // JavaScript failure both land here; `self.failed` still holds a
            // genuine failure - never taken by the source, only peeked - so
            // it is thrown rather than the typed error it crossed the stage
            // wrapped in, and the loader throws the JavaScript original in
            // its place.
            Some(Err(error)) => Err(self.failed.take().unwrap_or_else(|| napi_error(error))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
