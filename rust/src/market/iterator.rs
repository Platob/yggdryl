//! One book per symbol per instant, read out of a stream of statements.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::iter::FusedIterator;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::vec;

use smol_str::SmolStr;

use crate::graph::iterator::step_of;
use crate::graph::{Element, Event, MarketElement};
use crate::{
    Bloomberg, Cfi, Currency, Cusip, Decimal18, Error, Isin, Mic, Result, Sedol, State, Uuid,
};

use super::{Book, BookData, Level, Order, Quote, Statement, Symbol};

/// The state a print reports when the venue busted it: the print did not
/// happen, so it moves nothing - `40TRDCXL`, FIX's `ExecType(150)` of `H`.
const BUST: &str = "40TRDCXL";

/// One fact of the instrument a ladder is about, folded across every
/// statement that stated it: nothing yet, one value, or two that disagree.
#[derive(Clone, Debug, Default)]
enum Fact<T> {
    #[default]
    Unstated,
    Stated(T),
    Disagreed,
}

impl<T: PartialEq> Fact<T> {
    /// Folds one statement's value in: a stated value moves nothing to it,
    /// another value to disagreement, and disagreement stays.
    fn state(&mut self, value: Option<T>) {
        let Some(value) = value else {
            return;
        };
        match self {
            Self::Unstated => *self = Self::Stated(value),
            Self::Stated(held) if *held != value => *self = Self::Disagreed,
            Self::Stated(_) | Self::Disagreed => {}
        }
    }

    /// The one value every statement agreed on.
    fn stated(&self) -> Option<&T> {
        match self {
            Self::Stated(held) => Some(held),
            Self::Unstated | Self::Disagreed => None,
        }
    }
}

/// What the makers and prints of one ladder say the instrument is: each
/// fact where every statement that stated it agrees.
#[derive(Debug, Default)]
struct Instrument {
    currency: Fact<Currency>,
    unit: Fact<String>,
    ticker: Fact<String>,
    isin: Fact<Isin>,
    cusip: Fact<Cusip>,
    sedol: Fact<Sedol>,
    bloomberg: Fact<Bloomberg>,
    cfi: Fact<Cfi>,
    mic: Fact<Mic>,
    tradable: Fact<bool>,
}

impl Instrument {
    /// Folds one statement's instrument facts in: a currency of none, an
    /// empty unit or ticker and an absent code state nothing.
    fn state<E: MarketElement + ?Sized>(&mut self, statement: &E) {
        self.currency
            .state(Some(statement.get_currency().clone()).filter(|held| *held != Currency::none()));
        self.unit
            .state(Some(statement.get_unit().to_owned()).filter(|held| !held.is_empty()));
        self.ticker.state(
            statement
                .get_symbolticker()
                .map(str::trim)
                .filter(|held| !held.is_empty())
                .map(str::to_owned),
        );
        self.isin.state(statement.get_isincode().cloned());
        self.cusip.state(statement.get_cusipcode().cloned());
        self.sedol.state(statement.get_sedolcode().cloned());
        self.bloomberg.state(statement.get_bloombergcode().cloned());
        self.cfi.state(statement.get_cficode().cloned());
        self.mic.state(statement.get_miccode().cloned());
        self.tradable.state(statement.get_tradable());
    }

    /// States the agreed facts on a book.
    fn fill(&self, book: &mut BookData) {
        if let Some(currency) = self.currency.stated() {
            book.set_currency(currency.clone());
        }
        if let Some(unit) = self.unit.stated() {
            book.set_unit(unit.clone());
        }
        book.set_symbolticker(self.ticker.stated().cloned());
        book.set_isincode(self.isin.stated().cloned());
        book.set_cusipcode(self.cusip.stated().cloned());
        book.set_sedolcode(self.sedol.stated().cloned());
        book.set_bloombergcode(self.bloomberg.stated().cloned());
        book.set_cficode(self.cfi.stated().cloned());
        book.set_miccode(self.mic.stated().cloned());
        book.set_tradable(self.tradable.stated().copied());
    }
}

