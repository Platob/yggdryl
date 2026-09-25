//! Native Node.js view of the [`CoreEventIterator`] walk over
//! [`CoreMarketData`].

use napi::bindgen_prelude::{BigInt, Either, Env, Function, Result};
use napi_derive::napi;
use yggdryl::graph::{EventIterator as CoreEventIterator, MarketData as CoreMarketData};

use super::market_data::JsMarketData;
use super::{AnyMarketData, instant_of, market_data_of};
use crate::{Failed, Pulled};

/// The values a walk reads, once each pulled item is unwrapped to the
/// native value it holds.
type Source =
    std::iter::Map<Pulled<AnyMarketData<'static>>, fn(AnyMarketData<'static>) -> CoreMarketData>;

/// Unwrap one pulled item to the native value it holds, the `fn` item
/// [`Source`] maps with.
fn unwrap(item: AnyMarketData<'static>) -> CoreMarketData {
    market_data_of(&item)
}

/// A walk that chains each operation event to the live element it follows
/// and yields it enriched, pulling its items lazily from the caller's
/// iterable: any leaf or `MarketData`, the dated operations and trades
/// walking and every other variant yielded unchanged, in place. Yields
/// `MarketData`.
#[napi(js_name = "EventIterator")]
pub struct JsEventIterator {
    inner: CoreEventIterator<CoreMarketData, Source>,
    failed: Failed,
}

#[napi]
impl JsEventIterator {
    /// Opens a walk over the items `pull` hands over. `sorted` states that
    /// they arrive in their own order already; where they do not, the walk
    /// collects and sorts them first - which pulls the whole source right
    /// here, so a failure partway through it is thrown now rather than once
    /// the sorted prefix is exhausted. `snapshotNs`, given, is the grid step
    /// in nanoseconds the walk also yields living-identity snapshots at.
    #[napi(factory, js_name = "_eventIteratorNative", skip_typescript)]
    pub fn new_native(
        env: Env,
        pull: Function<'_, (), Option<AnyMarketData<'static>>>,
        sorted: bool,
        snapshot_ns: Option<Either<BigInt, f64>>,
    ) -> Result<Self> {
        let snapshot_ns = snapshot_ns
            .map(|value| instant_of(value, "snapshotNs"))
            .transpose()?;
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let source: Source = pulled.map(unwrap as _);
        let mut inner = CoreEventIterator::new(source, sorted);
        if let Some(snapshot_ns) = snapshot_ns {
            inner = inner.with_snapshot_ns(snapshot_ns);
        }
        if !sorted && let Some(error) = failed.take() {
            return Err(error);
        }
        Ok(Self { inner, failed })
    }

    /// The grid step in nanoseconds the walk reads snapshots at, or `null`.
    #[napi(getter)]
    pub fn snapshot_ns(&self) -> Option<BigInt> {
        self.inner.snapshot_ns().map(BigInt::from)
    }

    /// The elements still alive after what the walk has read so far, one
    /// per identity, in no order.
    #[napi]
    pub fn alive(&self) -> Vec<JsMarketData> {
        self.inner
            .alive()
            .cloned()
            .map(JsMarketData::from_core)
            .collect()
    }

    /// Advance the walk: the next value, or `null` at its end. The loader
    /// wraps this into the iterator protocol.
    ///
    /// A failure behind the caller's iterable throws once, in place of the
    /// end it would otherwise answer.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<MarketData>")]
    pub fn next(&mut self) -> Result<Option<JsMarketData>> {
        match self.inner.next() {
            Some(data) => Ok(Some(JsMarketData::from_core(data))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
