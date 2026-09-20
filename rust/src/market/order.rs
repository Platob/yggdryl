//! One order's life: the chain a venue's identifiers name, its state, the
//! quantity ordered against what filled and what is left, its price
//! ladder, its side, its instrument and how long it stands.

use crate::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
use crate::{DataType, Decimal18, Field, Result, Scalar};

use super::product::{Cells, MarketColumn, Product, column, feed, feed_decimal, finished};

/// What an order prices: at the market, at a limit, at a limit once a stop
/// triggers, or at the market once a stop triggers.
///
/// Read off the type an order states, as [`Order::pricing`] reads it, and
/// never stored: the stated fact is the type as the wire spelled it, and
/// this is what the four spellings a ladder cares about mean.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Pricing {
    /// At whatever the market offers: rests on no ladder.
    Market,
    /// At the limit price or better: what rests on a ladder.
    Limit,
    /// At the market once the stop price trades: rests nothing until the
    /// venue reports it triggered.
    Stop,
    /// At the limit once the stop price trades: likewise.
    StopLimit,
}

impl Pricing {
    /// The pricing one `OrdType(40)` code spells, where it spells one of
    /// the four - `1`, `2`, `3`, `4` - and nothing for another type, a
    /// pegged or a market-on-close order among them, which prices some
    /// other way.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim() {
            "1" => Some(Self::Market),
            "2" => Some(Self::Limit),
            "3" => Some(Self::Stop),
            "4" => Some(Self::StopLimit),
            _ => None,
        }
    }

    /// The `OrdType(40)` code that spells this pricing.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Market => "1",
            Self::Limit => "2",
            Self::Stop => "3",
            Self::StopLimit => "4",
        }
    }
}

/// What an order answers beyond the market's facts: the stop price and the
/// type it states, and what those imply with the rest.
///
/// A `get_`/`set_` pair reads and records a stated fact; a bare noun or an
/// `is_` predicate reads what the stated facts imply, on every call, and
/// stores nothing. The limit is [`MarketElement::get_px`], what was ordered
/// [`MarketElement::get_qty`], what filled [`MarketElement::get_cumqty`] at
/// [`MarketElement::get_avgpx`], what is left [`MarketElement::get_leavesqty`],
/// and whether the order can still change is [`Event::is_alive`].
pub trait Order: MarketEvent {
    /// The stop price, where the order states one.
    fn get_stoppx(&self) -> Option<Decimal18>;

    /// Records the stop price; `None` states none.
    fn set_stoppx(&mut self, stoppx: Option<Decimal18>);

    /// The type the order states, as the wire spells it - FIX's
    /// `OrdType(40)`, `1` for a market order, `2` for a limit - where it
    /// states one.
    fn get_ordtype(&self) -> Option<&str>;

    /// Records the type; `None` states none.
    fn set_ordtype(&mut self, ordtype: Option<String>);

    /// What the order prices: the type it states, read as [`Pricing`]
    /// reads a code, nothing where it states a type outside the four;
    /// where it states none, what the limit and the stop imply - a limit
    /// stated alone is a limit order, a stop alone a stop order, both a
    /// stop limit, neither a market order. A limit of zero is no limit.
    fn pricing(&self) -> Option<Pricing> {
        if let Some(code) = self.get_ordtype() {
            return Pricing::from_code(code);
        }
        let limit = self.get_px() > Decimal18::ZERO;
        let stop = self
            .get_stoppx()
            .is_some_and(|stoppx| stoppx > Decimal18::ZERO);
        Some(match (limit, stop) {
            (true, false) => Pricing::Limit,
            (false, true) => Pricing::Stop,
            (true, true) => Pricing::StopLimit,
            (false, false) => Pricing::Market,
        })
    }

    /// What is left to trade: what the order states is left; else what it
    /// ordered less what filled, floored at zero, where it states what
    /// filled and ordered something; else what it ordered.
    fn remaining(&self) -> Decimal18 {
        if let Some(leaves) = self.get_leavesqty() {
            return leaves;
        }
        let qty = self.get_qty();
        match self.get_cumqty() {
            Some(done) if !qty.is_zero() => qty
                .checked_sub(done)
                .unwrap_or(Decimal18::ZERO)
                .max(Decimal18::ZERO),
            _ => qty,
        }
    }

    /// What traded: what the order states filled; else what it ordered
    /// less what is left, floored at zero, where it states what is left
    /// and ordered something; else nothing.
    fn filled(&self) -> Decimal18 {
        if let Some(done) = self.get_cumqty() {
            return done;
        }
        let qty = self.get_qty();
        match self.get_leavesqty() {
            Some(leaves) if !qty.is_zero() => qty
                .checked_sub(leaves)
                .unwrap_or(Decimal18::ZERO)
                .max(Decimal18::ZERO),
            _ => Decimal18::ZERO,
        }
    }

    /// How much of what was ordered traded, `filled / qty` at eighteen
    /// places; nothing where nothing was ordered.
    fn filled_ratio(&self) -> Option<Decimal18> {
        let qty = self.get_qty();
        if qty.is_zero() {
            return None;
        }
        self.filled().checked_div(qty)
    }

