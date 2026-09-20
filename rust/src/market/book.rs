//! The ladder for one instrument at one instant: levels of price, size
//! and order count per side, to a declared depth, with what printed
//! against it and what the ladders imply.

use std::num::NonZeroU32;

use crate::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
use crate::{DataType, Decimal18, Error, Field, Result, Scalar, Side, StructType};

use super::product::{Cells, MarketColumn, Product, column, feed, finished};

/// One level of a ladder: the size resting at one price, and how many
/// orders and quote lanes rest there.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Level {
    /// The price of the level.
    pub px: Decimal18,
    /// The size resting at it: what the orders and quote lanes there have
    /// left, summed.
    pub qty: Decimal18,
    /// How many orders and quote lanes rest at it.
    pub count: u64,
}

/// What a book answers beyond the market's facts: its two ladders, how
/// many statements made it, and what the ladders imply.
///
/// A `get_`/`set_` pair reads and records a stated fact; a bare noun or an
/// `is_` predicate reads what the ladders imply, on every call, and stores
/// nothing. The top of each ladder is the lane [`MarketElement`] answers -
/// a book's `bidpx` is its best bid - so [`MarketElement::mid`] and
/// [`MarketElement::spread`] read across the tops; `px` is the mid, `qty`
/// the size resting on both ladders. The facts that need history are
/// stated once, by whoever read the book, as market facts of their own:
/// the last print is [`MarketElement::get_lastpx`] and
/// [`MarketElement::get_lastqty`], the volume since the chain began
/// [`MarketElement::get_cumqty`] and its average price
/// [`MarketElement::get_avgpx`], the previous book's mid and resting size
/// [`MarketElement::get_prevpx`] and [`MarketElement::get_prevqty`].
///
/// A missing top is nothing on every reading that divides by it; nothing
/// is ever padded with a sentinel price; a locked or crossed book is
/// stated, never refused, and read by [`Self::is_locked`] and
/// [`Self::is_crossed`]. In the formulas, `b` and `a` are the top prices
/// and `Qb` and `Qa` the top sizes.
pub trait Book: MarketEvent {
    /// How many levels per side the book was declared to hold.
    fn get_depth(&self) -> NonZeroU32;

    /// The bid ladder, best first: the highest price leading.
    fn get_bids(&self) -> &[Level];

    /// The ask ladder, best first: the lowest price leading.
    fn get_asks(&self) -> &[Level];

    /// Records the bid ladder: the levels sorted best first, one per
    /// price, at most the declared depth.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming `bids` for more levels than
    /// the depth or two levels at one price; the ladder is unchanged.
    fn set_bids(&mut self, levels: Vec<Level>) -> Result<()>;

    /// Records the ask ladder, likewise, naming `asks`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming `asks` for more levels than
    /// the depth or two levels at one price; the ladder is unchanged.
    fn set_asks(&mut self, levels: Vec<Level>) -> Result<()>;

    /// How many statements have been applied to the book's chain since it
    /// began - every order, quote and print, a print that moved nothing
    /// among them - as whoever read the book counted them.
    fn get_updates(&self) -> u64;

    /// Records the count.
    fn set_updates(&mut self, updates: u64);

    /// The top of the bid ladder.
    fn best_bid(&self) -> Option<&Level> {
        self.get_bids().first()
    }

    /// The top of the ask ladder.
    fn best_ask(&self) -> Option<&Level> {
        self.get_asks().first()
    }

    /// The level at `index` of the ladder `side` takes, the top at zero;
    /// nothing past the ladder, or for a side that takes no lane.
    fn level(&self, side: &Side, index: usize) -> Option<&Level> {
        if side.is_bid() {
            self.get_bids().get(index)
        } else if side.is_ask() {
            self.get_asks().get(index)
        } else {
            None
        }
    }

    /// Whether both ladders rest something.
    fn is_two_sided(&self) -> bool {
        self.best_bid().is_some() && self.best_ask().is_some()
    }

