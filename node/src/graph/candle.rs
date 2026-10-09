//! Native Node.js view of [`CoreCandle`], [`CoreCandleOptions`] and the
//! [`CoreCandleIterator`] walk.
//!
//! A candle's readings cross the way a book's do: an instant as a `bigint`
//! of nanoseconds, a decimal as its text, a count as a `number`, a reading
//! the bucket never saw as `null`. Nothing here buckets, folds or reads a
//! cell: the walk is the core's, the row the core's field, the JSON the
//! core's codec under that field.

use napi::bindgen_prelude::{BigInt, ClassInstance, Either, Either4, Env, Function, Result};
use napi_derive::napi;
use yggdryl_market::graph::{
    BookEvent as CoreBookEvent, Candle as CoreCandle, CandleIterator as CoreCandleIterator,
    CandleOptions as CoreCandleOptions, Ohlc,
};

use super::{AnyMarketData, decimal_text, instant_of, market_data_of};
use crate::field::JsField;
use crate::text::codec::JsScalar;
use crate::timezone::{JsTimezone, TimezoneInput, timezone_from_input};
use crate::{Failed, Pulled, exact_f64, javascript_failure, napi_error};

/// One reading's open, high, low and close over a bucket, each as decimal
/// text: the plain object a candle's `bid`, `ask`, `mid` and `spread` are.
#[napi(object, object_from_js = false)]
pub struct CandleReading {
    /// The first value of the bucket.
    pub open: String,
    /// The greatest value of the bucket.
    pub high: String,
    /// The least value of the bucket.
    pub low: String,
    /// The last value of the bucket.
    pub close: String,
}

/// The reading `held` states, or `None` for a bucket that never saw one.
fn reading_of(held: Option<Ohlc>) -> Option<CandleReading> {
    held.map(|reading| CandleReading {
        open: reading.open.to_string(),
        high: reading.high.to_string(),
        low: reading.low.to_string(),
        close: reading.close.to_string(),
    })
}

/// One OHLC of one book over one bucket: what the books of one cross code
/// whose instants fell in `[start, end)` read at their best bid, their best
/// ask, their midpoint and their spread, the quantities resting at the
/// touch when the bucket closed, and how many books it folded. Built by
/// `CandleIterator`, or read back from a row through `fromScalar`.
#[napi(js_name = "Candle")]
#[derive(Clone)]
pub struct JsCandle {
    pub(crate) inner: CoreCandle,
}

