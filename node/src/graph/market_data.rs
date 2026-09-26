//! Native Node.js view of [`CoreMarketData`], the one value over every leaf,
//! its lifted Arrow doors and the `toJSON` stream every leaf shares.

use std::iter::FusedIterator;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use napi::bindgen_prelude::{ClassInstance, Either, Either10, Env, Function, Result, Unknown};
use napi_derive::napi;
use yggdryl::FieldPath;
use yggdryl::graph::{MarketData as CoreMarketData, MarketKind, MarketView as CoreMarketView};
use yggdryl::holder::Buffer;
use yggdryl::ipc::{self, IpcOptions};

use super::book::{JsBookEvent, JsBookSide, JsSnapshotEvent};
use super::operation::{
    JsBookRef, JsExecution, JsExecutionEvent, JsOrder, JsOrderEvent, JsQuote, JsQuoteEvent,
};
use super::trade::JsTradeEvent;
use super::{AnyMarketData, market_data_from, market_data_of};
use crate::expression::JsPlan;
use crate::field::JsField;
use crate::iomedia::JsBatchReader;
use crate::text::line::{JsFieldPath, path_from_input};
use crate::{Pulled, exact_u64, javascript_failure, napi_error};

/// The root a batch of `MarketData` rows is named by, the core's own
/// spelling.
const ROOT_NAME: &str = "marketdata";

/// The leaf a value holds, as the class of its variant.
type Leaf = Either10<
    JsOrder,
    JsQuote,
    JsExecution,
    JsBookSide,
    JsOrderEvent,
    JsQuoteEvent,
    JsExecutionEvent,
    JsTradeEvent,
    JsBookEvent,
    JsSnapshotEvent,
>;

/// The leaf object `data` holds, as the class of its variant.
fn leaf_object(data: CoreMarketData) -> Leaf {
    match data {
        CoreMarketData::Order(leaf) => Leaf::A(JsOrder::from_core(leaf)),
        CoreMarketData::Quote(leaf) => Leaf::B(JsQuote::from_core(leaf)),
        CoreMarketData::Execution(leaf) => Leaf::C(JsExecution::from_core(leaf)),
        CoreMarketData::BookSide(leaf) => Leaf::D(JsBookSide::from_core(leaf)),
        CoreMarketData::OrderEvent(leaf) => Leaf::E(JsOrderEvent::from_core(leaf)),
        CoreMarketData::QuoteEvent(leaf) => Leaf::F(JsQuoteEvent::from_core(leaf)),
        CoreMarketData::ExecutionEvent(leaf) => Leaf::G(JsExecutionEvent::from_core(leaf)),
        CoreMarketData::TradeEvent(leaf) => Leaf::H(JsTradeEvent::from_core(leaf)),
        CoreMarketData::BookEvent(leaf) => Leaf::I(JsBookEvent::from_core(*leaf)),
        CoreMarketData::SnapshotEvent(leaf) => Leaf::J(JsSnapshotEvent::from_core(leaf)),
    }
}

/// The base64 text of `data`'s one-row `MarketData::arrow_reader` IPC
/// stream: what `toJSON` writes for every leaf.
pub(crate) fn into_json(data: CoreMarketData) -> Result<String> {
    let reader = CoreMarketData::arrow_reader([data], None, None).map_err(napi_error)?;
    let mut buffer = Buffer::new();
    ipc::overwrite_arrow_reader(&mut buffer, reader, &IpcOptions::new()).map_err(napi_error)?;
    Ok(BASE64.encode(buffer.into_bytes()))
}

/// The one value an [`into_json`] text holds, read back through
/// `MarketData::from_arrow_reader`.
pub(crate) fn from_json(text: &str) -> Result<CoreMarketData> {
    let stream = BASE64
        .decode(text)
        .map_err(|error| napi_error(format!("expected base64 MarketData text: {error}")))?;
    let buffer = Buffer::from_bytes(stream);
    let reader = ipc::read_batch_reader(&buffer, None, &IpcOptions::new()).map_err(napi_error)?;
    let mut rows = CoreMarketData::from_arrow_reader(reader).map_err(napi_error)?;
    let data = rows
        .next()
        .ok_or_else(|| napi_error("the MarketData text holds no row"))?
        .map_err(napi_error)?;
    if rows.next().is_some() {
        return Err(napi_error("the MarketData text holds more than one row"));
    }
    Ok(data)
}

/// The lifts a view appends, each path read once.
fn lifts_of(
    lifts: Option<Vec<Either<String, ClassInstance<'_, JsFieldPath>>>>,
) -> Result<Vec<FieldPath>> {
    lifts
        .unwrap_or_default()
        .into_iter()
        .map(|lift| {
            path_from_input(match &lift {
                Either::A(text) => Either::A(text.clone()),
                Either::B(path) => Either::B(&**path),
            })
        })
        .collect()
}

