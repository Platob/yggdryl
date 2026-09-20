//! One fill: the price and quantity that traded, the execution's own
//! identifier, and the order it belongs to.

use crate::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
use crate::{Field, Result, Scalar};

use super::product::{Cells, MarketColumn, Product, finished};

/// What an execution answers beyond the market's facts: where the fill
/// left its order.
///
/// An execution's facts are exactly what [`MarketElement`] names - the
/// price and quantity that traded are `px` and `qty`, their worth
/// [`MarketElement::notional`] - and its state is the order's status after
/// the fill, which is what a report of a fill states; so everything here
/// is provided and reads that.
pub trait Execution: MarketEvent {
    /// Whether the fill left its order open: the order's status after it
    /// is still live.
    fn is_partial(&self) -> bool {
        self.get_state().is_live()
    }

    /// Whether the fill completed its order: the order's status after it
    /// is done.
    fn completes(&self) -> bool {
        self.get_state().is_done()
    }
}

/// One fill: what traded, at what price, against which order.
///
/// The chain is the order's: `crosscode` is the identifier of the order
/// the fill belongs to and `crossuuid` the identity it derives, so every
/// execution of one order stands in that order's chain and a fill joins
/// the order table on it. The execution's own identifier, the venue's
/// `ExecID`, is among the names it goes by, `identifiers`. `px` and `qty`
/// are the price and the quantity that traded, the side the order's, the
/// instrument under the codes the market named it by, and the market it
/// traded on its MIC.
///
/// Its facts are exactly what the traits name, so it holds a
/// [`MarketEventData`] and nothing beside: its identity is what its
/// content digests to, its sources the message it was read from, its chain
/// the walk's to fill, and its row the sixteen event columns then the
/// market's facts, [`ExecutionData::field`].
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Execution, ExecutionData, Product};
/// use yggdryl::{Currency, Decimal18, Side, State};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut fill = ExecutionData::at(1_700_000_000_000_000_000);
/// fill.set_crosscode("O-100".to_owned());
/// fill.set_identifiers(BTreeMap::from([("execid".to_owned(), "E-1".to_owned())]));
/// fill.set_px("82.5".parse()?);
/// fill.set_qty(Decimal18::from_int(300));
/// fill.set_side(Side::read("1")?);
/// fill.set_currency(Currency::new("USD")?);
/// fill.set_state(State::read("PartiallyFilled")?);
/// fill.finalize();
/// assert_eq!(fill.get_curruuid(), fill.time_uuid()?);
/// assert_eq!(fill.notional(), Some(Decimal18::from_int(24_750)));
/// assert!(fill.is_partial() && !fill.completes());
/// let field = ExecutionData::field()?;
/// assert_eq!(field.fields()[16].name(), "px");
/// let again = ExecutionData::from_row(&field, &fill.into_row()?)?;
/// assert_eq!(again, fill);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionData {
    event: MarketEventData,
}

impl ExecutionData {
    /// An execution at `unix`, nanoseconds since the Unix epoch, stating
    /// nothing else yet.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            event: MarketEventData::at(unix),
        }
    }

    /// The row every execution publishes: [`Product::row_field`].
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal, which the fixed columns never
    /// raise.
    pub fn field() -> Result<Field> {
        Self::default().row_field()
    }

    /// The event the execution is.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
        &self.event
    }

    /// The event, to write.
    pub(crate) const fn event_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }

    /// The identity settled from what the execution states.
    fn settle(&mut self) {
        self.fill_market();
        self.sync_cross();
        let state = self.digest_market_event();
        let code = finished(&state);
        self.finalized(code);
    }

    /// Nothing beside the market's facts to fold.
    #[allow(clippy::unused_self)]
    const fn merge_own(&mut self, _other: &Self, _later: bool) -> bool {
        false
    }
}

impl Default for ExecutionData {
    /// An execution at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

super::product::holds_market_event!(ExecutionData via event, event_mut);

impl Execution for ExecutionData {}

impl Product for ExecutionData {
    const NAME: &'static str = "execution";

    const DISPLAY: &'static str = "Execution";

    const DESCRIPTION: &'static str = "One fill: the price and quantity that traded, the execution's own identifier \
         among its names, and the order it belongs to as its chain.";

    const MARKET: &'static [MarketColumn] = &[
        MarketColumn::Px,
        MarketColumn::Qty,
        MarketColumn::Side,
        MarketColumn::Currency,
        MarketColumn::Unit,
        MarketColumn::SymbolTicker,
        MarketColumn::IsinCode,
        MarketColumn::CusipCode,
        MarketColumn::SedolCode,
        MarketColumn::BloombergCode,
        MarketColumn::CfiCode,
        MarketColumn::MicCode,
    ];

    const OWN: &'static [&'static str] = &[];

    fn own_fields(&self) -> Result<Vec<Field>> {
        Ok(Vec::new())
    }

    fn own_cells(&self, _cells: &mut Vec<Scalar>) -> Result<()> {
        Ok(())
    }

    fn from_cells(cells: &Cells<'_>) -> Result<Self> {
        let mut fill = Self::default();
        cells.record(&mut fill);
        Ok(fill)
    }
}