    /// Whether the two tops are at one price: a spread of zero.
    fn is_locked(&self) -> bool {
        self.spread().is_some_and(|spread| spread.is_zero())
    }

    /// Whether the best bid is above the best ask: a negative spread.
    fn is_crossed(&self) -> bool {
        self.spread().is_some_and(|spread| spread.is_negative())
    }

    /// The spread as a share of the mid, `spread * 10 000 / mid`, each
    /// step at eighteen places truncated toward zero; nothing where there
    /// is no mid or it is zero.
    fn spread_bps(&self) -> Option<Decimal18> {
        let mid = self.mid().filter(|mid| !mid.is_zero())?;
        self.spread()?
            .checked_mul(Decimal18::from_int(10_000))?
            .checked_div(mid)
    }

    /// The size-weighted mid, `(b * Qa + a * Qb) / (Qb + Qa)`: the mid
    /// pulled toward the side with less resting on it, which is where the
    /// price is likelier to go; nothing where either top is missing.
    fn microprice(&self) -> Option<Decimal18> {
        let bid = self.best_bid()?;
        let ask = self.best_ask()?;
        let total = bid
            .qty
            .checked_add(ask.qty)
            .filter(|total| !total.is_zero())?;
        bid.px
            .checked_mul(ask.qty)?
            .checked_add(ask.px.checked_mul(bid.qty)?)?
            .checked_div(total)
    }

    /// The imbalance at the tops, `(Qb - Qa) / (Qb + Qa)`, in `[-1, 1]`:
    /// a missing top counts as no size, so a one-sided book answers one
    /// or minus one; nothing where both are missing.
    fn imbalance(&self) -> Option<Decimal18> {
        imbalance_of(
            self.best_bid().map_or(Decimal18::ZERO, |level| level.qty),
            self.best_ask().map_or(Decimal18::ZERO, |level| level.qty),
        )
    }

    /// The same imbalance over the sizes summed across the first `levels`
    /// of each ladder, a ladder shorter than that contributing what it
    /// has; nothing where both sums are zero, which no levels always is.
    fn imbalance_to_depth(&self, levels: usize) -> Option<Decimal18> {
        imbalance_of(
            size_of(self.get_bids(), levels),
            size_of(self.get_asks(), levels),
        )
    }

    /// The size resting on the bid ladder, summed, saturating at the
    /// precision.
    fn bid_size(&self) -> Decimal18 {
        size_of(self.get_bids(), usize::MAX)
    }

    /// The size resting on the ask ladder, summed, saturating.
    fn ask_size(&self) -> Decimal18 {
        size_of(self.get_asks(), usize::MAX)
    }

    /// The orders and quote lanes resting on the bid ladder, summed,
    /// saturating.
    fn bid_count(&self) -> u64 {
        self.get_bids()
            .iter()
            .fold(0, |count, level| count.saturating_add(level.count))
    }

    /// The orders and quote lanes resting on the ask ladder, summed,
    /// saturating.
    fn ask_count(&self) -> u64 {
        self.get_asks()
            .iter()
            .fold(0, |count, level| count.saturating_add(level.count))
    }
}

/// The size resting on the first `levels` of a ladder, summed, saturating
/// at the precision.
fn size_of(ladder: &[Level], levels: usize) -> Decimal18 {
    ladder
        .iter()
        .take(levels)
        .fold(Decimal18::ZERO, |size, level| {
            size.checked_add(level.qty).unwrap_or(Decimal18::MAX)
        })
}

/// `(bid - ask) / (bid + ask)`, nothing where both are zero.
fn imbalance_of(bid: Decimal18, ask: Decimal18) -> Option<Decimal18> {
    let total = bid.checked_add(ask).filter(|total| !total.is_zero())?;
    bid.checked_sub(ask)?.checked_div(total)
}