/// One lane resting on a ladder: a price and the size left at it.
type Lane = (Decimal18, Decimal18);

/// One side of a ladder being kept: the size and the count resting at
/// each price, a level removed when nothing rests at it.
type Levels = BTreeMap<Decimal18, (Decimal18, u64)>;

/// One order or quote resting on a ladder: its lanes, when it leaves, and
/// the sources of the statement that rested it, which the book that shows
/// it gone names where nothing else took it off.
#[derive(Debug)]
struct Maker {
    bid: Option<Lane>,
    ask: Option<Lane>,
    expirunix: Option<i64>,
    sources: Vec<Uuid>,
}

/// The live layer of one symbol: what rests, what printed, what the
/// instrument is, and the book before the next.
#[derive(Debug, Default)]
struct Ladder {
    bids: Levels,
    asks: Levels,
    /// Every live order and quote, by the identity its chain shares.
    resting: HashMap<Uuid, Maker>,
    instrument: Instrument,
    lastpx: Option<Decimal18>,
    lastqty: Option<Decimal18>,
    /// What printed since the chain began, and what it was worth.
    volume: Decimal18,
    turnover: Decimal18,
    /// How many statements were applied since the chain began.
    updates: u64,
    /// The sources of every statement applied since the last book.
    sources: Vec<Uuid>,
    /// The last book read: the chain's predecessor.
    previous: Option<BookData>,
}

impl Ladder {
    /// Rests one maker's lanes on the levels.
    fn rest(&mut self, maker: &Maker) {
        if let Some((px, size)) = maker.bid {
            rest(&mut self.bids, px, size);
        }
        if let Some((px, size)) = maker.ask {
            rest(&mut self.asks, px, size);
        }
    }

    /// Takes one maker's lanes off the levels.
    fn lift(&mut self, maker: &Maker) {
        if let Some((px, size)) = maker.bid {
            lift(&mut self.bids, px, size);
        }
        if let Some((px, size)) = maker.ask {
            lift(&mut self.asks, px, size);
        }
    }

    /// Applies one print: the last price and size, the volume and what it
    /// was worth; a print of nothing, or one the venue busted, moves
    /// nothing.
    fn print(&mut self, px: Decimal18, qty: Decimal18, state: &State) {
        if px <= Decimal18::ZERO || qty <= Decimal18::ZERO || state.as_str() == BUST {
            return;
        }
        self.lastpx = Some(px);
        self.lastqty = Some(qty);
        self.volume = self.volume.checked_add(qty).unwrap_or(Decimal18::MAX);
        self.turnover = px
            .checked_mul(qty)
            .and_then(|worth| self.turnover.checked_add(worth))
            .unwrap_or(Decimal18::MAX);
    }
}

/// Adds one lane to a side.
fn rest(side: &mut Levels, px: Decimal18, size: Decimal18) {
    if size <= Decimal18::ZERO {
        return;
    }
    let held = side.entry(px).or_insert((Decimal18::ZERO, 0));
    held.0 = held.0.checked_add(size).unwrap_or(Decimal18::MAX);
    held.1 = held.1.saturating_add(1);
}

/// Takes one lane off a side: the size floored at zero, the count less
/// one, the level removed once nothing rests at it.
fn lift(side: &mut Levels, px: Decimal18, size: Decimal18) {
    if size <= Decimal18::ZERO {
        return;
    }
    if let Some(held) = side.get_mut(&px) {
        held.0 = held
            .0
            .checked_sub(size)
            .unwrap_or(Decimal18::ZERO)
            .max(Decimal18::ZERO);
        held.1 = held.1.saturating_sub(1);
        if held.1 == 0 || held.0.is_zero() {
            side.remove(&px);
        }
    }
}

/// Where the statements come from: streamed in their own order, or
/// collected and sorted first.
enum Source<S, I> {
    Streamed(I, PhantomData<fn() -> S>),
    Sorted(vec::IntoIter<Result<Statement>>),
}