/// The batch bounds a caller stated, checked once.
fn batch_sizes(
    batch_row_size: Option<f64>,
    batch_byte_size: Option<f64>,
) -> Result<(Option<usize>, Option<u64>)> {
    let row = batch_row_size
        .map(|value| exact_u64(value, "batchRowSize"))
        .transpose()?
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX));
    let byte = batch_byte_size
        .map(|value| exact_u64(value, "batchByteSize"))
        .transpose()?;
    Ok((row, byte))
}

/// One value over every market leaf - an order, a quote or an execution,
/// undated or dated, a book side, a trade, a book or a snapshot control -
/// answering the element and market facts its leaf answers. Immutable:
/// every verb answers a new value.
#[napi(js_name = "MarketData")]
#[derive(Clone)]
pub struct JsMarketData {
    pub(crate) inner: CoreMarketData,
}

impl JsMarketData {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreMarketData) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsMarketData {
    /// Wrap any market leaf, through the core's own `From`.
    #[napi(
        constructor,
        ts_args_type = "leaf: MarketData | Order | Quote | Execution | BookSide | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent"
    )]
    pub fn new(leaf: Unknown<'_>) -> Result<Self> {
        market_data_from(leaf).map(Self::from_core)
    }

    /// Every leaf kind a value may be, in declaration order.
    #[napi]
    pub fn kinds() -> Vec<&'static str> {
        MarketKind::ALL.map(MarketKind::as_str).to_vec()
    }

    /// Which leaf this is, as its `MarketData.kinds()` spelling.
    #[napi(getter)]
    pub fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// Whether the leaf is one of the six dated ones.
    #[napi(getter)]
    pub fn is_event(&self) -> bool {
        self.inner.is_event()
    }

    /// The book control of an operation event or a snapshot control, else
    /// `null`.
    #[napi(getter)]
    pub fn book(&self) -> Option<JsBookRef> {
        self.inner.book().cloned().map(JsBookRef::from_core)
    }

    /// The undated order this value is, else `null`.
    #[napi]
    pub fn as_order(&self) -> Option<JsOrder> {
        self.inner.as_order().cloned().map(JsOrder::from_core)
    }

    /// The undated quote this value is, else `null`.
    #[napi]
    pub fn as_quote(&self) -> Option<JsQuote> {
        self.inner.as_quote().cloned().map(JsQuote::from_core)
    }

    /// The undated execution this value is, else `null`.
    #[napi]
    pub fn as_execution(&self) -> Option<JsExecution> {
        self.inner
            .as_execution()
            .cloned()
            .map(JsExecution::from_core)
    }

    /// The book side this value is, else `null`.
    #[napi]
    pub fn as_book_side(&self) -> Option<JsBookSide> {
        self.inner
            .as_book_side()
            .cloned()
            .map(JsBookSide::from_core)
    }

    /// The dated order this value is, else `null`.
    #[napi]
    pub fn as_order_event(&self) -> Option<JsOrderEvent> {
        self.inner
            .as_order_event()
            .cloned()
            .map(JsOrderEvent::from_core)
    }

    /// The dated quote this value is, else `null`.
    #[napi]
    pub fn as_quote_event(&self) -> Option<JsQuoteEvent> {
        self.inner
            .as_quote_event()
            .cloned()
            .map(JsQuoteEvent::from_core)
    }

    /// The dated execution this value is, else `null`.
    #[napi]
    pub fn as_execution_event(&self) -> Option<JsExecutionEvent> {
        self.inner
            .as_execution_event()
            .cloned()
            .map(JsExecutionEvent::from_core)
    }

    /// The trade this value is, else `null`.
    #[napi]
    pub fn as_trade_event(&self) -> Option<JsTradeEvent> {
        self.inner
            .as_trade_event()
            .cloned()
            .map(JsTradeEvent::from_core)
    }

    /// The book this value is, else `null`.
    #[napi]
    pub fn as_book_event(&self) -> Option<JsBookEvent> {
        self.inner
            .as_book_event()
            .cloned()
            .map(JsBookEvent::from_core)
    }

    /// The snapshot control this value is, else `null`.
    #[napi]
    pub fn as_snapshot_event(&self) -> Option<JsSnapshotEvent> {
        self.inner
            .as_snapshot_event()
            .cloned()
            .map(JsSnapshotEvent::from_core)
    }

    /// The leaf this value holds, as its own class.
    #[napi(
        ts_return_type = "Order | Quote | Execution | BookSide | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent"
    )]
    pub fn into_leaf(&self) -> Leaf {
        leaf_object(self.inner.clone())
    }

    /// The lifted `marketdata` row field every leaf is written under.
    #[napi]
    pub fn field() -> Result<JsField> {
        CoreMarketData::field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Reads `MarketData` from record batches - any subset of the lifted
    /// columns, in any order, foreign columns ignored: a lazy walk, fused
    /// after an error, the error thrown at the failing item.
    #[napi]
    pub fn from_arrow_reader(reader: &mut JsBatchReader) -> Result<JsMarketDataRowIterator> {
        let batches = reader.take()?;
        let rows = CoreMarketData::from_arrow_reader(batches).map_err(napi_error)?;
        Ok(JsMarketDataRowIterator::over(Box::new(rows)))
    }

    /// The plan one named view is over a `marketdata` stream - `orders`,
    /// `quotes`, `executions`, `trades`, `book_sides`, `books`, or the
    /// `lifecycle` of the chain `crosscode` names, the one view that takes
    /// one - read ignoring ASCII case, with each lift, a `FieldPath` read
    /// once, appended as a projection after the view's own columns. Built
    /// structurally; its text reads back as the same plan.
    #[napi(
        ts_args_type = "view: string, lifts?: Array<string | FieldPath> | null, crosscode?: string | null"
    )]
    pub fn plan(
        view: String,
        lifts: Option<Vec<Either<String, ClassInstance<'_, JsFieldPath>>>>,
        crosscode: Option<String>,
    ) -> Result<JsPlan> {
        let view = CoreMarketView::read(&view, crosscode.as_deref()).map_err(napi_error)?;
        let lifts = lifts_of(lifts)?;
        CoreMarketData::plan(&view, &lifts)
            .map(JsPlan::from_core)
            .map_err(napi_error)
    }

    /// `MarketData(<curruuid>, kind=.., crosscode=..)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "MarketData({}, kind={:?}, crosscode={:?})",
            yggdryl::graph::Element::get_curruuid(&self.inner),
            self.inner.kind().as_str(),
            yggdryl::graph::Element::get_crosscode(&self.inner),
        )
    }
}