/// The ladder for one instrument at one instant, to a declared depth.
///
/// The chain is the instrument's: `crosscode` is the symbol the book was
/// read under - the [`Symbol`](super::Symbol) its makers named, or the
/// global one - and `crossuuid` the identity it derives, so every book of
/// one symbol stands in one chain. The chain is flat: a book follows the
/// one before it and descends from it alone, never from the whole line,
/// because a stream reading one book per instant would otherwise carry a
/// lineage as long as itself in every row; its `prevpx` and `prevqty` are
/// the previous book's mid and resting size. The instant is `currunix`,
/// when the ladder was read, and `snapunix` the grid step it closed,
/// where a grid was declared. The two ladders are the bids, best first,
/// and the asks, best first, each a [`Level`] per price to the depth
/// declared when the book was opened - a ladder that grew to whatever
/// arrived would be unbounded, which is the failure a bounded row exists
/// to avoid - and the top of each is the lane the traits answer. The
/// instrument is under the codes the market named it by, with its
/// currency and unit, and whether it can trade is the venue's word,
/// folded from the makers, never the ladders'.
///
/// A book is a value of this crate: its identity is what its content
/// digests to - the two ladders and the count among it - its sources the
/// statements it was read at, its chain the reader's to fill, and its row
/// the sixteen event columns, the prints, the instrument, then the two
/// ladders and the count, [`BookData::field`]. Each ladder is a fixed-size
/// list at the declared depth, one nullable level per slot, chosen over a
/// variable list because a depth-ten book is then a fixed-width row: every
/// slot of every row sits at an offset the depth decides, so a scan reads
/// the best bid of a million books without an offsets pass, and a level
/// the ladder does not reach is a null in its slot rather than a shorter
/// row. The mid, the resting size, the lanes and every other reading are
/// the ladders' and are re-derived from the row's, so no column restates
/// another.
///
/// ```
/// use std::num::NonZeroU32;
///
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Book, BookData, Level, Product};
/// use yggdryl::{Currency, Decimal18, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let level = |px: &str, qty: i64, count: u64| -> yggdryl::Result<Level> {
///     Ok(Level { px: px.parse()?, qty: Decimal18::from_int(qty), count })
/// };
/// let depth = NonZeroU32::new(3).expect("three");
/// let mut book = BookData::at(1_700_000_000_000_000_000, depth);
/// book.set_crosscode("US0378331005".to_owned());
/// book.set_currency(Currency::new("USD")?);
/// book.set_bids(vec![level("82.4", 100, 1)?, level("82.5", 150, 2)?])?;
/// book.set_asks(vec![level("82.6", 40, 1)?, level("82.7", 80, 1)?])?;
/// book.finalize();
/// // The tops are the lanes, and the readings are the ladders'.
/// assert_eq!(book.get_bids()[0].px, "82.5".parse()?, "sorted best first");
/// assert_eq!((book.get_bidpx(), book.get_askqty()), (Some("82.5".parse()?), Some(Decimal18::from_int(40))));
/// assert_eq!(book.get_px(), "82.55".parse()?, "the mid");
/// assert_eq!(book.get_qty(), Decimal18::from_int(370), "resting on both ladders");
/// assert_eq!(book.spread(), Some("0.1".parse()?));
/// assert_eq!(book.imbalance(), Some("0.578947368421052631".parse()?));
/// assert_eq!(book.level(&Side::read("Buy")?, 1).map(|level| level.qty), Some(Decimal18::from_int(100)));
/// assert!(book.is_two_sided() && !book.is_locked());
/// // The row is fixed-width: three slots a side, the empty ones null.
/// let field = BookData::field(depth)?;
/// assert_eq!(field.fields().last().map(yggdryl::Field::name), Some("updates"));
/// let again = BookData::from_row(&field, &book.into_row()?)?;
/// assert_eq!(again, book);
/// // A deeper ladder than declared is refused, naming the side.
/// let refused = book.set_asks(vec![level("1", 1, 1)?, level("2", 1, 1)?, level("3", 1, 1)?, level("4", 1, 1)?]);
/// assert!(refused.unwrap_err().to_string().contains("asks"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct BookData {
    event: MarketEventData,
    depth: NonZeroU32,
    bids: Vec<Level>,
    asks: Vec<Level>,
    updates: u64,
}