/// One book per symbol per instant, read out of a stream of statements:
/// the live orders and quotes of each instrument kept as its ladders, the
/// prints against them kept as its last trade, volume and average price,
/// and the book of every symbol an instant touched read once the stream
/// moves past that instant.
///
/// The statements of one instant are buffered and applied together, so a
/// book is unique for its instant: the state after everything the instant
/// said. Applied means, for an order, that its former lanes leave the
/// ladder and, where it still [rests](Order::is_resting), its one lane
/// rests at its limit for what it has [left](Order::remaining), under the
/// identity its chain shares, so a later statement of the same order
/// replaces the earlier; a live order stating nothing of the ladder - no
/// price, no quantity, nothing filled or left, as a cancel request states
/// nothing - leaves its lanes as they stand, and a dead one - filled,
/// cancelled, rejected, expired - leaves the ladder. For a quote, that both
/// lanes it [quotes](Quote::bid) rest under its identity, a restatement
/// replacing both and a lane withdrawn to zero leaving. For a print - an
/// execution or a trade - that its price and size are the last, its size
/// joins the volume and its worth the turnover, never resting and never
/// touching a level; a print the venue busted moves nothing. A stream
/// carrying both the execution and the trade of one fill counts it twice,
/// so a caller feeds one of the two. An order past its expiration leaves
/// at the first instant closed at or after it, with no statement of its
/// own, and its symbol's book is read at that instant.
///
/// A statement keys the symbol of the instrument it names, [`Symbol::of`],
/// with every code a statement named registered as an alias of the symbol
/// it was keyed under, so an instrument named by its ticker first and its
/// ISIN later stays one book; a print keys under the maker it fills where
/// one rests; a maker keyed under a new symbol leaves the ladder it rested
/// on before it rests on the new one; and a statement naming no instrument
/// keys [`Symbol::GLOBAL`]. [`Self::with_symbol`] keys every statement
/// under one symbol instead, which read under the global symbol is the
/// global book of the whole stream. The books of one instant are yielded
/// in symbol order.
///
/// Every book is what [`BookData`] states: its ladders cut to the declared
/// depth, the instrument's facts where every statement that stated them
/// agrees - a global book over two currencies is priced in none - the
/// prints, how many statements made it, the sources of the statements
/// applied since the book before it, and the flat chain: its predecessor
/// and its place, so a stream of a million books carries no lineage. With
/// [`Self::with_snapshot_ns`], a step is one instant: the book is the
/// step's closing state, dated at the last instant that moved the symbol
/// and stamped with the step.
///
/// Held state, and its bound: one maker per live order or quote and one
/// level per distinct resting price, per symbol; a maker leaves on a
/// statement of it that does not rest, on its expiry, or with the stream,
/// exactly as the walk's live set is bounded; the statements of one open
/// instant; and the books of one closed instant until they are yielded.
/// A source error is yielded where it is met and neither closes the
/// instant nor moves a ladder; a statement before the open instant, under
/// the caller's word that the stream is sorted, is refused naming
/// `currunix` and moves nothing, because a ladder cannot be rewound - the
/// one layer that refuses it, where a walk lets it through unchained.
///
/// ```
/// use std::num::NonZeroU32;
///
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Book, BookIterator, ExecutionData, OrderData, Statement};
/// use yggdryl::{Currency, Decimal18, Side, State};
///
/// # fn main() -> yggdryl::Result<()> {
/// let order = |unix: i64, code: &str, side: &str, px: &str, qty: i64| -> yggdryl::Result<Statement> {
///     let mut order = OrderData::at(unix);
///     order.set_crosscode(code.to_owned());
///     order.set_symbolticker(Some("AAPL".to_owned()));
///     order.set_side(Side::read(side)?);
///     order.set_px(px.parse()?);
///     order.set_qty(Decimal18::from_int(qty));
///     order.set_state(State::read("New")?);
///     order.set_currency(Currency::new("USD")?);
///     order.finalize();
///     Ok(order.into())
/// };
/// let mut fill = ExecutionData::at(30);
/// fill.set_crosscode("A1".to_owned());
/// fill.set_symbolticker(Some("AAPL".to_owned()));
/// fill.set_px("10.5".parse()?);
/// fill.set_qty(Decimal18::from_int(40));
/// fill.set_state(State::read("PartiallyFilled")?);
/// fill.finalize();
/// let statements = [
///     order(10, "A1", "Buy", "10.5", 100)?,
///     order(10, "S1", "Sell", "10.7", 80)?,
///     order(20, "A2", "Buy", "10.5", 50)?,
///     Statement::from(fill),
/// ];
/// let depth = NonZeroU32::new(5).expect("five");
/// let books = BookIterator::new(statements.map(Ok), depth, true).collect::<yggdryl::Result<Vec<_>>>()?;
/// // One book per instant: two makers, then a third, then a print.
/// assert_eq!(books.len(), 3);
/// assert_eq!(books[0].get_crosscode(), "AAPL");
/// assert_eq!((books[0].bid_count(), books[0].ask_count()), (1, 1));
/// assert_eq!(books[1].best_bid().map(|level| (level.qty, level.count)), Some((Decimal18::from_int(150), 2)));
/// assert_eq!(books[1].get_px(), "10.6".parse()?, "the mid");
/// assert_eq!((books[2].get_lastpx(), books[2].get_cumqty()), (Some("10.5".parse()?), Some(Decimal18::from_int(40))));
/// assert_eq!((books[2].get_seqnum(), books[2].get_prevuuid()), (2, Some(books[1].get_curruuid())));
/// assert_eq!(books[2].get_parentuuids(), [books[1].get_curruuid()], "a flat chain");
/// # Ok(())
/// # }
/// ```
pub struct BookIterator<S, I> {
    source: Source<S, I>,
    depth: NonZeroU32,
    snapshot_ns: i64,
    symbol: Option<Symbol>,
    /// The open instant: every statement of it is applied, none of a
    /// later one.
    instant: Option<i64>,
    ladders: BTreeMap<Symbol, Ladder>,
    /// Every resting maker, by the identity its chain shares, to the
    /// symbol it rests under.
    placed: HashMap<Uuid, Symbol>,
    /// Every instrument code met, to the symbol it keys.
    known: HashMap<SmolStr, Symbol>,
    /// Every resting maker with an expiry, by expiry: the sweep's order.
    expiring: BTreeSet<(i64, Symbol, Uuid)>,
    /// The symbols whose ladder moved since their last book, and the
    /// instant each last moved at.
    touched: BTreeMap<Symbol, i64>,
    /// The books read at a closed instant, owed in symbol order.
    due: VecDeque<Result<BookData>>,
    /// The first statement of a later instant, pulled while the open one
    /// was still open.
    later: Option<Statement>,
    done: bool,
}