element_getters!(JsMarketData);
market_getters!(JsMarketData);
common_verbs!(JsMarketData);

/// A stream of `MarketData`: the lazy row-decode walk
/// `MarketData.fromArrowReader` answers, and the sorted operations
/// `FixCodec.marketOperations` answers.
#[napi(js_name = "MarketDataRowIterator")]
pub struct JsMarketDataRowIterator {
    inner: Box<dyn FusedIterator<Item = yggdryl::Result<CoreMarketData>> + Send>,
}

impl JsMarketDataRowIterator {
    /// Wrap a stream the core built.
    pub(crate) fn over(
        inner: Box<dyn FusedIterator<Item = yggdryl::Result<CoreMarketData>> + Send>,
    ) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsMarketDataRowIterator {
    /// Advance the stream: the next value, or `null` at its end; an item the
    /// stream refuses throws, once - a decode walk ends there, the sorted
    /// operations continue past it. The loader wraps this into the iterator
    /// protocol.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<MarketData>")]
    pub fn next(&mut self) -> Result<Option<JsMarketData>> {
        self.inner
            .next()
            .transpose()
            .map(|data| data.map(JsMarketData::from_core))
            .map_err(napi_error)
    }
}

/// Streams items - any leaf or `MarketData`, pulled lazily from the
/// caller's iterable - into bounded `marketdata` record batches:
/// `graph.MarketData.arrowReader`'s native half, a module function so the
/// loader can delete it once captured.
#[napi(js_name = "_marketDataArrowReaderNative", skip_typescript)]
pub fn market_data_arrow_reader_native(
    env: Env,
    pull: Function<'_, (), Option<AnyMarketData<'static>>>,
    batch_row_size: Option<f64>,
    batch_byte_size: Option<f64>,
) -> Result<JsBatchReader> {
    let (batch_row_size, batch_byte_size) = batch_sizes(batch_row_size, batch_byte_size)?;
    let pulled = Pulled::new(env, pull)?;
    let failed = pulled.failed.clone();
    let items = pulled
        .map(|item| Ok(market_data_of(&item)))
        .chain(std::iter::from_fn(move || {
            failed.take().map(|error| Err(javascript_failure(error)))
        }));
    let reader =
        CoreMarketData::arrow_reader(items, batch_row_size, batch_byte_size).map_err(napi_error)?;
    Ok(JsBatchReader::from_core(reader, ROOT_NAME))
}

/// One named view over a `marketdata` stream: exactly
/// `MarketData.plan(view, lifts, crosscode)` applied to `reader`, bound once
/// against its schema - `graph.MarketData.applyView`'s native half, a
/// module function so the loader can delete it once captured. The view and
/// the lifts are read before the reader is taken; the reader is consumed.
#[napi(js_name = "_marketDataApplyViewNative", skip_typescript)]
pub fn market_data_apply_view_native(
    view: String,
    reader: &mut JsBatchReader,
    lifts: Option<Vec<Either<String, ClassInstance<'_, JsFieldPath>>>>,
    crosscode: Option<String>,
) -> Result<JsBatchReader> {
    let view = CoreMarketView::read(&view, crosscode.as_deref()).map_err(napi_error)?;
    let lifts = lifts_of(lifts)?;
    let root = reader.root_name().to_owned();
    let viewed = CoreMarketData::apply_view(&view, &lifts, reader.take()?).map_err(napi_error)?;
    Ok(JsBatchReader::from_core(viewed, &root))
}