impl BookData {
    /// A book read at `unix`, nanoseconds since the Unix epoch, to `depth`
    /// levels per side, stating no level yet.
    #[must_use]
    pub fn at(unix: i64, depth: NonZeroU32) -> Self {
        Self {
            event: MarketEventData::at(unix),
            depth,
            bids: Vec::new(),
            asks: Vec::new(),
            updates: 0,
        }
    }

    /// The row every book of `depth` levels per side publishes:
    /// [`Product::row_field`] at that depth.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a depth a fixed-size list
    /// cannot hold.
    pub fn field(depth: NonZeroU32) -> Result<Field> {
        Self::at(0, depth).row_field()
    }

    /// The event the book is.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
        &self.event
    }

    /// The event, to write.
    pub(crate) const fn event_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }

    /// Brings the facts the ladders imply in step with them: the lanes are
    /// the tops, the price the mid - nothing where there is none - and the
    /// quantity what rests on both ladders; derived and never stated
    /// twice, overwriting whatever was set.
    fn fill_from_ladders(&mut self) {
        let top = |ladder: &[Level]| ladder.first().map(|level| (level.px, level.qty));
        let (bidpx, bidqty) = top(&self.bids).unzip();
        let (askpx, askqty) = top(&self.asks).unzip();
        self.set_bidpx(bidpx);
        self.set_bidqty(bidqty);
        self.set_askpx(askpx);
        self.set_askqty(askqty);
        let mid = self.mid().unwrap_or(Decimal18::ZERO);
        self.set_px(mid);
        let resting = self
            .bid_size()
            .checked_add(self.ask_size())
            .unwrap_or(Decimal18::MAX);
        self.set_qty(resting);
    }

    /// The identity settled from what the book states: what the ladders
    /// imply brought in step, then the market's facts, the depth, every
    /// level and the count digested.
    fn settle(&mut self) {
        self.fill_from_ladders();
        self.sync_cross();
        let mut state = self.digest_market_event();
        feed(&mut state, "depth", &self.depth.get().to_le_bytes());
        for (name, ladder) in [("bid", &self.bids), ("ask", &self.asks)] {
            for level in ladder {
                feed(&mut state, name, &level.px.units().to_le_bytes());
                feed(&mut state, name, &level.qty.units().to_le_bytes());
                feed(&mut state, name, &level.count.to_le_bytes());
            }
        }
        feed(&mut state, "updates", &self.updates.to_le_bytes());
        let code = finished(&state);
        self.finalized(code);
    }

    /// The ladders and the count of the later statement, where it is the
    /// later one.
    fn merge_own(&mut self, other: &Self, later: bool) -> bool {
        if !later
            || (other.bids == self.bids && other.asks == self.asks && other.updates == self.updates)
        {
            return false;
        }
        self.bids.clone_from(&other.bids);
        self.asks.clone_from(&other.asks);
        self.updates = other.updates;
        true
    }

    /// The flat following: the predecessor's identity and instant, the
    /// next place in the chain, the predecessor alone as the parent, its
    /// cross code where this one's differs, and its mid and resting size
    /// as the step before this book; nothing where it cannot follow - its
    /// own predecessor, or one that happened after it. What the chain is
    /// about is not taken: a book states every fact it knows on every
    /// statement, so its silence is disagreement among its makers, never
    /// something the book before it should fill.
    fn following(mut self, previous: &Self) -> Option<Self> {
        if previous.get_curruuid() == self.get_curruuid()
            || previous.get_currunix() > self.get_currunix()
        {
            return None;
        }
        self.set_prevuuid(Some(previous.get_curruuid()));
        self.set_prevunix(Some(previous.get_currunix()));
        self.set_seqnum(previous.get_seqnum().saturating_add(1));
        self.set_parentuuids(vec![previous.get_curruuid()]);
        if !previous.get_crosscode().is_empty() && self.get_crosscode() != previous.get_crosscode()
        {
            self.set_crosscode(previous.get_crosscode().to_owned());
        }
        if self.get_prevpx().is_none() && !previous.get_px().is_zero() {
            self.set_prevpx(Some(previous.get_px()));
        }
        if self.get_prevqty().is_none() && !previous.get_qty().is_zero() {
            self.set_prevqty(Some(previous.get_qty()));
        }
        self.finalize();
        Some(self)
    }
}