    /// Whether the order rests on a ladder right now: still alive, on a
    /// side that takes a lane, priced at a limit above zero, with
    /// something left. A market order, a stop order until it triggers, a
    /// cross, and a filled, cancelled, rejected or expired order rest
    /// nothing.
    fn is_resting(&self) -> bool {
        let side = self.get_side();
        self.is_alive()
            && (side.is_bid() || side.is_ask())
            && self.pricing() == Some(Pricing::Limit)
            && self.get_px() > Decimal18::ZERO
            && self.remaining() > Decimal18::ZERO
    }
}

/// One statement of an order: what the market said the order was at one
/// instant.
///
/// The chain is the order's: `crosscode` is the identifier every
/// statement of one order shares - the venue's `OrderID`, else the
/// client's identifier the statement is about - and `crossuuid` the
/// identity it derives, so an order's placement, its amendments and every
/// report against it stand in one chain whichever identifier each
/// spelled. The state is where the order stands, `qty` what was ordered
/// against `cumqty` what filled and `leavesqty` what is left, the price
/// ladder `px` (the limit), `stoppx` (the stop) and `avgpx` (what filled
/// averaged), the type it states, the side, the instrument under the
/// codes the market named it by, and how long it stands as its time in
/// force and `expirunix`.
///
/// An order is a value of this crate: its identity is what its content
/// digests to, settled by [`Element::finalize`], so two statements of one
/// order are one identity; its sources are the messages it was read from;
/// its chain - predecessor, place, parents - is the walk's to fill, and a
/// statement that follows another takes the order's terms the predecessor
/// stated where it restates none - the limit, the quantity, the stop, the
/// type, what filled so far and at what average - because what an order
/// was placed as and what filled do not vanish when a report is silent
/// about them; and its row opens with the sixteen event columns then
/// states the market's facts, the stop price and the type,
/// [`OrderData::field`].
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Order, OrderData, Pricing, Product};
/// use yggdryl::{Currency, Decimal18, Side, State};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut order = OrderData::at(1_700_000_000_000_000_000);
/// order.set_crosscode("O-100".to_owned());
/// order.set_state(State::read("New")?);
/// order.set_px("82.5".parse()?);
/// order.set_stoppx(Some("80".parse()?));
/// order.set_qty(Decimal18::from_int(1_000));
/// order.set_cumqty(Some(Decimal18::from_int(250)));
/// order.set_side(Side::read("1")?);
/// order.set_currency(Currency::new("USD")?);
/// order.finalize();
/// assert_eq!(order.get_curruuid(), order.time_uuid()?);
/// assert_ne!(order.get_crossuuid(), order.get_curruuid(), "the order's chain");
/// // What the stated facts imply.
/// assert_eq!(order.pricing(), Some(Pricing::StopLimit));
/// assert_eq!(order.remaining(), Decimal18::from_int(750));
/// assert_eq!(order.filled_ratio(), Some("0.25".parse()?));
/// assert!(!order.is_resting(), "a stop limit rests nothing until it triggers");
/// order.set_ordtype(Some("2".to_owned()));
/// assert!(order.is_resting(), "stated a limit order, it rests");
/// // The row: the sixteen event columns, then the order's own.
/// let field = OrderData::field()?;
/// assert_eq!(field.fields()[0].name(), "currunix");
/// assert_eq!(field.fields()[16].name(), "px");
/// assert_eq!(field.fields().last().map(yggdryl::Field::name), Some("ordtype"));
/// order.finalize();
/// let row = order.into_row()?;
/// let again = OrderData::from_row(&field, &row)?;
/// assert_eq!(again, order);
/// // A statement of the same order with a different stop is another identity.
/// let mut moved = order.clone();
/// moved.set_stoppx(Some("79".parse()?));
/// moved.finalize();
/// assert_ne!(moved.get_curruuid(), order.get_curruuid());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct OrderData {
    event: MarketEventData,
    stoppx: Option<Decimal18>,
    ordtype: Option<String>,
}

