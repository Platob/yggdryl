//! A price stated at an instant: one or two lanes, each a price, a size,
//! a currency and a unit, under the quote's own identifiers and its
//! validity.

use crate::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
use crate::{Decimal18, Field, Result, Scalar, Side};

use super::product::{Cells, MarketColumn, Product, finished};

/// What a quote answers beyond the market's facts: its lanes, read as
/// lanes.
///
/// A quote's facts are exactly the lanes [`MarketElement`] names, so
/// everything here is provided and reads what they imply: a lane quoted
/// is a price and a size both stated above zero - a zero price or size is
/// no quote on that side, which is FIX's own rule and what a withdrawn
/// lane looks like - and [`MarketElement::mid`] and
/// [`MarketElement::spread`] read across the two.
pub trait Quote: MarketEvent {
    /// The bid lane where it is quoted: what the quoter would pay, and for
    /// how much.
    fn bid(&self) -> Option<(Decimal18, Decimal18)> {
        lane(self.get_bidpx(), self.get_bidqty())
    }

    /// The ask lane where it is quoted: what the quoter would be paid, and
    /// for how much.
    fn ask(&self) -> Option<(Decimal18, Decimal18)> {
        lane(self.get_askpx(), self.get_askqty())
    }

    /// The lane `side` takes: the bid for a side that pays, the ask for
    /// one that is paid, nothing for a side that takes no lane - a cross,
    /// an unknown side.
    fn lane(&self, side: &Side) -> Option<(Decimal18, Decimal18)> {
        if side.is_bid() {
            self.bid()
        } else if side.is_ask() {
            self.ask()
        } else {
            None
        }
    }

    /// Whether both lanes are quoted.
    fn is_two_sided(&self) -> bool {
        self.bid().is_some() && self.ask().is_some()
    }
}

/// One lane where both its price and its size are stated above zero.
fn lane(px: Option<Decimal18>, qty: Option<Decimal18>) -> Option<(Decimal18, Decimal18)> {
    let px = px.filter(|held| *held > Decimal18::ZERO)?;
    let qty = qty.filter(|held| *held > Decimal18::ZERO)?;
    Some((px, qty))
}

/// One statement of a quote: what a party would pay and be paid at one
/// instant.
///
/// The chain is the quote's: `crosscode` is the identifier every
/// statement of one quote shares - the `QuoteID`, else the request it
/// answers - and `crossuuid` the identity it derives. The bid lane is what
/// the quoter would pay and the ask lane what it would be paid, each a
/// price, a size, a currency and a unit, one of them empty on a one-sided
/// quote; the instrument is under the codes the market named it by; and
/// how long the quote stands is its `expirunix`.
///
/// Its facts are exactly what the traits name, so it holds a
/// [`MarketEventData`] and nothing beside: its identity is what its
/// content digests to, its sources the messages it was read from, its
/// chain the walk's to fill, and its row the sixteen event columns then
/// the two lanes and the instrument, [`QuoteData::field`].
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Product, Quote, QuoteData};
/// use yggdryl::{Currency, Decimal18, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut quote = QuoteData::at(1_700_000_000_000_000_000);
/// quote.set_crosscode("Q-1".to_owned());
/// quote.set_bidpx(Some("82.4".parse()?));
/// quote.set_bidqty(Some(Decimal18::from_int(100)));
/// quote.set_askpx(Some("82.6".parse()?));
/// quote.set_askqty(Some(Decimal18::from_int(100)));
/// quote.set_bidcurrency(Some(Currency::new("USD")?));
/// quote.set_expirunix(Some(1_700_000_060_000_000_000));
/// quote.finalize();
/// assert_eq!(quote.get_curruuid(), quote.time_uuid()?);
/// // What the lanes imply.
/// assert!(quote.is_two_sided());
/// assert_eq!(quote.mid(), Some("82.5".parse()?));
/// assert_eq!(quote.spread(), Some("0.2".parse()?));
/// assert_eq!(quote.lane(&Side::read("Sell")?), Some(("82.6".parse()?, Decimal18::from_int(100))));
/// let field = QuoteData::field()?;
/// assert_eq!(field.fields()[16].name(), "bidpx");
/// let again = QuoteData::from_row(&field, &quote.into_row()?)?;
/// assert_eq!(again, quote);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteData {
    event: MarketEventData,
}

impl QuoteData {
    /// A quote stated at `unix`, nanoseconds since the Unix epoch, stating
    /// nothing else yet.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            event: MarketEventData::at(unix),
        }
    }

    /// The row every quote publishes: [`Product::row_field`].
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal, which the fixed columns never
    /// raise.
    pub fn field() -> Result<Field> {
        Self::default().row_field()
    }

    /// The event the quote is.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
        &self.event
    }

    /// The event, to write.
    pub(crate) const fn event_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }

    /// The identity settled from what the quote states.
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

impl Default for QuoteData {
    /// A quote at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

super::product::holds_market_event!(QuoteData via event, event_mut);

impl Quote for QuoteData {}

impl Product for QuoteData {
    const NAME: &'static str = "quote";

    const DISPLAY: &'static str = "Quote";

    const DESCRIPTION: &'static str = "One statement of a quote: one or two lanes, each a price, a size, a currency \
         and a unit, under the quote's own identifiers and its validity.";

    const MARKET: &'static [MarketColumn] = &[
        MarketColumn::BidPx,
        MarketColumn::BidQty,
        MarketColumn::BidCurrency,
        MarketColumn::BidUnit,
        MarketColumn::AskPx,
        MarketColumn::AskQty,
        MarketColumn::AskCurrency,
        MarketColumn::AskUnit,
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
        let mut quote = Self::default();
        cells.record(&mut quote);
        Ok(quote)
    }
}