impl JsCandle {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreCandle) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsCandle {
    /// The required struct `candle` every candle row is laid out under:
    /// `crosscode`, `ticker`, `start`, `end`, the four cells of each reading
    /// (`bidopen` .. `spreadclose`), `bidqty`, `askqty` and `books`.
    #[napi]
    pub fn field() -> Result<JsField> {
        CoreCandle::field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Read a candle back from the named struct `intoScalar` answers or the
    /// ordered row `Candle.field()` lays it out as; the loader widens any
    /// plain object through `Scalar.from` first.
    #[napi(factory, js_name = "_fromScalarNative", skip_typescript)]
    pub fn from_scalar_native(value: &JsScalar) -> Result<Self> {
        CoreCandle::from_scalar(&value.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The book's stored cross code, such as `3:0:ACME`.
    #[napi(getter)]
    pub fn crosscode(&self) -> String {
        self.inner.crosscode.to_string()
    }

    /// The book's ticker, where it stated one.
    #[napi(getter)]
    pub fn ticker(&self) -> Option<String> {
        self.inner.ticker.as_ref().map(ToString::to_string)
    }

    /// The bucket's start: nanoseconds since the epoch, UTC.
    #[napi(getter)]
    pub fn start(&self) -> BigInt {
        BigInt::from(self.inner.start)
    }

    /// The bucket's end, exclusive: nanoseconds since the epoch, UTC.
    #[napi(getter)]
    pub fn end(&self) -> BigInt {
        BigInt::from(self.inner.end)
    }

    /// The best bid's open, high, low and close over the books stating one;
    /// `null` where none did.
    #[napi(getter)]
    pub fn bid(&self) -> Option<CandleReading> {
        reading_of(self.inner.bid)
    }

    /// The best ask's open, high, low and close over the books stating one;
    /// `null` where none did.
    #[napi(getter)]
    pub fn ask(&self) -> Option<CandleReading> {
        reading_of(self.inner.ask)
    }

    /// The BBO midpoint's open, high, low and close over the books stating
    /// one; `null` where none did.
    #[napi(getter)]
    pub fn mid(&self) -> Option<CandleReading> {
        reading_of(self.inner.mid)
    }

    /// The spread's open, high, low and close over the books stating one;
    /// `null` where none did.
    #[napi(getter)]
    pub fn spread(&self) -> Option<CandleReading> {
        reading_of(self.inner.spread)
    }

    /// The last book's quantity at the best bid, as decimal text; `null`
    /// where it had none.
    #[napi(getter)]
    pub fn bidqty(&self) -> Option<String> {
        decimal_text(self.inner.bidqty)
    }

    /// The last book's quantity at the best ask, as decimal text; `null`
    /// where it had none.
    #[napi(getter)]
    pub fn askqty(&self) -> Option<String> {
        decimal_text(self.inner.askqty)
    }

    /// How many books folded into the bucket.
    #[napi(getter)]
    pub fn books(&self) -> Result<f64> {
        exact_f64(self.inner.books, "books")
    }

    /// The candle as the named struct of its cells - the flat row
    /// `Candle.field()` declares - an absent ticker, reading or quantity a
    /// null.
    #[napi]
    pub fn into_scalar(&self) -> JsScalar {
        JsScalar::from_core(self.inner.into_scalar())
    }

    /// Whether this candle states the same cells as `other`.
    #[napi]
    pub fn equals(&self, other: &JsCandle) -> bool {
        self.inner == other.inner
    }

    /// A cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// The candle's row as the plain JSON object the core's JSON codec
    /// writes it as under `Candle.field()`: the flat cells, the instants as
    /// ISO 8601 text and every decimal as text, so `JSON.stringify` is exact
    /// and `fromJSON` reads it back.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        let text = yggdryl::into_json_scalar(&self.inner.into_scalar()).map_err(napi_error)?;
        serde_json::from_str(&text).map_err(napi_error)
    }

    /// Rebuild a candle `toJSON` wrote: the object, or its text.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        let document = crate::json_document(value).map_err(napi_error)?;
        let text = serde_json::to_string(&document).map_err(napi_error)?;
        let field = CoreCandle::field().map_err(napi_error)?;
        let value = yggdryl::from_json_scalar_with_field(text, &field).map_err(napi_error)?;
        CoreCandle::from_scalar(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// `Candle(<crosscode>, start=<start>, end=<end>)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "Candle({:?}, start={}, end={})",
            self.inner.crosscode.as_str(),
            self.inner.start,
            self.inner.end
        )
    }
}

/// How instants are bucketed: the interval, spelled as a count and a unit
/// (`30s`, `1m`, `5m`, `1h`, `1d`, `1w`) or as a count of nanoseconds, and
/// the zone whose wall clock the buckets align to - UTC unless named - so a
/// daily candle opens at local midnight and hourly candles follow a
/// saving-time change.
#[napi(js_name = "CandleOptions")]
pub struct JsCandleOptions {
    pub(crate) inner: CoreCandleOptions,
}

impl Clone for JsCandleOptions {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl JsCandleOptions {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreCandleOptions) -> Self {
        Self { inner }
    }
}

/// A `CandleOptions`, or what one is built from: an interval spelling, or a
/// count of nanoseconds, aligned to UTC.
pub(crate) type CandleOptionsInput<'a> =
    Either4<ClassInstance<'a, JsCandleOptions>, String, BigInt, f64>;

/// The core options `value` names: the options themselves, or those an
/// interval spells, aligned to UTC, through the core's own two doors - a
/// count and a unit, or a `bigint` or whole `number` of nanoseconds read as
/// an instant is.
pub(crate) fn candle_options_from_input(
    value: CandleOptionsInput<'_>,
) -> Result<CoreCandleOptions> {
    let options = match value {
        Either4::A(options) => return Ok(options.inner.clone()),
        Either4::B(spelling) => CoreCandleOptions::from_spelling(&spelling),
        Either4::C(count) => CoreCandleOptions::new(instant_of(Either::A(count), "interval")?),
        Either4::D(count) => CoreCandleOptions::new(instant_of(Either::B(count), "interval")?),
    };
    options.map_err(napi_error)
}

#[napi]
impl JsCandleOptions {
    /// Buckets of `interval` - another `CandleOptions`, a spelling such as
    /// `'1m'`, or a `bigint` or whole `number` of nanoseconds - aligned to
    /// `timezone`'s wall clock where one is named, and otherwise to the
    /// given options' own zone, or UTC.
    #[napi(constructor)]
    // NAPI reads the parameter type syntactically for the declaration, so the
    // union is spelled here rather than through an alias.
    pub fn new(
        interval: Either4<ClassInstance<'_, JsCandleOptions>, String, BigInt, f64>,
        timezone: Option<TimezoneInput<'_>>,
    ) -> Result<Self> {
        let options = candle_options_from_input(interval)?;
        Ok(Self::from_core(match timezone {
            Some(timezone) => options.with_timezone(timezone_from_input(timezone)?),
            None => options,
        }))
    }