impl OrderData {
    /// An order stated at `unix`, nanoseconds since the Unix epoch, stating
    /// nothing else yet: the identity is what [`Element::finalize`] derives
    /// once the facts are in.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            event: MarketEventData::at(unix),
            stoppx: None,
            ordtype: None,
        }
    }

    /// The row every order publishes: [`Product::row_field`], which is one
    /// row for every order.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal, which the fixed columns never
    /// raise.
    pub fn field() -> Result<Field> {
        Self::default().row_field()
    }

    /// The event the order is: every fact the three graph traits answer,
    /// held as fields.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
        &self.event
    }

    /// The event, to write.
    pub(crate) const fn event_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }

    /// The identity settled from what the order states: the market's facts,
    /// the stop price and the type.
    fn settle(&mut self) {
        self.fill_market();
        self.sync_cross();
        let mut state = self.digest_market_event();
        feed_decimal(&mut state, "stoppx", self.stoppx);
        if let Some(ordtype) = &self.ordtype {
            feed(&mut state, "ordtype", ordtype.as_bytes());
        }
        let code = finished(&state);
        self.finalized(code);
    }

    /// The stop price and the type the later statement states, else this
    /// one's.
    fn merge_own(&mut self, other: &Self, later: bool) -> bool {
        let mut changed = false;
        let stoppx = if later {
            other.stoppx.or(self.stoppx)
        } else {
            self.stoppx.or(other.stoppx)
        };
        if stoppx != self.stoppx {
            self.stoppx = stoppx;
            changed = true;
        }
        let ordtype = if later {
            other.ordtype.as_ref().or(self.ordtype.as_ref())
        } else {
            self.ordtype.as_ref().or(other.ordtype.as_ref())
        }
        .cloned();
        if ordtype != self.ordtype {
            self.ordtype = ordtype;
            changed = true;
        }
        changed
    }

    /// The timed market reading, then the order's terms the predecessor
    /// stated where this statement restates none: the limit and the
    /// quantity where this one states zero, the stop, the type, what
    /// filled and at what average where it states none, and what is left
    /// only where it restated no quantity either, because what is left
    /// depends on what was ordered.
    fn following(self, previous: &Self) -> Option<Self> {
        let mut this = MarketEvent::following_market(self, previous)?;
        let mut changed = false;
        let restated_qty = !this.get_qty().is_zero();
        if this.get_px().is_zero() && !previous.get_px().is_zero() {
            this.set_px(previous.get_px());
            changed = true;
        }
        if !restated_qty && !previous.get_qty().is_zero() {
            this.set_qty(previous.get_qty());
            changed = true;
        }
        if this.stoppx.is_none() && previous.stoppx.is_some() {
            this.stoppx = previous.stoppx;
            changed = true;
        }
        if this.ordtype.is_none() && previous.ordtype.is_some() {
            this.ordtype.clone_from(&previous.ordtype);
            changed = true;
        }
        if this.get_cumqty().is_none() && previous.get_cumqty().is_some() {
            this.set_cumqty(previous.get_cumqty());
            changed = true;
        }
        if this.get_avgpx().is_none() && previous.get_avgpx().is_some() {
            this.set_avgpx(previous.get_avgpx());
            changed = true;
        }
        if !restated_qty && this.get_leavesqty().is_none() && previous.get_leavesqty().is_some() {
            this.set_leavesqty(previous.get_leavesqty());
            changed = true;
        }
        if changed {
            this.finalize();
        }
        Some(this)
    }
}

impl Default for OrderData {
    /// An order at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

super::product::holds_market_event!(OrderData via event, event_mut, with_previous = Self::following);

impl Order for OrderData {
    fn get_stoppx(&self) -> Option<Decimal18> {
        self.stoppx
    }

    fn set_stoppx(&mut self, stoppx: Option<Decimal18>) {
        self.stoppx = stoppx;
    }

    fn get_ordtype(&self) -> Option<&str> {
        self.ordtype.as_deref()
    }

    fn set_ordtype(&mut self, ordtype: Option<String>) {
        self.ordtype = ordtype.filter(|held| !held.is_empty());
    }
}

impl Product for OrderData {
    const NAME: &'static str = "order";

    const DISPLAY: &'static str = "Order";

    const DESCRIPTION: &'static str = "One statement of an order: the chain the venue's identifiers name, its state, \
         the quantity ordered against what filled and what is left, its price ladder, its side, \
         its instrument and how long it stands.";

    const MARKET: &'static [MarketColumn] = &[
        MarketColumn::Px,
        MarketColumn::AvgPx,
        MarketColumn::Qty,
        MarketColumn::CumQty,
        MarketColumn::LeavesQty,
        MarketColumn::Side,
        MarketColumn::Currency,
        MarketColumn::Unit,
        MarketColumn::Tif,
        MarketColumn::Tradable,
        MarketColumn::SymbolTicker,
        MarketColumn::IsinCode,
        MarketColumn::CusipCode,
        MarketColumn::SedolCode,
        MarketColumn::BloombergCode,
        MarketColumn::CfiCode,
        MarketColumn::MicCode,
    ];

    const OWN: &'static [&'static str] = &["stoppx", "ordtype"];

    fn own_fields(&self) -> Result<Vec<Field>> {
        Ok(vec![
            column(
                "stoppx",
                "StopPx",
                "The stop price, exact; empty where the order states none.",
                Decimal18::dtype(),
                true,
            )?,
            column(
                "ordtype",
                "OrdType",
                "The type the order states, as the wire spells it - `1` market, `2` limit, `3` \
                 stop, `4` stop limit; empty where it states none.",
                DataType::utf8(),
                true,
            )?,
        ])
    }

    fn own_cells(&self, cells: &mut Vec<Scalar>) -> Result<()> {
        cells.push(self.stoppx.map_or(Scalar::Null, Scalar::from));
        cells.push(self.ordtype.as_deref().map_or(Scalar::Null, Scalar::from));
        Ok(())
    }

    fn from_cells(cells: &Cells<'_>) -> Result<Self> {
        let mut order = Self::default();
        cells.record(&mut order);
        order.stoppx = Decimal18::from_scalar(cells.own(0));
        order.set_ordtype(cells.own(1).as_str().map(str::to_owned));
        Ok(order)
    }
}