impl Default for BookData {
    /// A book at the epoch, one level deep, stating nothing.
    fn default() -> Self {
        Self::at(0, NonZeroU32::MIN)
    }
}

/// One ladder validated: sorted best first by `best`, one level per price,
/// at most `depth` deep.
fn ladder(
    side: &'static str,
    depth: NonZeroU32,
    mut levels: Vec<Level>,
    best: impl Fn(&Level, &Level) -> std::cmp::Ordering,
) -> Result<Vec<Level>> {
    let refused = |reason: String| Error::InvalidRecord {
        path: side.into(),
        reason: reason.into(),
    };
    if levels.len() > depth.get() as usize {
        return Err(refused(format!(
            "expected at most {depth} levels, the declared depth, got {}",
            levels.len()
        )));
    }
    levels.sort_by(best);
    if let Some(pair) = levels.windows(2).find(|pair| pair[0].px == pair[1].px) {
        return Err(refused(format!(
            "expected one level per price, got two at {}",
            pair[0].px
        )));
    }
    Ok(levels)
}

super::product::holds_market_event!(BookData via event, event_mut, with_previous = Self::following);

impl Book for BookData {
    fn get_depth(&self) -> NonZeroU32 {
        self.depth
    }

    fn get_bids(&self) -> &[Level] {
        &self.bids
    }

    fn get_asks(&self) -> &[Level] {
        &self.asks
    }

    fn set_bids(&mut self, levels: Vec<Level>) -> Result<()> {
        self.bids = ladder("bids", self.depth, levels, |left, right| {
            right.px.cmp(&left.px)
        })?;
        Ok(())
    }

    fn set_asks(&mut self, levels: Vec<Level>) -> Result<()> {
        self.asks = ladder("asks", self.depth, levels, |left, right| {
            left.px.cmp(&right.px)
        })?;
        Ok(())
    }

    fn get_updates(&self) -> u64 {
        self.updates
    }

    fn set_updates(&mut self, updates: u64) {
        self.updates = updates;
    }
}

/// The datatype one level is stated in.
fn level_dtype() -> Result<DataType> {
    Ok(DataType::from(StructType::from_fields([
        column(
            "px",
            "Px",
            "The price of the level, exact.",
            Decimal18::dtype(),
            false,
        )?,
        column(
            "qty",
            "Qty",
            "The size resting at the level, exact.",
            Decimal18::dtype(),
            false,
        )?,
        column(
            "count",
            "Count",
            "How many orders and quote lanes rest at the level.",
            DataType::UInt64,
            false,
        )?,
    ])?))
}

/// One ladder as the fixed-size list its column holds: a slot per level
/// to the depth, the slots past the ladder null.
fn ladder_cells(ladder: &[Level], depth: NonZeroU32) -> Scalar {
    let depth = depth.get() as usize;
    let mut slots = Vec::with_capacity(depth);
    for level in ladder.iter().take(depth) {
        slots.push(Scalar::from_sequence([
            Scalar::from(level.px),
            Scalar::from(level.qty),
            Scalar::from(level.count),
        ]));
    }
    slots.resize(depth, Scalar::Null);
    Scalar::from_sequence(slots)
}

