//! Native Node.js view of [`CoreBookEvent`], [`CoreBookSide`],
//! [`CoreSnapshotEvent`], [`CoreSnapshotPartition`] and the
//! [`CoreBookIterator`] walk.

use napi::bindgen_prelude::{Array, BigInt, Either, Env, Function, Null, Result, Unknown};
use napi_derive::napi;
use yggdryl::graph::{
    BookEvent as CoreBookEvent, BookIterator as CoreBookIterator, BookSide as CoreBookSide, Event,
    Market, MarketData as CoreMarketData, SnapshotEvent as CoreSnapshotEvent,
    SnapshotPartition as CoreSnapshotPartition,
};
use yggdryl::{Scalar, Side as CoreSide};

use super::market_data::JsMarketData;
use super::operation::{JsBookRef, JsExecutionEvent};
use super::{AnyMarketData, decimal_text, instant_of, market_data_from, market_data_of, optional};
use crate::{Failed, Pulled, exact_u64, javascript_failure, napi_error, or_null, ordering_value};

/// The two named slots a snapshot-partition object states.
#[napi(object)]
#[derive(Clone)]
pub struct SnapshotPartitionInput {
    /// The book scope this partition replaces.
    pub scope: String,
    /// The symbol this partition is for; `null` in global mode.
    #[napi(ts_type = "string | null")]
    pub symbol: Option<Either<String, Null>>,
}

/// One scope a full snapshot replaces: a symbol - `null` in global mode -
/// and the book scope the entries stated.
#[napi(js_name = "SnapshotPartition")]
#[derive(Clone)]
pub struct JsSnapshotPartition {
    pub(crate) inner: CoreSnapshotPartition,
}

impl JsSnapshotPartition {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreSnapshotPartition) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsSnapshotPartition {
    /// A partition for `scope`, and `symbol` where the operations named
    /// one.
    #[napi(constructor)]
    pub fn new(input: SnapshotPartitionInput) -> Self {
        Self::from_core(CoreSnapshotPartition {
            symbol: optional(input.symbol).map(Into::into),
            scope: input.scope.into(),
        })
    }

    /// The book scope this partition replaces.
    #[napi(getter)]
    pub fn scope(&self) -> String {
        self.inner.scope.to_string()
    }

    /// The symbol this partition is for; `null` in global mode.
    #[napi(getter)]
    pub fn symbol(&self) -> Option<String> {
        self.inner.symbol.as_ref().map(ToString::to_string)
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsSnapshotPartition) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Whether this partition names the same scope and symbol as `other`.
    #[napi]
    pub fn equals(&self, other: &JsSnapshotPartition) -> bool {
        self.inner == other.inner
    }

    /// The partition's own stable hash: its symbol and scope, digested as
    /// one record; equal partitions share it.
    #[napi]
    pub fn stable_hash(&self) -> BigInt {
        BigInt::from(snapshot_partition_hash(&self.inner))
    }

    /// A cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// `SnapshotPartition(scope=.., symbol=..)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "SnapshotPartition(scope={:?}, symbol={})",
            self.inner.scope.as_str(),
            self.inner.symbol.as_ref().map_or_else(
                || "null".to_owned(),
                |symbol| format!("{:?}", symbol.as_str())
            )
        )
    }

    /// The partition's own two slots, so it survives `JSON.stringify` and
    /// is what `new SnapshotPartition(...)` reads back.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> SnapshotPartitionInput {
        SnapshotPartitionInput {
            scope: self.scope(),
            symbol: Some(self.symbol().map_or(Either::B(Null), Either::A)),
        }
    }
}

/// One price limit of a book side, as the plain object JavaScript reads.
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
}

/// A count of limits, checked once: a whole number of at most 2^53.
fn levels_of(levels: f64) -> Result<usize> {
    Ok(usize::try_from(exact_u64(levels, "levels")?).unwrap_or(usize::MAX))
}