impl<S, I> std::fmt::Debug for BookIterator<S, I> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BookIterator")
            .field("depth", &self.depth)
            .field("snapshot_ns", &self.snapshot_ns)
            .field("symbol", &self.symbol)
            .field("instant", &self.instant)
            .finish_non_exhaustive()
    }
}

impl<S, I> BookIterator<S, I>
where
    S: Into<Statement>,
    I: Iterator<Item = Result<S>>,
{
    /// Opens the iterator over `statements`, reading books `depth` levels
    /// a side. `sorted` is the caller's word that the statements arrive in
    /// instant order, which a walked stream does; otherwise they are
    /// collected and stably sorted first, the errors met yielded first in
    /// their order.
    pub fn new(
        statements: impl IntoIterator<IntoIter = I>,
        depth: NonZeroU32,
        sorted: bool,
    ) -> Self {
        let source = if sorted {
            Source::Streamed(statements.into_iter(), PhantomData)
        } else {
            let mut errors = Vec::new();
            let mut held: Vec<Statement> = Vec::new();
            for item in statements {
                match item {
                    Ok(statement) => held.push(statement.into()),
                    Err(error) => errors.push(Err(error)),
                }
            }
            held.sort_by_key(Event::get_currunix);
            errors.extend(held.into_iter().map(Ok));
            Source::Sorted(errors.into_iter())
        };
        Self {
            source,
            depth,
            snapshot_ns: 0,
            symbol: None,
            instant: None,
            ladders: BTreeMap::new(),
            placed: HashMap::new(),
            known: HashMap::new(),
            expiring: BTreeSet::new(),
            touched: BTreeMap::new(),
            due: VecDeque::new(),
            later: None,
            done: false,
        }
    }

    /// A grid of `snapshot_ns` nanoseconds aligned on the epoch: one book
    /// per symbol per step it was touched in, the step's closing state,
    /// stamped with the step; zero or less takes the grid away, which is
    /// one book per instant.
    #[must_use]
    pub const fn with_snapshot_ns(mut self, snapshot_ns: i64) -> Self {
        self.snapshot_ns = snapshot_ns;
        self
    }

    /// Every statement keyed under `symbol` instead of the one it names,
    /// so one book stands for the whole stream: [`Symbol::GLOBAL`] is the
    /// global book, and a named symbol a caller's own keying of a stream
    /// whose statements name no instrument.
    #[must_use]
    pub fn with_symbol(mut self, symbol: Symbol) -> Self {
        self.symbol = Some(symbol);
        self
    }

    /// The grid step, where there is one.
    #[must_use]
    pub const fn snapshot_ns(&self) -> Option<i64> {
        if self.snapshot_ns > 0 {
            Some(self.snapshot_ns)
        } else {
            None
        }
    }

    /// How many levels a side every book is read to.
    #[must_use]
    pub const fn depth(&self) -> NonZeroU32 {
        self.depth
    }

    /// The one symbol every statement is keyed under, where one was
    /// declared.
    #[must_use]
    pub const fn symbol(&self) -> Option<&Symbol> {
        self.symbol.as_ref()
    }

    /// The next statement or error, the held one first.
    fn pull(&mut self) -> Option<Result<Statement>> {
        if let Some(held) = self.later.take() {
            return Some(Ok(held));
        }
        match &mut self.source {
            Source::Streamed(source, _) => source.next().map(|held| held.map(Into::into)),
            Source::Sorted(source) => source.next(),
        }
    }

    /// The symbol one statement keys: the declared one; else, for a
    /// print, the symbol of the maker it fills where one rests; else the
    /// symbol any code it names is known under; else the one it names.
    fn key_of(&self, statement: &Statement) -> Symbol {
        if let Some(symbol) = &self.symbol {
            return symbol.clone();
        }
        if statement.is_print() {
            if let Some(symbol) = self.placed.get(&statement.get_crossuuid()) {
                return symbol.clone();
            }
        }
        codes_of(statement)
            .find_map(|code| self.known.get(code.as_str()))
            .cloned()
            .unwrap_or_else(|| Symbol::of(statement))
    }

    /// Registers every code a statement names as an alias of the symbol
    /// it was keyed under, where the code is not known yet.
    fn register(&mut self, statement: &Statement, key: &Symbol) {
        for code in codes_of(statement) {
            self.known.entry(code).or_insert_with(|| key.clone());
        }
    }

    /// Takes one maker off the ladder it rests on, where it rests, and
    /// touches that symbol at `at`; the ladder names what rested the
    /// maker among its sources, so the book that shows it gone names it.
    fn retire(&mut self, identity: Uuid, at: i64) {
        let Some(symbol) = self.placed.remove(&identity) else {
            return;
        };
        if let Some(ladder) = self.ladders.get_mut(&symbol) {
            if let Some(maker) = ladder.resting.remove(&identity) {
                ladder.lift(&maker);
                ladder.sources.extend_from_slice(&maker.sources);
                if let Some(expiry) = maker.expirunix {
                    self.expiring.remove(&(expiry, symbol.clone(), identity));
                }
            }
        }
        self.touched.insert(symbol, at);
    }

    /// Rests one maker under `key`.
    fn place(&mut self, key: &Symbol, identity: Uuid, maker: Maker) {
        if maker.bid.is_none() && maker.ask.is_none() {
            return;
        }
        if let Some(expiry) = maker.expirunix {
            self.expiring.insert((expiry, key.clone(), identity));
        }
        let ladder = self.ladders.entry(key.clone()).or_default();
        ladder.rest(&maker);
        ladder.resting.insert(identity, maker);
        self.placed.insert(identity, key.clone());
    }

    /// Applies one statement of the open instant.
    fn apply(&mut self, statement: Statement) {
        let at = statement.get_currunix();
        let key = self.key_of(&statement);
        self.register(&statement, &key);
        {
            let ladder = self.ladders.entry(key.clone()).or_default();
            ladder.updates = ladder.updates.saturating_add(1);
            ladder.sources.extend_from_slice(statement.get_srcuuids());
            ladder.instrument.state(&statement);
        }
        self.touched.insert(key.clone(), at);
        match statement {
            Statement::Order(order) => {
                let identity = order.get_crossuuid();
                let states_lane = order.get_px() > Decimal18::ZERO
                    || !order.get_qty().is_zero()
                    || order.get_leavesqty().is_some()
                    || order.get_cumqty().is_some();
                if !order.is_alive() {
                    self.retire(identity, at);
                } else if states_lane {
                    self.retire(identity, at);
                    if order.is_resting() {
                        let lane = Some((order.get_px(), order.remaining()));
                        let side = order.get_side();
                        self.place(
                            &key,
                            identity,
                            Maker {
                                bid: lane.filter(|_| side.is_bid()),
                                ask: lane.filter(|_| side.is_ask()),
                                expirunix: order.get_expirunix(),
                                sources: order.get_srcuuids().to_vec(),
                            },
                        );
                    }
                }
            }
            Statement::Quote(quote) => {
                let identity = quote.get_crossuuid();
                self.retire(identity, at);
                if quote.is_alive() {
                    self.place(
                        &key,
                        identity,
                        Maker {
                            bid: quote.bid(),
                            ask: quote.ask(),
                            expirunix: quote.get_expirunix(),
                            sources: quote.get_srcuuids().to_vec(),
                        },
                    );
                }
            }
            Statement::Execution(_) | Statement::Trade(_) => {
                if let Some(ladder) = self.ladders.get_mut(&key) {
                    ladder.print(
                        statement.get_px(),
                        statement.get_qty(),
                        statement.get_state(),
                    );
                }
            }
        }
    }

    /// Takes every maker past its expiry at `open` off its ladder.
    fn sweep(&mut self, open: i64) {
        while let Some((expiry, symbol, identity)) = self.expiring.first().cloned() {
            if expiry > open {
                break;
            }
            self.expiring.remove(&(expiry, symbol.clone(), identity));
            let still = self
                .ladders
                .get(&symbol)
                .and_then(|ladder| ladder.resting.get(&identity))
                .is_some_and(|maker| maker.expirunix == Some(expiry));
            if still {
                self.retire(identity, open);
            }
        }
    }

    /// Closes the open instant `open`, the next instant being `next` or
    /// none at the end: the expiries swept, then every touched symbol's
    /// book read where the instant closes one - always without a grid,
    /// and with one where the next instant falls in a later step or there
    /// is none.
    fn close(&mut self, open: i64, next: Option<i64>) {
        self.sweep(open);
        let reads = match (self.snapshot_ns(), next) {
            (None, _) | (Some(_), None) => true,
            (Some(step), Some(next)) => step_of(next, step) > step_of(open, step),
        };
        if !reads {
            return;
        }
        for (symbol, at) in std::mem::take(&mut self.touched) {
            let book = self.read(&symbol, at);
            self.due.push_back(book);
        }
    }

    /// The book of `symbol` as its ladder stands, read at `at`.
    fn read(&mut self, symbol: &Symbol, at: i64) -> Result<BookData> {
        let depth = self.depth;
        let step = self.snapshot_ns().map(|step| step_of(at, step));
        let ladder = self.ladders.entry(symbol.clone()).or_default();
        let mut book = BookData::at(at, depth);
        book.set_crosscode(symbol.as_str().to_owned());
        ladder.instrument.fill(&mut book);
        let level = |(px, (qty, count)): (&Decimal18, &(Decimal18, u64))| Level {
            px: *px,
            qty: *qty,
            count: *count,
        };
        let take = depth.get() as usize;
        book.set_bids(ladder.bids.iter().rev().take(take).map(level).collect())?;
        book.set_asks(ladder.asks.iter().take(take).map(level).collect())?;
        book.set_lastpx(ladder.lastpx);
        book.set_lastqty(ladder.lastqty);
        if ladder.volume > Decimal18::ZERO {
            book.set_cumqty(Some(ladder.volume));
            book.set_avgpx(ladder.turnover.checked_div(ladder.volume));
        }
        book.set_updates(ladder.updates);
        book.set_snapunix(step);
        let mut sources = std::mem::take(&mut ladder.sources);
        sources.sort_unstable();
        sources.dedup();
        book.set_srcuuids(sources);
        let book = match &ladder.previous {
            Some(previous) => book.clone().with_previous(previous).unwrap_or_else(|| {
                let mut book = book;
                book.finalize();
                book
            }),
            None => {
                let mut book = book;
                book.finalize();
                book
            }
        };
        ladder.previous = Some(book.clone());
        Ok(book)
    }
}