/// The levels one ladder cell states, in the order they sit, a null slot
/// closing the ladder.
fn levels_of(value: &Scalar) -> Vec<Level> {
    value
        .as_sequence()
        .map(|slots| slots.iter().map_while(level_of).collect())
        .unwrap_or_default()
}

/// The level one slot states, read as the ordered row it canonicalizes to
/// or as the named input shape it may still be; nothing for a null slot.
fn level_of(value: &Scalar) -> Option<Level> {
    let (px, qty, count) = if let Some(cells) = value.as_sequence() {
        (cells.first()?, cells.get(1)?, cells.get(2)?)
    } else {
        let named = value.as_struct()?;
        (named.get("px")?, named.get("qty")?, named.get("count")?)
    };
    Some(Level {
        px: Decimal18::from_scalar(px)?,
        qty: Decimal18::from_scalar(qty)?,
        count: count.as_u64()?,
    })
}

impl Product for BookData {
    const NAME: &'static str = "book";

    const DISPLAY: &'static str = "Book";

    const DESCRIPTION: &'static str = "The ladder for one instrument at one instant: levels of price, size and \
         order count per side, to a declared depth, with what printed against it.";

    const MARKET: &'static [MarketColumn] = &[
        MarketColumn::LastPx,
        MarketColumn::LastQty,
        MarketColumn::AvgPx,
        MarketColumn::CumQty,
        MarketColumn::Tradable,
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

    const OWN: &'static [&'static str] = &["bids", "asks", "updates"];

    fn own_fields(&self) -> Result<Vec<Field>> {
        let depth = i32::try_from(self.depth.get()).map_err(|_| Error::InvalidRecord {
            path: Self::NAME.into(),
            reason: crate::text::expected_got("a depth a fixed-size list can hold", self.depth),
        })?;
        let ladder = |name, display, item, description| {
            column(
                name,
                display,
                description,
                DataType::fixed_size_list(level_dtype()?.nullable_field(item), depth)?,
                false,
            )
        };
        Ok(vec![
            ladder(
                "bids",
                "Bids",
                "bid",
                "The bid ladder, best first: one slot per level to the declared depth, \
                 each a price, a size and a count, the slots past the ladder empty.",
            )?,
            ladder(
                "asks",
                "Asks",
                "ask",
                "The ask ladder, best first: one slot per level to the declared depth, \
                 each a price, a size and a count, the slots past the ladder empty.",
            )?,
            column(
                "updates",
                "Updates",
                "How many statements have been applied to the book's chain since it \
                 began.",
                DataType::UInt64,
                false,
            )?,
        ])
    }

    fn own_cells(&self, cells: &mut Vec<Scalar>) -> Result<()> {
        cells.push(ladder_cells(&self.bids, self.depth));
        cells.push(ladder_cells(&self.asks, self.depth));
        cells.push(Scalar::from(self.updates));
        Ok(())
    }

    fn from_cells(cells: &Cells<'_>) -> Result<Self> {
        let bids = cells.own(0);
        let asks = cells.own(1);
        // The depth is the ladder's own width: what the row's fixed-size
        // list states, the wider of the two where a row was cut down.
        let width = |value: &Scalar| value.as_sequence().map_or(0, <[Scalar]>::len);
        let depth = u32::try_from(width(bids).max(width(asks)))
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or_else(|| Error::InvalidRecord {
                path: "bids".into(),
                reason: "expected a ladder of at least one slot, the declared depth, got no \
                         ladder"
                    .into(),
            })?;
        let mut book = Self::at(0, depth);
        cells.record(&mut book);
        book.set_bids(levels_of(bids))?;
        book.set_asks(levels_of(asks))?;
        book.updates = cells.own(2).as_u64().unwrap_or(0);
        book.fill_from_ladders();
        Ok(book)
    }
}