/// One side of a book: persistent live orders and quotes, price ordered,
/// beside the deltas applied since the last emitted book. Immutable:
/// `withOperation` and every verb answer a new side.
#[napi(js_name = "BookSide")]
#[derive(Clone)]
pub struct JsBookSide {
    pub(crate) inner: CoreBookSide,
}

impl JsBookSide {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreBookSide) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsBookSide {
    /// An empty bid or ask side; `side` is read through the core `Side`
    /// vocabulary.
    #[napi(constructor)]
    pub fn new(side: String) -> Result<Self> {
        let side = CoreSide::read(&side).map_err(napi_error)?;
        CoreBookSide::new(side)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The live orders and quotes, best price first, each a `MarketData`.
    #[napi(getter)]
    pub fn live(&self) -> Vec<JsMarketData> {
        self.inner
            .live()
            .cloned()
            .map(JsMarketData::from_core)
            .collect()
    }

    /// The deltas applied since the last emitted book, each a `MarketData`.
    #[napi(getter)]
    pub fn deltas(&self) -> Vec<JsMarketData> {
        self.inner
            .deltas()
            .iter()
            .cloned()
            .map(JsMarketData::from_core)
            .collect()
    }

    /// How many identities are live on this side.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// Whether the side holds no live entry.
    #[napi(getter)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The best live price on this side, as decimal text; `null` where
    /// empty.
    #[napi(getter)]
    pub fn best_price(&self) -> Option<String> {
        decimal_text(self.inner.best_price())
    }

    /// The aggregate quantity at the exact best price; `null` where empty.
    #[napi(getter)]
    pub fn best_quantity(&self) -> Option<String> {
        decimal_text(self.inner.best_quantity())
    }

    /// One limit per price level, best first and the one unpriced limit
    /// last, each naming its entries' `curruuid`s in position order.
    #[napi(getter)]
    pub fn limits(&self) -> Vec<BookLimit> {
        self.inner
            .limits()
            .map(|limit| BookLimit {
                price: or_null(decimal_text(limit.price)),
                quantity: limit.quantity.to_string(),
                uuids: limit.uuids.iter().map(ToString::to_string).collect(),
            })
            .collect()
    }

    /// The exact quantity resting on the first `levels` limits, the
    /// unpriced one counted where reached, as decimal text: `'0'` for an
    /// empty side or no level, `null` only past what a decimal holds.
    #[napi]
    pub fn depth(&self, levels: f64) -> Result<Option<String>> {
        Ok(decimal_text(self.inner.depth(levels_of(levels)?)))
    }

    /// This side with one order or quote event - a leaf or a `MarketData` -
    /// atomically applied.
    #[napi(
        ts_args_type = "operation: MarketData | Order | Quote | Execution | BookSide | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent"
    )]
    pub fn with_operation(&self, operation: Unknown<'_>) -> Result<Self> {
        let operation = market_data_from(operation)?;
        let mut side = self.inner.clone();
        side.add_operation(operation).map_err(napi_error)?;
        Ok(Self::from_core(side))
    }
}

element_getters!(JsBookSide);
market_getters!(JsBookSide);
common_verbs!(JsBookSide);
element_repr!(JsBookSide, "BookSide");