/// Every instrument code a statement names, as text: the ISIN, the
/// ticker, the CUSIP, the SEDOL and the Bloomberg identifier.
fn codes_of(statement: &Statement) -> impl Iterator<Item = SmolStr> + '_ {
    let ticker = statement
        .get_symbolticker()
        .map(str::trim)
        .filter(|held| !held.is_empty())
        .map(SmolStr::new);
    [
        statement
            .get_isincode()
            .map(|held| SmolStr::new(held.as_str())),
        ticker,
        statement
            .get_cusipcode()
            .map(|held| SmolStr::new(held.as_str())),
        statement
            .get_sedolcode()
            .map(|held| SmolStr::new(held.as_str())),
        statement
            .get_bloombergcode()
            .map(|held| SmolStr::new(held.as_str())),
    ]
    .into_iter()
    .flatten()
}

impl<S, I> Iterator for BookIterator<S, I>
where
    S: Into<Statement>,
    I: Iterator<Item = Result<S>>,
{
    type Item = Result<BookData>;

    fn next(&mut self) -> Option<Result<BookData>> {
        loop {
            if let Some(book) = self.due.pop_front() {
                return Some(book);
            }
            if self.done {
                return None;
            }
            let statement = match self.pull() {
                None => {
                    self.done = true;
                    if let Some(open) = self.instant {
                        self.close(open, None);
                    }
                    continue;
                }
                Some(Err(error)) => return Some(Err(error)),
                Some(Ok(statement)) => statement,
            };
            let at = statement.get_currunix();
            match self.instant {
                Some(open) if at < open => {
                    return Some(Err(Error::InvalidRecord {
                        path: "currunix".into(),
                        reason: crate::text::expected_got(
                            format_args!(
                                "an instant at or after the open one, {open}, in a stream the caller called sorted"
                            ),
                            at,
                        ),
                    }));
                }
                Some(open) if at > open => {
                    self.later = Some(statement);
                    self.close(open, Some(at));
                    self.instant = Some(at);
                }
                _ => {
                    self.instant = Some(at);
                    self.apply(statement);
                }
            }
        }
    }
}

impl<S, I> FusedIterator for BookIterator<S, I>
where
    S: Into<Statement>,
    I: Iterator<Item = Result<S>>,
{
}