    /// The bucket width in nanoseconds of the zone's wall clock.
    #[napi(getter)]
    pub fn interval(&self) -> BigInt {
        BigInt::from(self.inner.interval())
    }

    /// The zone the buckets align to.
    #[napi(getter)]
    pub fn timezone(&self) -> JsTimezone {
        JsTimezone::from_core(*self.inner.timezone())
    }

    /// The interval as its count and the widest unit dividing it exactly:
    /// what the constructor reads back.
    #[napi(getter)]
    pub fn spelling(&self) -> String {
        self.inner.spelling()
    }

    /// These buckets aligned to another zone.
    #[napi]
    pub fn with_timezone(&self, timezone: TimezoneInput<'_>) -> Result<Self> {
        Ok(Self::from_core(
            self.inner
                .clone()
                .with_timezone(timezone_from_input(timezone)?),
        ))
    }

    /// Whether `other` buckets by the same interval in the same zone.
    #[napi]
    pub fn equals(&self, other: &JsCandleOptions) -> bool {
        self.inner == other.inner
    }

    /// A cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// The interval's spelling, then the zone where it is not UTC:
    /// `1h Europe/Zurich`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        let zone = self.inner.timezone();
        if zone.is_utc() {
            self.inner.spelling()
        } else {
            format!("{} {}", self.inner.spelling(), zone.as_str())
        }
    }
}

/// The core walk's source: the caller's books, a JavaScript failure crossing
/// as one typed sentinel.
type CandleSource = Box<dyn Iterator<Item = yggdryl::Result<CoreBookEvent>>>;

/// Candles from a sorted stream of books, one per cross code and bucket,
/// pulling the books lazily from the caller's iterable. Yields `Candle`;
/// a regression in the books' instants is refused at `$.book.transunix` and
/// ends the walk.
#[napi(js_name = "CandleIterator")]
pub struct JsCandleIterator {
    inner: CoreCandleIterator<CandleSource>,
    failed: Failed,
}

#[napi]
impl JsCandleIterator {
    /// Opens a candle walk over the items `pull` hands over - a `BookEvent`,
    /// or a `MarketData` or another leaf, each narrowed to the book it holds
    /// through the core's own `TryFrom`, which refuses any other kind by
    /// name - bucketed by `options`.
    #[napi(factory, js_name = "_candleIteratorNative", skip_typescript)]
    pub fn new_native(
        env: Env,
        pull: Function<'_, (), Option<AnyMarketData<'static>>>,
        options: CandleOptionsInput<'_>,
    ) -> Result<Self> {
        let options = candle_options_from_input(options)?;
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        // The core walk takes a typed error, not a native one, so a
        // JavaScript failure - which the loader throws on into the pull -
        // crosses it as one sentinel `Err`, and the walk drops the bucket it
        // cut short as it would on a refusal of its own. `failed` is peeked,
        // never taken, so it still holds the failure for `next` to throw
        // once the walk surfaces the sentinel in its place; the loader
        // throws the JavaScript original instead. Yielded exactly once:
        // `Chain` stops calling a side once it answers `None`.
        let source: CandleSource = {
            let sentinel_failed = failed.clone();
            let mut yielded = false;
            Box::new(
                pulled
                    .map(|item| CoreBookEvent::try_from(market_data_of(&item)))
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
        Ok(Self {
            inner: CoreCandleIterator::new(source, options),
            failed,
        })
    }

    /// The options the walk buckets by.
    #[napi(getter)]
    pub fn options(&self) -> JsCandleOptions {
        JsCandleOptions::from_core(self.inner.options().clone())
    }

    /// Advance the walk: the next candle, or `null` at its end. The loader
    /// wraps this into the iterator protocol.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<Candle>")]
    pub fn next(&mut self) -> Result<Option<JsCandle>> {
        match self.inner.next() {
            Some(Ok(candle)) => Ok(Some(JsCandle::from_core(candle))),
            // A core refusal (an unsorted book) or the sentinel standing in
            // for a JavaScript failure both land here; `self.failed` still
            // holds a genuine failure - never taken by the source, only
            // peeked - so it is thrown rather than the sentinel, and the
            // loader throws the JavaScript original in its place.
            Some(Err(error)) => Err(self.failed.take().unwrap_or_else(|| napi_error(error))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