/// One coherent view of a market at one exact nanosecond instant: the bid
/// and ask depth, the executions at that instant and the scopes its last
/// snapshot replaced. Immutable: `withOperations` and every verb answer a
/// new book.
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
    /// An empty book for `symbol` at `currunix` nanoseconds since the epoch.
    #[napi(constructor)]
    pub fn new(currunix: Either<BigInt, f64>, symbol: String) -> Result<Self> {
        let currunix = instant_of(currunix, "currunix")?;
        Ok(Self::from_core(CoreBookEvent::new(currunix, symbol)))
    }

    /// The bid side.
    #[napi(getter)]
    pub fn bid(&self) -> JsBookSide {
        JsBookSide::from_core(self.inner.bid().clone())
    }

    /// The ask side, shaped as the bid.
    #[napi(getter)]
    pub fn ask(&self) -> JsBookSide {
        JsBookSide::from_core(self.inner.ask().clone())
    }

    /// The executions at this book's instant.
    #[napi(getter)]
    pub fn executions(&self) -> Vec<JsExecutionEvent> {
        self.inner
            .executions()
            .iter()
            .cloned()
            .map(JsExecutionEvent::from_core)
            .collect()
    }

    /// The scopes this book's last full snapshot replaced.
    #[napi(getter)]
    pub fn snapshot_partitions(&self) -> Vec<JsSnapshotPartition> {
        self.inner
            .snapshot_partitions()
            .iter()
            .cloned()
            .map(JsSnapshotPartition::from_core)
            .collect()
    }

    /// Whether the best bid is strictly above the best ask.
    #[napi(getter)]
    pub fn is_crossed(&self) -> bool {
        self.inner.is_crossed()
    }

    /// Whether both bests are stated and equal.
    #[napi(getter)]
    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    /// The best ask less the best bid, negative when crossed, as decimal
    /// text; `null` where a side states no best.
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
    /// order, quote, execution or trade event, a snapshot control, or a
    /// `MarketData` holding one.
    #[napi(
        ts_args_type = "operations: Array<MarketData | Order | Quote | Execution | BookSide | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent>"
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
common_verbs!(JsSnapshotEvent);
event_verbs!(JsSnapshotEvent, "SnapshotEvent");

/// The core stage's source: the caller's items as `MarketData`, a
/// JavaScript failure crossing as one typed sentinel.
type BookSource = Box<dyn Iterator<Item = yggdryl::Result<CoreMarketData>> + Send>;

/// Books from a sorted stream of operations, one per symbol and effective
/// timestamp, or one consolidated `GLOBAL` book, pulling its items lazily
/// from the caller's iterable. Yields `BookEvent`.
#[napi(js_name = "BookIterator")]
pub struct JsBookIterator {
    inner: CoreBookIterator<BookSource>,
    failed: Failed,
}

#[napi]
impl JsBookIterator {
    /// Opens a book walk over the items `pull` hands over - any leaf or
    /// `MarketData`, sorted by their own event order; `snapshotMillis === 0`
    /// disables grid snapshots and `global` emits one consolidated `GLOBAL`
    /// book.
    #[napi(factory, js_name = "_bookIteratorNative", skip_typescript)]
    pub fn new_native(
        env: Env,
        pull: Function<'_, (), Option<AnyMarketData<'static>>>,
        snapshot_millis: f64,
        global: bool,
    ) -> Result<Self> {
        let snapshot_millis = exact_u64(snapshot_millis, "snapshotMillis")?;
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        // The core stage takes a typed error, not a native one, so a
        // JavaScript failure crosses it as one sentinel `Err` - peeked, never
        // taken, so `failed` still holds the original for `next` to throw as
        // itself once the stage surfaces the sentinel in its place. Yielded
        // exactly once: `Chain` stops calling a side once it answers `None`.
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
        let inner = CoreBookIterator::new(source, snapshot_millis, global).map_err(napi_error)?;
        Ok(Self { inner, failed })
    }

    /// Whether this walk emits one consolidated `GLOBAL` book.
    #[napi(getter)]
    pub fn global(&self) -> bool {
        self.inner.global()
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
            // it is thrown as itself rather than as the typed error it
            // crossed the stage wrapped in.
            Some(Err(error)) => Err(self.failed.take().unwrap_or_else(|| napi_error(error))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}

/// The stable hash of a snapshot partition: its symbol and scope as one
/// record `Scalar` - no symbol a null - digested by the crate's one
/// `stable_hash`, so equal partitions hash alike in either language.
pub(crate) fn snapshot_partition_hash(partition: &CoreSnapshotPartition) -> u64 {
    Scalar::from_sequence([
        partition.symbol.clone().map_or(Scalar::Null, Scalar::from),
        Scalar::from(partition.scope.clone()),
    ])
    .stable_hash()
}
