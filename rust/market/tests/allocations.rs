//! The allocation claims over what `yggdryl-market` owns, counted by the
//! same allocator as the core's own in `rust/tests/allocations.rs`.
//!
//! What a borrowed protocol view allocates, counted rather than asserted.
//!
//! A view is one pointer plus a `Scheme`, built per call rather than stored,
//! and that is only defensible if building one and reading through it costs
//! nothing. A comment saying so is not evidence, and neither is a timing: a
//! stray `String` in an accessor hides easily inside a map lookup. So this
//! counts them, and pins the three places a protocol read does allocate - a
//! key handed back to the caller, a lookup key too long for `SmolStr`'s inline
//! buffer, and a value that is a serie.
//!
//! It also pins the no-op write. A rewrite of a value a field already carries
//! must cost the same however much metadata surrounds it, because it stops
//! before copying the map; nothing else fails when that short-circuit is lost,
//! so nothing else would notice.
//!
//! The counting allocator is a global and a program has exactly one, which is
//! why this is its own test target rather than a case in another file.

#[path = "support/install.rs"]
mod install;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use std::sync::Arc;

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event};
use yggdryl::{
    ArrowCastOptions, ArrowCastPlan, DataType, DataTypeId, Decimal, Field, FieldPath, FieldScalar,
    Scalar, Serie, State, TimeUnit, Timezone,
};
use yggdryl::{StructType, Uuid};
use yggdryl_market::IdKey;
use yggdryl_market::graph::{
    BookEvent, BookIterator, BookRef, ExecutionEvent, Market, MarketData, MdUpdateAction,
    Operation, OrderEvent, QuoteEvent, TradeEvent,
};
use yggdryl_market::{IdSource, IdType, Identifier, Side};

/// A pass-through allocator that counts allocations, and the bytes they
/// hold, while armed.
struct Counting;

/// Allocations since the counter was armed.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// Bytes the armed thread allocated less the bytes it freed since it was
/// armed - below zero where it frees what it held before - and the most
/// that difference reached.
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

/// Move the armed thread's live bytes by `delta`, keeping the peak.
fn held(delta: isize) {
    let live = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

/// A size as a byte delta: an allocation never exceeds `isize::MAX`.
fn signed(size: usize) -> isize {
    isize::try_from(size).unwrap_or(isize::MAX)
}

// Armed *per thread*, and const-initialized so reading it inside the allocator
// cannot itself allocate. Cargo runs the cases in this file concurrently, and a
// process-wide flag would charge one case for another's setup - which looks
// exactly like the stray accessor allocation these cases exist to catch.
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Held across a counted section, so only one thread is ever armed.
static COUNTING: Mutex<()> = Mutex::new(());

// SAFETY: every method forwards to `System`, which upholds the contract; the
// counter is an atomic and the flag is a const-initialized thread local, so
// neither adds aliasing nor re-enters the allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            held(signed(layout.size()));
        }
        // SAFETY: `layout` is forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ARMED.get() {
            held(-signed(layout.size()));
        }
        // SAFETY: the pointer came from `System.alloc` with this same layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            held(signed(size) - signed(layout.size()));
        }
        // SAFETY: the pointer and layout came from this allocator.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Count the allocations `work` performs, and return them with its answer.
fn counted<T>(work: impl FnOnce() -> T) -> (usize, T) {
    let (counted, _, _, answer) = armed(work);
    (counted, answer)
}

/// Run `work` armed: its allocations, the most bytes it held at once, the
/// bytes it still held when it returned and its answer.
fn armed<T>(work: impl FnOnce() -> T) -> (usize, usize, isize, T) {
    let guard = COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The machine's name is read once per process, by whichever surface asks
    // first; read here, outside every count, it is never any one test's cost.
    black_box(yggdryl::HOSTNAME.len());
    ALLOCATIONS.store(0, Ordering::Relaxed);
    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    ARMED.set(true);
    let answer = work();
    ARMED.set(false);
    let counted = ALLOCATIONS.load(Ordering::Relaxed);
    let peak = usize::try_from(PEAK.load(Ordering::Relaxed)).unwrap_or(0);
    let live = LIVE.load(Ordering::Relaxed);
    drop(guard);
    (counted, peak, live, answer)
}

/// Count `work` run once and run a thousand times.
///
/// One run alone cannot tell a per-call cost from a one-time initialization,
/// so every case reports both and asserts the relationship between them.
fn counted_once_and_repeated(mut work: impl FnMut()) -> (usize, usize) {
    work();
    let (once, ()) = counted(&mut work);
    let (repeated, ()) = counted(|| {
        for _ in 0..1_000 {
            work();
        }
    });
    (once, repeated)
}

/// Assert a read costs nothing, whether it runs once or a thousand times.
fn free(what: &str, work: impl FnMut()) {
    let (once, repeated) = counted_once_and_repeated(work);
    assert_eq!(once, 0, "{what} allocated on a single read");
    assert_eq!(repeated, 0, "{what} allocated over a thousand reads");
}

/// Count `work` over an input `setup` builds outside the count, once and
/// then sixty-four times over inputs built ahead: a stage that takes a
/// fresh message or row by value is charged for what it does with it and
/// never for the clone that hands it one. Sixty-four rather than a thousand,
/// because a packed message is tens of kilobytes.
fn counted_each<I, O>(
    mut setup: impl FnMut() -> I,
    mut work: impl FnMut(I) -> O,
) -> (usize, usize) {
    drop(work(setup()));
    let one = setup();
    let (once, out) = counted(|| work(one));
    drop(out);
    let inputs: Vec<I> = (0..64).map(|_| setup()).collect();
    let mut outputs = Vec::with_capacity(64);
    let (repeated, ()) = counted(|| outputs.extend(inputs.into_iter().map(&mut work)));
    drop(outputs);
    (once, repeated)
}

/// Assert a read costs exactly `each` allocations every time it runs.
fn costs(what: &str, each: usize, work: impl FnMut()) {
    let (once, repeated) = counted_once_and_repeated(work);
    assert_eq!(once, each, "{what} cost changed for a single read");
    assert_eq!(
        repeated,
        each * 1_000,
        "{what} did not cost {each} per read over a thousand"
    );
}

#[test]
fn market_following_allocates_nothing_for_an_inherited_ticker() {
    crate::install::installed();
    let mut previous = OrderEvent::at(1);
    previous.finalize();
    let next = OrderEvent::at(2);
    let (baseline, _) = counted(|| next.with_previous(&previous).unwrap());

    previous.set_ticker(Some(SmolStr::new("AAPL")), true);
    previous.finalize();
    let next = OrderEvent::at(2);
    let (inherited, next) = counted(|| next.with_previous(&previous).unwrap());
    assert_eq!(next.get_ticker(), Some("AAPL"));
    // A ticker is a `SmolStr`: one of a symbol's length lives inline, so
    // inheriting it copies and allocates nothing where the owned `String`
    // it replaced cost one allocation.
    assert_eq!(inherited, baseline, "an inherited inline ticker is a copy");

    let mut next = OrderEvent::at(2);
    next.set_ticker(Some(SmolStr::new("MSFT")), true);
    let (stated, next) = counted(|| next.with_previous(&previous).unwrap());
    assert_eq!(next.get_ticker(), Some("MSFT"));
    assert_eq!(stated, baseline, "a stated ticker needs no clone");
}

#[test]
fn market_event_identity_refresh_and_finalization_allocate_nothing() {
    crate::install::installed();
    let mut event = OrderEvent::at(1_700_000_000_000_000_000);
    event.set_hashcode(1);
    let mut generation = 1_u64;
    free("refreshing and finalizing a market event identity", || {
        generation = generation.wrapping_add(1);
        event.set_transunix(1_700_000_000_000_000_000 + generation as i64);
        event.set_seqnum(generation);
        event.set_crosshashcode(generation);
        event.finalized(generation.rotate_left(17));
        black_box((event.get_uuid(), event.get_crossuuid()));
    });
}

#[test]
fn a_leaf_moves_into_and_out_of_market_data_without_allocating() {
    crate::install::installed();
    let mut event = OrderEvent::at(1);
    event.set_crosscode("ORDER-1".to_owned());
    event.finalize();
    let mut operation = Some(event);
    let (into_value, value) =
        counted(|| MarketData::from(operation.take().expect("one operation")));
    assert_eq!(into_value, 0, "leaf to value allocated");

    let mut value = Some(value);
    let (into_leaf, operation) =
        counted(|| OrderEvent::try_from(value.take().expect("one value")).unwrap());
    assert_eq!(into_leaf, 0, "value to leaf allocated");

    // An event drops its clocks into the element it states, and an element
    // dated again is an event: both moves keep the one holder.
    let mut operation = Some(operation);
    let (into_element, element) =
        counted(|| operation.take().expect("one operation").into_element());
    assert_eq!(into_element, 0, "event to element allocated");

    let mut element = Some(element);
    let (at, operation) = counted(|| element.take().expect("one element").at(2));
    assert_eq!(at, 0, "element to event allocated");
    black_box(operation);
}

fn allocation_trade_parts(executions: usize) -> (OrderEvent, Vec<ExecutionEvent>) {
    let mut root = OrderEvent::at(1);
    root.set_crosscode("ALLOC-TRADE".to_owned());
    root.set_ticker(Some(SmolStr::new("ALLOC")), true);
    root.set_state(State::read("Filled").expect("the shipped filled state"));
    root.finalize();
    let executions = (0..executions)
        .rev()
        .map(|index| {
            let mut event = ExecutionEvent::at(1);
            event.set_crosscode(format!("ALLOC-EXEC-{index:04}"));
            event.set_ticker(Some(SmolStr::new("ALLOC")), true);
            event.set_side(
                Side::read(if index % 2 == 0 { "Buy" } else { "Sell" }).unwrap(),
                true,
            );
            event.set_state(State::read("Filled").expect("the shipped filled state"));
            event.finalize();
            event
        })
        .collect();
    (root, executions)
}

#[test]
fn trade_construction_does_not_allocate_per_execution() {
    crate::install::installed();
    let (root, executions) = allocation_trade_parts(1);
    let (shallow_allocations, shallow) = counted(|| TradeEvent::from_parts(&root, executions));
    let shallow = shallow.expect("the shallow trade");

    let (root, executions) = allocation_trade_parts(128);
    let (deep_allocations, deep) = counted(|| TradeEvent::from_parts(&root, executions));
    let deep = deep.expect("the deep trade");
    assert_eq!(shallow.executions().len(), 1);
    assert_eq!(deep.executions().len(), 128);
    assert!(
        deep_allocations <= shallow_allocations + 4,
        "constructing 128 executions allocated {deep_allocations} times but one execution allocated {shallow_allocations} times"
    );
    black_box((shallow, deep));
}

fn allocation_book_operation(
    code: impl Into<String>,
    unix: i64,
    quantity: i64,
    state: &str,
) -> MarketData {
    let mut event = QuoteEvent::at(unix);
    event.set_crosscode(code.into());
    event.set_ticker(Some(SmolStr::new("ALLOC")), true);
    event.set_side(Side::read("Buy").expect("the shipped buy side"), true);
    event.set_price(Some(Decimal::from_int(100)), true);
    event.set_quantity(Some(Decimal::from_int(quantity)), true);
    event.set_state(State::read(state).expect("a shipped state"));
    event.finalize();
    event.into()
}

fn allocation_book(entries: usize) -> BookEvent {
    let mut book = BookEvent::new(1, "ALLOC");
    book.add_operations(
        (0..entries).map(|index| allocation_book_operation(format!("ALLOC-{index}"), 1, 1, "New")),
    )
    .expect("the initial depth");
    book
}

/// One step of a book walk between ticks - the next book, emitted as a
/// delta book - makes as many allocations at 8 levels a side as at 1,024,
/// whether the consumer drops every book or holds every one: nothing the
/// step allocates is per level or per entry, and a delta book holds no
/// side, so holding it never makes the next step copy one.
/// The second step is counted, the walk's first change after its first
/// book - every entry in its delta - behind it. A copy of a side's store is
/// the same few
/// allocations at any depth, so this count cannot tell a shared store from
/// a copied one; `a_walk_shares_each_side_s_store_with_the_books_it_emits`
/// in `rust/market/tests/graph/book.rs` pins the sharing itself.
#[test]
fn a_delta_book_walk_step_allocates_alike_at_8_and_1024_levels_while_every_book_is_held() {
    crate::install::installed();
    let step = |levels: usize, held: bool| {
        let updates = [2, 3].map(|unix| allocation_level_entry("Buy", 0, 0, unix, "Replaced"));
        let mut books = BookIterator::new(
            allocation_level_entries(levels).into_iter().chain(updates),
            0,
        )
        .unwrap();
        let first = books.next().unwrap().unwrap();
        assert_eq!(first.delta().len(), levels * 16);
        assert_eq!(first.events().len(), 0);
        let second = books.next().unwrap().unwrap();
        assert!(!second.is_complete());
        let kept = held.then_some((first, second));
        let (allocations, book) = counted(|| books.next().unwrap().unwrap());
        assert!(!book.is_complete());
        assert_eq!(book.delta().len(), 1);
        assert_eq!(book.events().len(), 0);
        black_box((kept, book));
        allocations
    };
    for held in [false, true] {
        let (shallow, deep) = (step(8, held), step(1_024, held));
        assert!(
            deep <= shallow + 2,
            "a walk step allocated {deep} times at 1,024 levels but {shallow} times at 8, every book held: {held}"
        );
    }
}

/// One step of a walk at a touch 1,024 entries deep on each side makes as
/// many allocations as at 8: the step finds and moves one entry of the
/// level, and settling the top of book and checking the level it joined
/// read what the level keeps - nothing it builds is per entry.
#[test]
fn a_walk_step_at_a_deep_touch_allocates_alike_at_8_and_1024_entries() {
    crate::install::installed();
    let step = |entries: usize| {
        let touch = ["Buy", "Sell"].into_iter().flat_map(move |side| {
            (0..entries).map(move |slot| allocation_level_entry(side, 0, slot, 1, "New"))
        });
        let updates =
            [2, 3].map(|unix| allocation_level_entry("Buy", 0, entries / 2, unix, "Replaced"));
        let mut books = BookIterator::new(touch.chain(updates), 0).unwrap();
        assert_eq!(books.next().unwrap().unwrap().delta().len(), 2 * entries);
        drop(books.next().unwrap().unwrap());
        let (allocations, book) = counted(|| books.next().unwrap().unwrap());
        assert!(!book.is_complete());
        assert_eq!(book.delta().len(), 1);
        black_box(book);
        allocations
    };
    let (shallow, deep) = (step(8), step(1_024));
    assert!(
        deep <= shallow + 2,
        "a walk step allocated {deep} times at a touch 1,024 deep but {shallow} times at 8"
    );
}

/// Rebuilding a delta book over the complete book before it, through
/// [`Element::with_previous`], makes as many allocations over a book 8
/// levels a side deep as over one 1,024 deep: it replays its delta, and
/// never copies an entry or builds anything per level.
#[test]
fn a_delta_book_rebuild_allocates_per_delta_not_per_level() {
    crate::install::installed();
    let rebuild = |levels: usize| {
        let updates = [2, 3].map(|unix| allocation_level_entry("Buy", 0, 0, unix, "Replaced"));
        let books = BookIterator::new(
            allocation_level_entries(levels).into_iter().chain(updates),
            0,
        )
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
        assert_eq!(books.len(), 3);
        let origin = BookEvent::new(books[0].get_transunix(), books[0].get_crosscode());
        let first = books[0].clone().with_previous(&origin).unwrap();
        let previous = books[1].clone().with_previous(&first).unwrap();
        let delta = books[2].clone();
        let (allocations, rebuilt) = counted(|| delta.with_previous(&previous).unwrap());
        assert_eq!(rebuilt.alive().count(), levels * 16);
        black_box((previous, rebuilt));
        allocations
    };
    let (shallow, deep) = (rebuild(8), rebuild(1_024));
    assert!(
        deep <= shallow + 2,
        "a rebuild allocated {deep} times at 1,024 levels but {shallow} times at 8"
    );
}

/// A complete book's row writes no source of an entry alive on it - the row
/// of the delta that applied the entry writes them - so a book whose alive
/// entries were each read from an element of their own lays out in as many
/// bytes as one whose entries state none, at 8 levels a side and at 1,024:
/// the row drops sixteen bytes per source per alive entry. Laying them out
/// allocates nothing for them either: the one allocation a sourced entry
/// costs is the write check's copy of it, which finalizes the copy to prove
/// the entry canonical and copies its sources with it.
#[test]
fn a_book_row_lays_out_alike_whether_its_alive_entries_state_sources() {
    crate::install::installed();
    let laid_out = |levels: usize, sourced: bool| {
        let mut entries = allocation_level_entries(levels);
        if sourced {
            for (entry, source) in entries.iter_mut().zip(1_u128..) {
                entry.set_srcuuids(vec![Uuid::from_v8(source)]);
            }
        }
        let update = allocation_level_entry("Buy", 0, 0, 2, "Replaced");
        let books = BookIterator::new(entries.into_iter().chain([update]), 0)
            .unwrap()
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(books.len(), 2);
        let origin = BookEvent::new(books[0].get_transunix(), books[0].get_crosscode());
        let first = books[0].clone().with_previous(&origin).unwrap();
        let book = books[1].clone().with_previous(&first).unwrap();
        assert_eq!(book.alive().count(), levels * 16);
        assert_eq!(book.delta().len(), 1);
        let stating = book
            .alive()
            .filter(|entry| !entry.get_srcuuids().is_empty())
            .count();
        let value = MarketData::from(book);
        let (allocations, batch) = counted(|| {
            let mut rows = MarketData::arrow_reader(vec![value], None, None).unwrap();
            rows.next().unwrap().unwrap()
        });
        (yggdryl::arrow::memory_size(&batch), allocations, stating)
    };
    for levels in [8, 1_024] {
        let (bare, bare_allocations, none) = laid_out(levels, false);
        let (sourced, sourced_allocations, stating) = laid_out(levels, true);
        println!(
            "book row at {levels} levels a side: {bare} bytes and {bare_allocations} allocations with no source, {sourced} bytes and {sourced_allocations} allocations with {stating} alive entries stating one"
        );
        assert_eq!(
            (none, stating),
            (0, levels * 16 - 1),
            "the update states none"
        );
        assert_eq!(
            sourced, bare,
            "a book row of {levels} levels a side took {sourced} bytes with sourced alive entries but {bare} without"
        );
        assert!(
            sourced_allocations <= bare_allocations + stating,
            "a book row of {levels} levels a side allocated {sourced_allocations} times with {stating} sourced alive entries but {bare_allocations} without"
        );
    }
}

/// A book's readings by kind - its resting orders, the orders and quotes
/// among its delta, and the executions among its events - borrow the entries
/// the book holds: nothing is allocated, at 8 entries or 1,024, the resting
/// orders walking every live quote to find none.
#[test]
fn a_book_s_readings_by_kind_allocate_nothing() {
    crate::install::installed();
    for entries in [8, 1_024] {
        let book = allocation_book(entries);
        assert_eq!(book.quotes().count(), entries);
        assert_eq!(book.ordlive().count(), 0);
        assert_eq!(book.events().count(), 0);
        free("reading a book's entries by kind", || {
            black_box(book.ordlive().map(black_box).count());
            black_box(book.orddelta().map(black_box).count());
            black_box(book.quotes().map(black_box).count());
            black_box(book.executions().map(black_box).count());
            black_box(book.controls().map(black_box).count());
            black_box(book.delta().map(black_box).count());
            black_box(book.events().map(black_box).count());
        });
    }
}

#[test]
fn one_book_update_does_not_allocate_per_live_entry() {
    crate::install::installed();
    let mut shallow = allocation_book(1);
    let shallow_update = allocation_book_operation("ALLOC-0", 2, 2, "Replaced");
    let (shallow_allocations, shallow_result) =
        counted(|| shallow.add_operations([shallow_update]));
    shallow_result.expect("the shallow update");

    let mut deep = allocation_book(128);
    let deep_update = allocation_book_operation("ALLOC-0", 2, 2, "Replaced");
    let (deep_allocations, deep_result) = counted(|| deep.add_operations([deep_update]));
    deep_result.expect("the deep update");
    assert_eq!(shallow.alive().count(), 1);
    assert_eq!(deep.alive().count(), 128);

    assert!(
        deep_allocations <= shallow_allocations + 4,
        "one update allocated {deep_allocations} times at depth 128 but {shallow_allocations} times at depth 1"
    );
    black_box((shallow, deep));
}

/// A book of `levels` distinct prices on each side - bids from 1000 down,
/// asks from 1001 up - with eight entries resting at every price: past the
/// four a vector grown by pushing holds in its first allocation, so a limit
/// that grew its identities rather than collecting them shows in the count.
fn allocation_book_levels(levels: usize) -> BookEvent {
    let mut book = BookEvent::new(1, "ALLOC");
    book.add_operations(allocation_level_entries(levels))
        .expect("the initial depth");
    book
}

/// The entries of [`allocation_book_levels`], in the order it adds them.
fn allocation_level_entries(levels: usize) -> Vec<MarketData> {
    ["Buy", "Sell"]
        .into_iter()
        .flat_map(|side| {
            (0..levels).flat_map(move |level| {
                (0..8).map(move |slot| allocation_level_entry(side, level, slot, 1, "New"))
            })
        })
        .collect()
}

/// The entry at `slot` of `level` on `side`, at `unix` in `state`.
fn allocation_level_entry(
    side: &str,
    level: usize,
    slot: usize,
    unix: i64,
    state: &str,
) -> MarketData {
    let offset = i64::try_from(level).expect("a small corpus");
    let (name, price) = if side == "Buy" {
        ("B", 1_000 - offset)
    } else {
        ("A", 1_001 + offset)
    };
    let mut event = QuoteEvent::at(unix);
    event.set_crosscode(format!("{name}-{level}-{slot}"));
    event.set_ticker(Some(SmolStr::new("ALLOC")), true);
    event.set_side(Side::read(side).expect("a shipped side"), true);
    event.set_price(Some(Decimal::from_int(price)), true);
    event.set_quantity(Some(Decimal::from_int(unix + offset)), true);
    event.set_state(State::read(state).expect("a shipped state"));
    event.finalize();
    MarketData::from(event)
}

/// Every reading of a book's bests and depth is read off the price levels
/// in place: nothing is built, at two depths.
#[test]
fn book_readings_allocate_nothing() {
    crate::install::installed();
    for levels in [8, 128] {
        let book = allocation_book_levels(levels);
        assert_eq!(book.limits(Side::Buy).count(), levels);
        free("best_price", || {
            black_box(black_box(&book).best_price(Side::Buy));
        });
        free("best_quantity", || {
            black_box(black_box(&book).best_quantity(Side::Sell));
        });
        free("is_crossed", || {
            black_box(black_box(&book).is_crossed());
        });
        free("is_locked", || {
            black_box(black_box(&book).is_locked());
        });
        free("spread", || {
            black_box(black_box(&book).spread());
        });
        free("bbo_midpoint", || {
            black_box(black_box(&book).bbo_midpoint());
        });
        free("median_quantity", || {
            black_box(black_box(&book).median_quantity());
        });
        free("imbalance", || {
            black_box(black_box(&book).imbalance(levels));
        });
        free("depth", || {
            black_box(black_box(&book).depth(Side::Buy, levels));
        });
    }
}

/// A side's limits cost one vector each - its entries' identities,
/// collected at their exact count - and nothing per entry, and the
/// `bidlimits` cell a book row states for the side one vector more, which
/// holds them, at two depths.
#[test]
fn book_limits_allocate_one_vector_per_limit() {
    crate::install::installed();
    for levels in [8, 128] {
        let book = allocation_book_levels(levels);
        let (allocations, count) = counted(|| book.limits(Side::Buy).map(black_box).count());
        assert_eq!(count, levels);
        assert_eq!(
            allocations, levels,
            "{levels} limits of eight entries each allocated {allocations} times"
        );
        let (allocations, cell) = counted(|| book.limits(Side::Buy).collect::<Vec<_>>());
        assert_eq!(cell.len(), levels);
        assert_eq!(
            allocations,
            levels + 1,
            "the bidlimits cell of {levels} limits allocated {allocations} times"
        );
    }
}

/// One order the market-data read pin reads back: every fact a row hands
/// back owning heap - the cross code, the sources, three identifiers (past
/// the map's inline two), the metadata and a book control - each at an
/// inline width, so a clone owns exactly what the row does. The control's
/// action and position are walk-time and read back as none; the box holding
/// its scope is the same one allocation.
fn allocation_market_order(index: usize) -> MarketData {
    let unix = 1_700_000_000_000_000_000 + i64::try_from(index).expect("a small corpus");
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(format!("ALLOC-ORDER-{index:06}"));
    order.set_seqnum(1 + u64::try_from(index).expect("a small corpus"));
    order.set_price(Some(Decimal::from_int(100)), true);
    order.set_quantity(Some(Decimal::from_int(10)), true);
    order.set_side(Side::read("Buy").expect("the shipped buy side"), true);
    order.set_ticker(Some(SmolStr::new("ALLOC")), true);
    order.set_state(State::read("New").expect("the shipped new state"));
    order.set_srcuuids(vec![Uuid::from_v8(7)]);
    order
        .insert_identifier(
            Identifier::new(IdKey::base(IdType::OrderId), &format!("ORDER-{index:06}")).unwrap(),
        )
        .expect("an identifier");
    order
        .insert_identifier(
            Identifier::new(IdKey::base(IdType::MdEntryId), &format!("ENTRY-{index:06}")).unwrap(),
        )
        .expect("an identifier");
    order
        .insert_identifier(
            Identifier::new(IdKey::base(IdType::ClOrdId), &format!("CL-{index:06}")).unwrap(),
        )
        .expect("an identifier");
    order.set_metadata(
        Some(
            [(SmolStr::new("venue"), SmolStr::new("XNAS"))]
                .into_iter()
                .collect(),
        ),
        true,
    );
    order.set_book(Some(BookRef {
        action: Some(MdUpdateAction::New),
        scope: Some(SmolStr::new("Symbol=ALLOC")),
        position: Some(1),
        entry_px: None,
        entry_size: None,
    }));
    order.finalize();
    MarketData::from(order)
}

/// A `marketdata` row read back costs the value it answers and nothing
/// else: the plan, the landing and the narrowed leaves are the stream's and
/// the batch's, paid before its first row, and every later row reads its
/// cells off the leaves, validates its identities and facts against the
/// rebuilt leaf without rebuilding what it compares, and allocates exactly
/// what a clone of the answer does - at two corpus sizes.
#[test]
fn a_market_data_row_read_allocates_only_what_it_hands_back() {
    crate::install::installed();
    for rows in [64, 512] {
        let values: Vec<MarketData> = (0..rows).map(allocation_market_order).collect();
        let (handed, ()) = counted(|| {
            for value in &values {
                black_box(value.clone());
            }
        });
        assert_eq!(handed % rows, 0, "every order owns the same heap");
        let batch = MarketData::arrow_reader(values, Some(rows), None)
            .expect("a reader")
            .next()
            .expect("one batch")
            .expect("the batch");
        let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        let mut read = MarketData::from_arrow_reader(source).expect("the canonical schema");
        black_box(read.next().expect("a first row").expect("the first order"));
        let (allocations, count) = counted(|| {
            read.by_ref().fold(0, |count, value| {
                black_box(value.expect("an order"));
                count + 1
            })
        });
        assert_eq!(count, rows - 1);
        assert_eq!(
            allocations,
            handed / rows * (rows - 1),
            "reading {count} rows allocated {allocations} times, but the orders they answer own {} allocations each",
            handed / rows
        );
    }
}

/// An order that also states an ISIN and two FX rates: the `isincode` and
/// `fxrates` columns a row read reads beside every other.
fn allocation_market_order_with_rates(index: usize) -> MarketData {
    let MarketData::OrderEvent(mut order) = allocation_market_order(index) else {
        unreachable!("the allocation order is an order event")
    };
    order
        .insert_securityid(
            Identifier::new(IdKey::base(IdType::Isin), "US0378331005").expect("an ISIN"),
        )
        .expect("a plain holder");
    order.set_fxrates(
        ["EUR", "JPY"]
            .into_iter()
            .map(|target| {
                (
                    yggdryl::Ccy::new(target).expect("a currency"),
                    Decimal::from_int(2),
                )
            })
            .collect(),
        true,
    );
    order.finalize();
    MarketData::from(order)
}

/// A row stating an ISIN and FX rates reads as the row without them does:
/// the value it answers and nothing else - the `isincode` compared in place
/// against `securityids`, the rates read into the one map the answer
/// holds and checked against it without a copy - at two corpus sizes.
#[test]
fn a_market_data_row_with_rates_reads_only_what_it_hands_back() {
    crate::install::installed();
    for rows in [64, 512] {
        let values: Vec<MarketData> = (0..rows).map(allocation_market_order_with_rates).collect();
        let (handed, ()) = counted(|| {
            for value in &values {
                black_box(value.clone());
            }
        });
        assert_eq!(handed % rows, 0, "every order owns the same heap");
        let batch = MarketData::arrow_reader(values, Some(rows), None)
            .expect("a reader")
            .next()
            .expect("one batch")
            .expect("the batch");
        let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        let mut read = MarketData::from_arrow_reader(source).expect("the canonical schema");
        black_box(read.next().expect("a first row").expect("the first order"));
        let (allocations, count) = counted(|| {
            read.by_ref().fold(0, |count, value| {
                black_box(value.expect("an order"));
                count + 1
            })
        });
        assert_eq!(count, rows - 1);
        assert_eq!(
            allocations,
            handed / rows * (rows - 1),
            "reading {count} rows allocated {allocations} times, but the orders they answer own {} allocations each",
            handed / rows
        );
    }
}

/// The key of a book is borrowed from the input whatever it states: its
/// ISIN identifier, its ticker, or the static number that states none.
#[test]
fn a_book_crosscode_allocates_nothing_for_any_input() {
    crate::install::installed();
    let order = allocation_market_order(1);
    let (allocations, key) = counted(|| black_box(order.book_crosscode().len()));
    assert_eq!(key, "ALLOC".len());
    assert_eq!(allocations, 0);
    let listed = allocation_market_order_with_rates(1);
    let (allocations, key) = counted(|| black_box(listed.book_crosscode().len()));
    assert_eq!(key, "US0378331005".len());
    assert_eq!(allocations, 0);
    let MarketData::OrderEvent(mut blank) = allocation_market_order(1) else {
        unreachable!("the allocation order is an order event")
    };
    blank.set_ticker(None, true);
    let (allocations, key) = counted(|| black_box(blank.book_crosscode().len()));
    assert_eq!(key, yggdryl::Isin::NONE.len());
    assert_eq!(allocations, 0);
}

/// A `marketdata` batch is laid out column by column from the typed
/// leaves: per row it costs the canonical clone its write check compares
/// against and nothing else - no cell is built as a value - so what the
/// columns cost beyond those clones grows with the buffers' doublings and
/// not with the rows, at two corpus sizes.
#[test]
fn a_market_data_batch_write_allocates_per_row_only_its_canonical_check() {
    crate::install::installed();
    let overhead = |rows: usize| {
        let values: Vec<MarketData> = (0..rows).map(allocation_market_order).collect();
        let (cloned, ()) = counted(|| {
            for value in &values {
                black_box(value.clone());
            }
        });
        let mut reader = MarketData::arrow_reader(values, Some(rows), None).expect("a reader");
        let (written, batch) = counted(|| reader.next().expect("one batch").expect("the batch"));
        assert_eq!(batch.num_rows(), rows);
        black_box(batch);
        written
            .checked_sub(cloned)
            .expect("a write costs at least its checks")
    };
    let (small, large) = (overhead(64), overhead(512));
    // 448 more rows add under one allocation per four of them: the
    // buffers' doublings, where a cell built per row would add hundreds.
    assert!(
        large < small + (512 - 64) / 4,
        "512 rows cost {large} allocations beyond their checks, against {small} for 64"
    );
}

/// `count` books of `ALLOC`, a second apart, each holding one order alive
/// and applied.
fn allocation_market_books(count: usize) -> Vec<MarketData> {
    (0..count)
        .map(|index| {
            let unix = 1 + i64::try_from(index).expect("a small corpus");
            let mut book = BookEvent::new(unix, "ALLOC");
            book.add_operations([allocation_book_operation("ALLOC-0", unix, 1, "New")])
                .expect("one order");
            MarketData::from(book)
        })
        .collect()
}

/// A book row's nested lists - its `alive`, `delta` and `events` rows and its
/// `bidlimits` and `asklimits` levels - are laid out straight into the
/// batch's items, no list built per row: past its canonical check, what a
/// batch of books costs grows with the buffers' doublings and not with the
/// rows, at two corpus sizes.
#[test]
fn a_book_batch_write_allocates_no_nested_list_per_row() {
    crate::install::installed();
    let overhead = |rows: usize| {
        let values = allocation_market_books(rows);
        // The check clones each entry, alive and applied, and the book's
        // event: what a clone of the book holds - its event and its delta's
        // vector, sharing the one entry applied, and its events' vector,
        // empty and so no allocation - and a clone of each of its entries,
        // alive and applied, less the delta's vector the check never builds.
        let (checked, ()) = counted(|| {
            for value in &values {
                let book = value.as_book_event().expect("a book");
                black_box(book.clone());
                for entry in book.alive().chain(book.delta()).chain(book.events()) {
                    black_box(entry.clone());
                }
            }
        });
        let checked = checked
            .checked_sub(rows)
            .expect("a delta's vector per book");
        let mut reader = MarketData::arrow_reader(values, Some(rows), None).expect("a reader");
        let (written, batch) = counted(|| reader.next().expect("one batch").expect("the batch"));
        assert_eq!(batch.num_rows(), rows);
        black_box(batch);
        written
            .checked_sub(checked)
            .expect("a write costs at least its checks")
    };
    let (small, large) = (overhead(64), overhead(512));
    // 448 more books add under one allocation per four of them: the
    // buffers' doublings, where a list built per row would add hundreds.
    assert!(
        large < small + (512 - 64) / 4,
        "512 books cost {large} allocations beyond their checks, against {small} for 64"
    );
}

/// A batch of the books a walk emits between its snapshot ticks - each
/// a delta book stating one order, its `alive` cell and its levels null -
/// costs as many allocations over a book 8 levels a side deep as over one
/// 1,024 deep: writing one lays out and checks its delta, never an entry
/// or a level it holds.
#[test]
fn a_batch_of_delta_books_writes_alike_at_8_and_1024_levels() {
    crate::install::installed();
    let write = |levels: usize| {
        let updates = (2..66).map(|unix| allocation_level_entry("Buy", 0, 0, unix, "Replaced"));
        let books = BookIterator::new(
            allocation_level_entries(levels).into_iter().chain(updates),
            0,
        )
        .unwrap()
        .skip(1)
        .map(|book| book.map(MarketData::from))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
        assert_eq!(books.len(), 64);
        let mut reader = MarketData::arrow_reader(books, Some(64), None).expect("a reader");
        let (allocations, batch) =
            counted(|| reader.next().expect("one batch").expect("the batch"));
        assert_eq!(batch.num_rows(), 64);
        assert_eq!(batch.column_by_name("alive").unwrap().null_count(), 64);
        black_box(batch);
        allocations
    };
    let (shallow, deep) = (write(8), write(1_024));
    assert!(
        deep <= shallow + 2,
        "64 delta books cost {deep} allocations at 1,024 levels but {shallow} at 8"
    );
}

/// A filtered book walk lays out one batch per pull ahead for its filter,
/// so what the filter costs grows with the batch's buffers and not with
/// the inputs it judges, at two corpus sizes.
#[test]
fn a_filtered_book_walk_lays_out_one_batch_per_pull_ahead() {
    crate::install::installed();
    let walk = |inputs: usize, filter: Option<&str>| {
        let operations: Vec<MarketData> = (0..inputs)
            .map(|index| {
                let unix = 1 + i64::try_from(index).expect("a small corpus");
                allocation_book_operation(format!("ALLOC-{index}"), unix, 1, "New")
            })
            .collect();
        let books = BookIterator::new(operations.into_iter(), 0).unwrap();
        let books = match filter {
            Some(filter) => books.with_filter(filter).unwrap(),
            None => books,
        };
        let (allocations, count) = counted(|| {
            books.fold(0, |count, book| {
                black_box(book.unwrap());
                count + 1
            })
        });
        assert_eq!(count, inputs);
        allocations
    };
    let overhead = |inputs: usize| {
        walk(inputs, Some("side = 'BUYS'"))
            .checked_sub(walk(inputs, None))
            .expect("a filter costs at least nothing")
    };
    let (small, large) = (overhead(64), overhead(512));
    // 448 more inputs add under one allocation per sixteen of them: the
    // batch's and the pulled leaves' doublings, where a cell or a batch
    // per input would add hundreds.
    assert!(
        large < small + (512 - 64) / 16,
        "filtering 512 inputs cost {large} allocations, against {small} for 64"
    );
}

/// `rows` orders as one record column of a registered venue code, a side
/// and a count: three venue runs, changing at a third and at two thirds
/// of the rows, and the side buying over the first half and selling over
/// the second, so keyed by both they cut four windows at any row count.
fn coded_orders(rows: usize) -> Serie {
    use arrow_array::{Int64Array, StringArray, StructArray, UInt8Array};
    use arrow_schema::DataType as ArrowDataType;

    let root = DataType::from(
        StructType::from_fields([
            DataType::Mic.required_field("venue"),
            Side::dtype().required_field("side"),
            DataType::Int64.required_field("count"),
        ])
        .expect("three children"),
    )
    .required_field("order");
    let ArrowDataType::Struct(fields) = root
        .clone()
        .into_arrow_field()
        .expect("a projection")
        .data_type()
        .clone()
    else {
        panic!("a record projects to a struct")
    };
    let records = StructArray::try_new(
        fields,
        vec![
            Arc::new(StringArray::from(
                (0..rows)
                    .map(|index| ["XNAS", "XNYS", "XPAR"][index * 3 / rows])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt8Array::from(
                (0..rows)
                    .map(|index| if index < rows / 2 { 1 } else { 2 })
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                (0..rows)
                    .map(|index| i64::try_from(index).expect("a row count"))
                    .collect::<Vec<_>>(),
            )),
        ],
        None,
    )
    .expect("an order record");
    Serie::from_arrow_array(Some(&root), Arc::new(records), ArrowCastOptions::new())
        .expect("orders")
}

/// Four owned two-cell keys beside one-child payload slices. The registered-code comparison values are built once; the owned contexts add no per-row work. Pinned at 64 and 4096 rows.
const WINDOW_BY_CODED_KEY: usize = 89;

#[test]
fn a_record_key_with_a_code_child_cuts_with_a_constant_count() {
    crate::install::installed();
    // One code cell once sent the whole key to the values' order, one run
    // built a row; the record rung compares each cell on its own rung, so
    // only the code builds its values, once, and the call costs the same at
    // 64 rows and at 4,096.
    let venue_side: yggdryl::Selector = "venue, side".parse().expect("a selector");
    for rows in [64_usize, 4_096] {
        let orders = coded_orders(rows);
        // Once outside every count, so no process-wide first use is charged.
        assert_eq!(
            orders.window_by(&venue_side, false).expect("windows").len(),
            4
        );
        let (build, windows) = counted(|| {
            black_box(&orders)
                .window_by(&venue_side, false)
                .expect("windows")
        });
        assert_eq!(windows.len(), 4);
        drop(windows);
        let (walk, walked) = counted(|| {
            black_box(&orders)
                .window_by(&venue_side, false)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
        assert_eq!(walked, 4);
        assert_eq!(
            (build, walk),
            (WINDOW_BY_CODED_KEY, WINDOW_BY_CODED_KEY),
            "window_by a venue code and a side over {rows} records: one plan a call, one key a window"
        );
    }
}

/// A registered enum kind's column reads its cells as the kind's own values
/// without allocating: a member's code is read off its buffer, and the kind
/// is read off the field, never looked up.
#[test]
fn a_registered_column_cell_read_allocates_nothing() {
    crate::install::installed();
    for rows in [64_usize, 4_096] {
        let side = Side::dtype();
        let members = Serie::from_scalars(
            side.clone().nullable_field("side"),
            (0..rows).map(|row| {
                if row % 5 == 4 {
                    Scalar::Null
                } else {
                    side.scalar("BUYS").expect("a side")
                }
            }),
        )
        .expect("an enum column");
        let dtype = members.field().expect("a column").dtype().clone();
        free(&format!("reading a {dtype} cell at {rows} rows"), || {
            black_box(black_box(&members).scalar(0).expect("a cell"));
        });
        free(
            &format!("reading an absent {dtype} cell at {rows} rows"),
            || {
                black_box(black_box(&members).scalar(4).expect("a cell"));
            },
        );
        assert_eq!(members.scalar(0).expect("a cell").enum_name(), Some("BUYS"));
    }
}

/// A cast into a registered enum kind reads the kind once per array and runs
/// its per-row check against that reading: a column costs its output
/// buffers whatever its length, never a lookup or a value per row.
#[test]
fn a_registered_cast_ingest_costs_the_same_at_any_length() {
    crate::install::installed();
    let side = Field::new("side", Side::dtype(), true);
    let text = Field::new("side", DataType::utf8(), true);
    let names = ArrowCastPlan::compile(&text, &side, ArrowCastOptions::new())
        .expect("text casts into an enum");
    let numbers = Field::new("side", DataType::Int64, true);
    let members = ArrowCastPlan::compile(&numbers, &side, ArrowCastOptions::new())
        .expect("integers cast into an enum");
    let spellings = ["BUYS", "SELL", "SSHT"];
    let mut name_counts = Vec::new();
    let mut member_counts = Vec::new();
    for rows in [64_usize, 4_096] {
        let column = Serie::from_scalars(
            text.clone(),
            (0..rows).map(|row| Scalar::from(spellings[row % spellings.len()])),
        )
        .expect("a text column");
        let cast = || names.apply(&column).expect("the column casts");
        drop(cast());
        let (allocations, landed) = counted(cast);
        assert_eq!(landed.len(), rows);
        assert_eq!(landed.scalar(1).expect("a row").enum_name(), Some("SELL"));
        name_counts.push(allocations);

        let column = Serie::from_scalars(
            numbers.clone(),
            (0..rows).map(|row| Scalar::from(i64::try_from(row % 3).expect("small"))),
        )
        .expect("an integer column");
        let cast = || members.apply(&column).expect("the column casts");
        drop(cast());
        let (allocations, landed) = counted(cast);
        assert_eq!(landed.len(), rows);
        assert_eq!(landed.scalar(2).expect("a row").enum_name(), Some("SELL"));
        member_counts.push(allocations);
    }
    assert_eq!(
        name_counts[0], name_counts[1],
        "a text to enum cast cost {name_counts:?} allocations at 64 and 4096 rows"
    );
    assert_eq!(
        member_counts[0], member_counts[1],
        "an integer to enum cast cost {member_counts:?} allocations at 64 and 4096 rows"
    );
}

/// One `marketdata` batch: two orders, a trade of two executions and a book.
fn view_corpus() -> arrow_array::RecordBatch {
    let (root, executions) = allocation_trade_parts(2);
    let trade = TradeEvent::from_parts(&root, executions).expect("the trade");
    let values = (0..2).map(allocation_market_order).chain([
        MarketData::from(trade),
        MarketData::from(allocation_book(1)),
    ]);
    let mut rows = MarketData::arrow_reader(values, None, None).expect("the row field");
    let batch = rows
        .next()
        .expect("one batch")
        .expect("the leaves are canonical");
    assert!(rows.next().is_none());
    batch
}

#[test]
fn a_view_plan_is_compiled_once_per_stream() {
    crate::install::installed();
    use yggdryl_market::graph::MarketView;

    let batch = view_corpus();
    let isin: FieldPath = "securityids['isin'] as isin".parse().expect("a lift");
    for (view, lifts) in [
        (MarketView::Orders, vec![isin]),
        (MarketView::Trades, Vec::new()),
        (MarketView::Books, Vec::new()),
    ] {
        // An endless stream of the one batch: the view binds when it is
        // applied, and every batch it is then pulled for costs one batch.
        let source = {
            let batch = batch.clone();
            std::iter::repeat_with(move || Ok(batch.clone()))
        };
        let reader: yggdryl::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
            source,
            batch.schema(),
        ));
        let mut viewed =
            MarketData::apply_view(&view, &lifts, reader).expect("the view binds once");
        let (once, repeated) = counted_once_and_repeated(|| {
            black_box(
                viewed
                    .next()
                    .expect("an endless stream")
                    .expect("the view answers"),
            );
        });
        assert!(
            once > 0,
            "{view}: a batch that allocates nothing moved nothing"
        );
        assert_eq!(
            repeated,
            once * 1_000,
            "{view}: one batch cost {once} and a thousand cost {repeated}"
        );
        eprintln!("view_{view}: per batch {once}");
    }
}

/// One value per prebuilt shared field, built through the datatype's own
/// contract so each names exactly the datatype it is pinned under.
///
/// `Variant` keeps a shared field but no value names it - a variant value
/// describes itself - so it is the one prebuilt id with nothing to infer.
fn prebuilt_values() -> Vec<(DataTypeId, Scalar)> {
    use yggdryl_market::{MarketDataKind, MarketDataType, TimeInForce};

    let seeds: [(DataTypeId, Scalar); 58] = [
        (DataTypeId::Null, Scalar::Null),
        (DataTypeId::Boolean, Scalar::from(true)),
        (DataTypeId::Int8, Scalar::from(1_i64)),
        (DataTypeId::Int16, Scalar::from(1_i64)),
        (DataTypeId::Int32, Scalar::from(1_i64)),
        (DataTypeId::Int64, Scalar::from(1_i64)),
        (DataTypeId::UInt8, Scalar::from(1_i64)),
        (DataTypeId::UInt16, Scalar::from(1_i64)),
        (DataTypeId::UInt32, Scalar::from(1_i64)),
        (DataTypeId::UInt64, Scalar::from(1_i64)),
        (DataTypeId::Float16, Scalar::from(1.5_f64)),
        (
            DataTypeId::Decimal,
            Scalar::Decimal("82.5".parse().unwrap()),
        ),
        (
            DataTypeId::BigDecimal,
            Scalar::BigDecimal(yggdryl::BigDecimal::from_int(3)),
        ),
        (DataTypeId::Float32, Scalar::from(1.5_f64)),
        (DataTypeId::Float64, Scalar::from(1.5_f64)),
        (DataTypeId::Date32, Scalar::date32(19_723)),
        (DataTypeId::Date64, Scalar::date32(19_723)),
        (DataTypeId::Binary, Scalar::from(&b"ABC"[..])),
        (DataTypeId::LargeBinary, Scalar::from(&b"ABC"[..])),
        (DataTypeId::BinaryView, Scalar::from(&b"ABC"[..])),
        (DataTypeId::LargeBinaryView, Scalar::from(&b"ABC"[..])),
        (DataTypeId::Utf8String, Scalar::from("AAPL")),
        (DataTypeId::LargeUtf8String, Scalar::from("AAPL")),
        (DataTypeId::Utf8StringView, Scalar::from("AAPL")),
        (DataTypeId::LargeUtf8StringView, Scalar::from("AAPL")),
        (DataTypeId::AsciiString, Scalar::from("AAPL")),
        (DataTypeId::LargeAsciiString, Scalar::from("AAPL")),
        (DataTypeId::AsciiStringView, Scalar::from("AAPL")),
        (DataTypeId::LargeAsciiStringView, Scalar::from("AAPL")),
        (DataTypeId::Cp1252String, Scalar::from("AAPL")),
        (DataTypeId::LargeCp1252String, Scalar::from("AAPL")),
        (DataTypeId::Cp1252StringView, Scalar::from("AAPL")),
        (DataTypeId::LargeCp1252StringView, Scalar::from("AAPL")),
        (DataTypeId::Country, Scalar::from("US")),
        (DataTypeId::Ccy, Scalar::from("USD")),
        (DataTypeId::Mic, Scalar::from("XNAS")),
        (DataTypeId::Cfi, Scalar::from("ESVUFR")),
        (DataTypeId::Isin, Scalar::from("US0378331005")),
        (DataTypeId::Cusip, Scalar::from("037833100")),
        (DataTypeId::Sedol, Scalar::from("B0YBKJ7")),
        (DataTypeId::Bbg, Scalar::from("AAPL US EQUITY")),
        (DataTypeId::Ric, Scalar::from("AAPL.OQ")),
        (DataTypeId::Forex, Scalar::from("EURUSD")),
        (DataTypeId::Figi, Scalar::from("BBG000BLNQ16")),
        (DataTypeId::Lei, Scalar::from("HWUPKR0MPOU8FGXBT394")),
        (DataTypeId::Bic, Scalar::from("DEUTDEFF500")),
        (DataTypeId::Elf, Scalar::from("8888")),
        (DataTypeId::Dti, Scalar::from("X9J9K872S")),
        (DataTypeId::Fisn, Scalar::from("ACME CORP/SH")),
        (Side::ID, Scalar::from("1")),
        (DataTypeId::State, Scalar::from("NEW")),
        (MarketDataKind::ID, Scalar::from("ORDR")),
        (TimeInForce::ID, Scalar::from("DAY")),
        (MarketDataType::ID, Scalar::from("ORDLIMIT")),
        (DataTypeId::Unit, Scalar::from("Shares")),
        (
            DataTypeId::Uuid,
            Scalar::from("123e4567-e89b-12d3-a456-426614174000"),
        ),
        (DataTypeId::Version, Scalar::from("5.0.2")),
        (DataTypeId::Timezone, Scalar::from("America/New_York")),
    ];
    let mut values: Vec<(DataTypeId, Scalar)> = seeds
        .into_iter()
        .map(|(id, seed)| {
            let dtype = DataType::from_str(id.as_str()).expect("the id names a datatype");
            let value = dtype
                .scalar(seed)
                .expect("the seed is a value of the datatype");
            assert_eq!(value.dtype().expect("a leaf names itself"), dtype, "{id:?}");
            (id, value)
        })
        .collect();
    let url = DataType::url()
        .scalar("https://example.com/a")
        .expect("the text is a URL");
    values.push((DataTypeId::Url, url));
    let urn = DataType::urn()
        .scalar("URN:ISBN:0451450523")
        .expect("the text is a URN");
    values.push((DataTypeId::Urn, urn));
    // A MIME type and a media type are pinned outside the seed loop for the
    // same reason a URL and a URN are: their canonical text is not what was
    // written.
    let mime = DataType::MimeType
        .scalar("application/json")
        .expect("the text is a MIME type");
    values.push((DataTypeId::MimeType, mime));
    let media = DataType::MediaType
        .scalar("application/json; charset=utf-8")
        .expect("the text is a media type");
    values.push((DataTypeId::MediaType, media));
    // Every prebuilt id is either pinned here or one no value ever names:
    // a variant is the one datatype with no value of its own.
    let unnamed = [DataTypeId::Variant];
    let pinned: std::collections::HashSet<DataTypeId> = values.iter().map(|(id, _)| *id).collect();
    for id in DataTypeId::all() {
        let prebuilt = !id.is_parameterized()
            && DataType::from_str(id.as_str()).is_ok_and(|dtype| dtype.id() == id);
        if prebuilt && !unnamed.contains(&id) {
            assert!(pinned.contains(&id), "{id:?} has a shared field and no pin");
        }
    }
    values
}

#[test]
fn inferring_a_field_scalar_borrows_a_prebuilt_field_and_allocates_nothing() {
    crate::install::installed();
    // The typed view borrows the field the crate keeps for the datatype, so
    // typing a leaf value that names itself builds no field and no copy of
    // one - once, and a thousand times, for every prebuilt id.
    for (id, value) in prebuilt_values() {
        free(&format!("inferring a typed {id:?}"), || {
            black_box(FieldScalar::infer(black_box(&value).clone()).expect("the value infers"));
        });
    }
    // The plain string and the plain byte value name the family's default
    // layout, whose field the crate keeps beside the other prebuilt ones.
    for value in [Scalar::from("x"), Scalar::from(vec![1_u8])] {
        free(&format!("inferring a typed {}", value.kind()), || {
            black_box(FieldScalar::infer(black_box(&value).clone()).expect("the value infers"));
        });
    }
    // A parameterized leaf is interned on its first ask and borrowed after.
    for dtype in [
        DataType::decimal128(10, 2).expect("a valid decimal"),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("a valid instant"),
        DataType::fixed_ascii(4).unwrap(),
    ] {
        free(&format!("looking up the shared field of {dtype}"), || {
            black_box(black_box(&dtype).shared_field().expect("an interned field"));
        });
    }
}

/// One column per leaf a cell read must build nothing for.
fn typed_leaf_columns() -> Vec<Serie> {
    let zone = Timezone::from_str("Europe/Paris").expect("a zone");
    let at = DataType::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: zone,
    };
    [
        (DataType::Int64, Scalar::from(42_i64)),
        (DataType::Boolean, Scalar::from(true)),
        (DataType::Float64, Scalar::from(1.5_f64)),
        (
            DataType::decimal(10, 2).expect("a decimal"),
            Scalar::decimal128(1_025, 2),
        ),
        (DataType::Date64, Scalar::date64(86_400_000)),
        (
            at,
            Scalar::datetime64(1_700_000_000_000_000, TimeUnit::Microsecond, zone)
                .expect("an instant"),
        ),
        (
            DataType::Duration32(TimeUnit::Second),
            Scalar::duration32(90, TimeUnit::Second).expect("a duration"),
        ),
        (DataType::utf8(), Scalar::from("AAPL")),
        // A code and the three enum storages: a cell of a registered kind is
        // read as its typed leaf is, with nothing built.
        (
            DataType::from_str("isin").expect("a kind"),
            DataType::from_str("isin")
                .expect("a kind")
                .scalar("US0378331005")
                .expect("an ISIN"),
        ),
        (
            DataType::from_str("side").expect("a kind"),
            DataType::from_str("side")
                .expect("a kind")
                .scalar("BUYS")
                .expect("a side"),
        ),
        (
            DataType::from_str("state").expect("a kind"),
            DataType::from_str("state")
                .expect("a kind")
                .scalar("PENDING_NEW")
                .expect("a state"),
        ),
        (
            DataType::from_str("marketdatatype").expect("a kind"),
            DataType::from_str("marketdatatype")
                .expect("a kind")
                .scalar("ORDLIMIT")
                .expect("a type"),
        ),
    ]
    .into_iter()
    .map(|(dtype, value)| {
        Serie::from_scalars(dtype.nullable_field("cell"), [value, Scalar::Null])
            .expect("a leaf column")
    })
    .collect()
}

#[test]
fn a_leaf_cell_read_allocates_nothing() {
    crate::install::installed();
    // A leaf reads its own typed buffer through the reading its field
    // resolved where it landed: one buffer read and one constructor, no
    // downcast and no value that owns memory.
    for column in typed_leaf_columns() {
        let dtype = column.field().expect("a column").dtype().clone();
        free(&format!("reading a {dtype} cell"), || {
            black_box(black_box(&column).scalar(0).expect("a cell"));
        });
        free(&format!("reading an absent {dtype} cell"), || {
            black_box(black_box(&column).scalar(1).expect("a cell"));
        });
    }
}

#[test]
fn a_record_row_costs_its_run_and_nothing_per_cell() {
    crate::install::installed();
    // A record row is one run of its cells: the run's storage is the one
    // allocation, and every cell under it is read without one.
    let columns = typed_leaf_columns();
    let root = StructType::from_fields(columns.iter().enumerate().map(|(index, column)| {
        column
            .field()
            .expect("a column")
            .clone()
            .with_name(format!("cell_{index}"))
    }))
    .map(DataType::from)
    .expect("a record")
    .required_field("row");
    let rows = [0_usize, 1].map(|row| {
        Scalar::from_sequence(
            columns
                .iter()
                .map(|column| column.scalar(row).expect("a cell")),
        )
    });
    let records = Serie::from_scalars(root, rows).expect("a record column");
    costs("reading a record row", 1, || {
        black_box(black_box(&records).scalar(0).expect("a row"));
    });
}

/// A typed leaf the walk re-keys onto its chain pays for what moved and
/// nothing else, its finalize allocating nothing: over followers of a buy
/// live element stored `10:1:J`, a follower of the chain's side stating
/// another code costs one allocation - the code's text, the prefix already
/// the holder's - and a side-less one three - two for the side it is lent,
/// the leg `side_moved` quotes and the code `reprefix` writes under it, and
/// one for the code; one already holding the side and the code costs
/// nothing and moves nothing. Read off the run at 8 and 1024 followers.
#[cfg(feature = "internals")]
#[test]
fn a_rekey_costs_the_chains_cross_code_on_a_typed_leaf() {
    crate::install::installed();
    use yggdryl_market::internals::graph_iterator::rekeyed;

    let mut live = OrderEvent::at(1);
    live.set_crosscode("J".to_owned());
    live.set_side(Side::Buy, true);
    live.finalize();
    assert_eq!(live.get_crosscode(), "10:1:J");
    let identity = live.get_crossuuid();
    let follower = |side: Option<Side>, code: &str| {
        let mut event = OrderEvent::at(2);
        event.set_crosscode(code.to_owned());
        if let Some(side) = side {
            event.set_side(side, true);
        }
        event.set_price(Some(Decimal::from_int(99)), true);
        event.set_quantity(Some(Decimal::from_int(5)), true);
        event.finalize();
        event
    };
    for followers in [8, 1024] {
        for (what, side, code, each, moves) in [
            (
                "the chain's side, another code",
                Some(Side::Buy),
                "K",
                1,
                true,
            ),
            ("no side, another code", None, "K", 3, true),
            ("the chain's side and code", Some(Side::Buy), "J", 0, false),
        ] {
            let mut held: Vec<OrderEvent> = (0..followers).map(|_| follower(side, code)).collect();
            let (allocations, moved) = counted(|| {
                held.iter_mut()
                    .map(|event| rekeyed(event, &live, identity))
                    .filter(|moved| *moved)
                    .count()
            });
            assert_eq!(moved, if moves { followers } else { 0 }, "{what}");
            assert_eq!(
                allocations,
                each * followers,
                "{what}: {allocations} over {followers} followers"
            );
            assert!(
                held.iter().all(|event| event.get_side() == Side::Buy
                    && event.get_crosscode() == "10:1:J"
                    && event.get_crossuuid() == identity),
                "{what}: every follower stands under the chain"
            );
        }
    }
}
#[test]
fn instrument_codes_construct_and_classify_without_allocating() {
    crate::install::installed();
    use yggdryl::{Bbg, Cfi, Cusip, Figi, Isin, Ric, Sedol};
    free("long Bloomberg validation", || {
        assert!(Bbg::is_canonical("AAPL US Equity Long Identifier"));
    });
    free("RIC construction", || {
        black_box(Ric::new(black_box("0005.HK")).unwrap());
    });
    let ric = Ric::new("VOD.L").unwrap();
    free("RIC validation and exchange code", || {
        assert!(Ric::is_canonical(black_box("0#.FTSE")));
        assert_eq!(black_box(&ric).exchange_code(), Some("L"));
    });
    free("ISIN construction", || {
        std::hint::black_box(Isin::new("us0378331005").unwrap());
    });
    // The readings a merge ranks by are arithmetic over the text and one
    // binary search of the listed prefixes: no value is built.
    free("ISIN closing", || {
        assert!(Isin::is_closed(black_box("US0378331005")));
        assert!(!Isin::is_closed(black_box("XX0000000001")));
    });
    free("ISIN rank", || {
        assert_eq!(Isin::rank_of(black_box("US0378331005")), 2);
        assert_eq!(Isin::rank_of(black_box("XX0000000001")), 0);
    });
    free("identifier type rank", || {
        assert_eq!(IdType::Isin.rank(black_box("US0378331005")), 2);
        assert_eq!(IdType::Cusip.rank(black_box("037833101")), 0);
        assert_eq!(IdType::OrderId.rank(black_box("O-1")), 1);
    });
    free("CUSIP construction", || {
        std::hint::black_box(Cusip::new("037833100").unwrap());
    });
    free("SEDOL construction", || {
        std::hint::black_box(Sedol::new("b0swjx3").unwrap());
    });
    let figi = Figi::new("BBG000BLNQ16").unwrap();
    free("FIGI construction", || {
        black_box(Figi::new("bbg000blnq16").unwrap());
    });
    free("FIGI clone", || {
        black_box(figi.clone());
    });
    free("CFI validation", || {
        assert!(Cfi::is_classified("ESVUFR"));
    });
    free("CFI merging", || {
        assert_eq!(Cfi::refined("ESXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
    });
    free("CFI inference", || {
        assert_eq!(Cfi::coarse('E', Some('S')).as_deref(), Some("ESXXXX"));
    });
}

#[test]
fn identifier_reads_and_inline_inserts_allocate_nothing() {
    crate::install::installed();
    use yggdryl_market::Identifiers;
    // A key - a source and a type - and a value, 24 bytes each: 72, where
    // the retired `parent` and `orig` were two more 24-byte strings at 120.
    assert_eq!(std::mem::size_of::<Identifier>(), 72);
    assert_eq!(std::mem::size_of::<Identifiers>(), IDENTIFIERS_SIZE);
    let mut ids = Identifiers::new();
    ids.insert(Identifier::new(IdKey::base(IdType::Account), "ACC-1").unwrap());
    let venue: IdSource = "venue".parse().unwrap();
    let user = IdKey::new(venue.clone(), IdType::UserId);
    ids.insert(Identifier::new(user.clone(), "U-1").unwrap());
    free("an Identifiers read of a held type", || {
        assert_eq!(ids.get(black_box(&IdType::Account)), Some("ACC-1"));
    });
    free("an Identifiers read of a held type and source", || {
        assert_eq!(ids.get_from(black_box(&user)), Some("U-1"));
    });
    free("an Identifiers read of an absent type", || {
        assert_eq!(ids.get(black_box(&IdType::DeskId)), None);
    });
    // A word is folded on the stack: a member is static text and any other
    // word within SmolStr's inline width is held inline, so reading a
    // spelling into its type costs nothing either.
    free("an IdType read of a member through an alias", || {
        assert_eq!(
            black_box("ISIN_Number").parse::<IdType>().unwrap(),
            IdType::Isin
        );
    });
    free("an IdType read of a short word no member names", || {
        assert!(
            !black_box("House Code")
                .parse::<IdType>()
                .unwrap()
                .is_known()
        );
    });
    free("an IdSource read of a member", || {
        assert_eq!(
            black_box("PROPRIETARY").parse::<IdSource>().unwrap(),
            IdSource::Proprietary
        );
    });
    // A real number replacing a masked one under the base key, and the
    // explicit restatement of the masked one over it, each move in place:
    // the slot is held, the value inline, the rank read off the text.
    let mut ranked = Identifiers::new();
    assert!(ranked.insert(Identifier::new(IdKey::base(IdType::Isin), "XX0000000001").unwrap()));
    free("a real number replacing a masked one", || {
        assert!(ranked.insert(
            Identifier::new(IdKey::base(IdType::Isin), black_box("US0378331005")).unwrap()
        ));
        assert!(
            ranked.set(
                Identifier::new(IdKey::base(IdType::Isin), black_box("XX0000000001")).unwrap()
            )
        );
        black_box(&ranked);
    });
    // A codified type and source are static text and a 23-byte value is the
    // widest SmolStr holds inline: two identifiers cost the map its one
    // backing and nothing each, the base key the named one fills included.
    costs("two identifiers into a new set", 1, || {
        let mut ids = Identifiers::new();
        assert!(ids.insert(Identifier::new(IdKey::base(IdType::OrderId), "Z").unwrap()));
        assert!(
            ids.insert(
                Identifier::new(
                    IdKey::new(
                        black_box(IdSource::Proprietary),
                        black_box(IdType::ExecutingTrader)
                    ),
                    black_box("ABCDEFGHIJKLMNOPQRSTUVW")
                )
                .unwrap()
            )
        );
        assert_eq!(ids.len(), 3);
        black_box(&ids);
    });
    // A value under the bic or the legalentityidentifier source is held as
    // the code and ranked by it in place: an eleven-byte BIC and a
    // twenty-byte LEI stay inline, and a closing LEI replacing a typo under
    // its key moves no slot.
    let lei = |value: &str| {
        Identifier::new(
            IdKey::new(IdSource::LegalEntityIdentifier, IdType::ClientId),
            value,
        )
    };
    let mut coded: Identifiers = [lei("HWUPKR0MPOU8FGXBT395").unwrap()].into_iter().collect();
    free(
        "a BIC and an LEI held and ranked under their sources",
        || {
            let firm = Identifier::new(
                IdKey::new(black_box(IdSource::Bic), black_box(IdType::ExecutingFirm)),
                black_box("deutdeff500"),
            )
            .unwrap();
            assert_eq!(firm.value(), "DEUTDEFF500");
            black_box(coded.insert(lei(black_box("hwupkr0mpou8fgxbt394")).unwrap()));
            black_box((&firm, &coded));
        },
    );
    assert_eq!(
        coded.get_from(&IdKey::new(
            IdSource::LegalEntityIdentifier,
            IdType::ClientId
        )),
        Some("HWUPKR0MPOU8FGXBT394")
    );
}

/// An ISIN registry learns a statement of a known instrument that says
/// nothing new but the instant it was met - its `lastunix`, moved in place
/// on the row it holds - then the same statement again, which moves
/// nothing, fills an element that leaves nothing unsaid and looks a row up
/// by its ISIN without allocating, whatever its size.
#[test]
fn an_isin_registry_reads_and_learns_a_known_instrument_without_allocating() {
    crate::install::installed();
    use yggdryl::Isin;
    use yggdryl_market::graph::{Market, OrderEvent};
    use yggdryl_market::{IsinEntry, IsinRegistry};
    let numbered = |number: usize| {
        let body = format!("FR{number:09}");
        let digit = Isin::closing_digit(&body).unwrap();
        format!("{body}{digit}")
    };
    for size in [64, 4_096] {
        let mut registry = IsinRegistry::new();
        for number in 0..size {
            registry
                .merge(
                    IsinEntry::new(Isin::new(numbered(number)).unwrap())
                        .with_updunix(Some(1))
                        .try_with_code(IdType::Common, &format!("C-{number}"))
                        .unwrap()
                        .try_with_code(IdType::Ric, &format!("R{number}.X"))
                        .unwrap(),
                )
                .unwrap();
        }
        let isin = numbered(7);
        let mut stated = OrderEvent::at(2);
        for (kind, value) in [
            (IdType::Isin, isin.as_str()),
            (IdType::Common, "C-7"),
            (IdType::Ric, "R7.X"),
        ] {
            stated
                .insert_securityid(Identifier::new(IdKey::base(kind), value).unwrap())
                .unwrap();
        }
        let (meeting, met) = counted(|| registry.learn(black_box(&stated)));
        assert!(met, "the instant the instrument was met moves");
        assert_eq!(
            meeting, 0,
            "learning only the instant at {size} instruments"
        );
        free(
            &format!("learning nothing new at {size} instruments"),
            || {
                assert!(!registry.learn(black_box(&stated)));
            },
        );
        free(&format!("filling nothing at {size} instruments"), || {
            assert!(!registry.fill(black_box(&mut stated)));
        });
        free(&format!("a row by its ISIN at {size} instruments"), || {
            assert!(registry.get(black_box(isin.as_str())).is_some());
        });
    }
}

/// `size` instruments numbered from zero, each with a CFI code, a market,
/// a ticker of its own - `T` and its number - a common code and a RIC.
fn isin_registry_of(size: usize) -> yggdryl_market::IsinRegistry {
    use yggdryl::{Cfi, Isin, Mic};
    use yggdryl_market::{IsinEntry, IsinRegistry};
    let mut registry = IsinRegistry::new();
    for number in 0..size {
        registry
            .merge(
                IsinEntry::new(Isin::new(numbered_isin("FR", number)).unwrap())
                    .with_updunix(Some(1))
                    .with_cficode(Some(Cfi::new("ESVUFR").unwrap()))
                    .with_miccode(Some(Mic::new("XPAR").unwrap()))
                    .with_ticker(Some(smol_str::SmolStr::new(format!("T{number}"))))
                    .try_with_code(IdType::Common, &format!("C-{number}"))
                    .unwrap()
                    .try_with_code(IdType::Ric, &format!("R{number}.PA"))
                    .unwrap(),
            )
            .unwrap();
    }
    registry
}

/// The ISIN numbered `number` under the two-letter `prefix`.
fn numbered_isin(prefix: &str, number: usize) -> String {
    let body = format!("{prefix}{number:09}");
    let digit = yggdryl::Isin::closing_digit(&body).unwrap();
    format!("{body}{digit}")
}

/// A ticker leads back to its row through the ticker index without
/// allocating, whatever the registry holds - on no market, on its own, and
/// to none on another - and a fill keyed by the ticker alone, the ISIN
/// derived first, costs the same at 64 instruments as at 4,096: nothing it
/// builds is per instrument the registry holds.
#[test]
fn an_isin_registry_reads_and_fills_by_ticker_alike_at_every_size() {
    crate::install::installed();
    use yggdryl::Mic;
    use yggdryl_market::graph::{Market, OrderEvent};
    let paris = Mic::new("XPAR").unwrap();
    let london = Mic::new("XLON").unwrap();
    let mut fills = Vec::with_capacity(2);
    for size in [64, 4_096] {
        let registry = isin_registry_of(size);
        let isin = numbered_isin("FR", 7);
        assert_eq!(
            registry
                .get_by_ticker("T7", Some(&paris))
                .map(|row| row.isin().as_str()),
            Some(isin.as_str()),
            "the ticker T7 names one row at {size} instruments"
        );
        free(
            &format!("a row by its ticker at {size} instruments"),
            || {
                assert!(registry.get_by_ticker(black_box("T7"), None).is_some());
            },
        );
        free(
            &format!("a row by its ticker on its market at {size} instruments"),
            || {
                assert!(
                    registry
                        .get_by_ticker(black_box("T7"), Some(&paris))
                        .is_some()
                );
            },
        );
        free(
            &format!("a ticker on another market at {size} instruments"),
            || {
                assert!(
                    registry
                        .get_by_ticker(black_box("T7"), Some(&london))
                        .is_none()
                );
            },
        );
        let mut stated = OrderEvent::at(2);
        stated.set_ticker(Some(SmolStr::new("T7")), true);
        stated.set_miccode(Some(paris.clone()), true);
        let (filling, repeated) = counted_each(
            || stated.clone(),
            |mut held| {
                assert!(
                    registry.fill(&mut held),
                    "a fill by ticker at {size} instruments"
                );
                held
            },
        );
        assert_eq!(
            repeated,
            filling * 64,
            "a fill by ticker at {size} instruments"
        );
        fills.push(filling);
    }
    assert_eq!(
        fills[0], fills[1],
        "a fill by ticker at 64 and at 4,096 instruments"
    );
}

/// The ticker index grows as the rows do, by doubling: learning as many new
/// instruments as a registry holds, each listing a ticker of its own, costs
/// exactly one allocation more than learning the same instruments with no
/// ticker - the index's one table doubling - at 64 instruments as at 4,096.
/// A listing's slot is inline, so nothing else the index holds is per
/// instrument.
#[test]
fn an_isin_registry_ticker_index_doubles_once_as_its_listings_double() {
    crate::install::installed();
    use yggdryl::Mic;
    use yggdryl_market::graph::{Market, OrderEvent};
    let learning = |size: usize, ticker: bool| {
        let mut registry = isin_registry_of(size);
        let statements: Vec<OrderEvent> = (0..size)
            .map(|number| {
                let mut stated = OrderEvent::at(2);
                stated
                    .insert_securityid(
                        Identifier::new(IdKey::base(IdType::Isin), &numbered_isin("BE", number))
                            .unwrap(),
                    )
                    .unwrap();
                stated.set_miccode(Some(Mic::new("XBRU").unwrap()), true);
                if ticker {
                    stated.set_ticker(Some(SmolStr::new(format!("N{number}"))), true);
                }
                stated
            })
            .collect();
        let (allocations, learned) = counted(|| {
            statements
                .iter()
                .filter(|stated| registry.learn(black_box(*stated)))
                .count()
        });
        assert_eq!(learned, size, "{size} new instruments learned");
        assert_eq!(registry.len(), 2 * size);
        allocations
    };
    for size in [64, 4_096] {
        let (listed, unlisted) = (learning(size, true), learning(size, false));
        assert_eq!(
            listed,
            unlisted + 1,
            "{size} new tickers into an index of {size}: {listed} against {unlisted} with none"
        );
    }
}

/// Learning a new instrument builds its row in place - the ISIN, the CFI
/// code, the market, the ticker and up to four codes inline - so the
/// table's own slot is all it can cost: an ISIN sorting before every other
/// lands in a leaf ascending inserts left with room, and the ticker
/// index's slot is inline, its table of 64 or 4,096 distinct tickers having
/// room for one more, so learning it allocates nothing, whatever the
/// registry holds.
#[test]
fn an_isin_registry_learns_a_new_instrument_into_its_row_inline() {
    crate::install::installed();
    use yggdryl::{Cfi, Mic};
    use yggdryl_market::graph::{Market, OrderEvent};
    for size in [64, 4_096] {
        let mut registry = isin_registry_of(size);
        let mut stated = OrderEvent::at(2);
        for (kind, value) in [
            (IdType::Isin, numbered_isin("BE", 1).as_str()),
            (IdType::Common, "C-NEW"),
            (IdType::Belgian, "B-NEW"),
            (IdType::Valor, "1234567"),
        ] {
            stated
                .insert_securityid(Identifier::new(IdKey::base(kind), value).unwrap())
                .unwrap();
        }
        stated.set_cficode(Some(Cfi::new("ESVUFR").unwrap()), true);
        stated.set_miccode(Some(Mic::new("XBRU").unwrap()), true);
        stated.set_ticker(Some(smol_str::SmolStr::new("NEW")), true);
        let (learning, learned) = counted(|| registry.learn(black_box(&stated)));
        assert!(learned);
        assert_eq!(
            learning, 0,
            "learning a new instrument at {size} instruments"
        );
    }
}

/// `resolve` by a stated ISIN, by a lookup code and by the ticker on its
/// market borrows the row it answers and allocates nothing, whatever the
/// registry holds: the code index is keyed by the type and the code, read
/// through a borrowed key.
#[test]
fn an_isin_registry_resolves_by_isin_code_and_ticker_without_allocating() {
    crate::install::installed();
    use yggdryl::Mic;
    use yggdryl_market::Resolution;
    use yggdryl_market::graph::{Market, OrderEvent};
    for size in [64, 4_096] {
        let registry = isin_registry_of(size);
        let isin = numbered_isin("FR", 7);
        let by_isin = {
            let mut element = OrderEvent::at(2);
            element
                .insert_securityid(Identifier::new(IdKey::base(IdType::Isin), &isin).unwrap())
                .unwrap();
            element
        };
        let by_code = {
            let mut element = OrderEvent::at(2);
            element
                .insert_securityid(Identifier::new(IdKey::base(IdType::Common), "C-7").unwrap())
                .unwrap();
            element
        };
        let by_ticker = {
            let mut element = OrderEvent::at(2);
            element.set_ticker(Some(SmolStr::new("T7")), true);
            element.set_miccode(Some(Mic::new("XPAR").unwrap()), true);
            element
        };
        for (how, element) in [
            ("ISIN", &by_isin),
            ("code", &by_code),
            ("ticker", &by_ticker),
        ] {
            assert!(matches!(
                registry.resolve(element),
                Resolution::Matched { entry, .. } if entry.isin().as_str() == isin
            ));
            free(&format!("resolving by {how} at {size} instruments"), || {
                assert!(matches!(
                    registry.resolve(black_box(element)),
                    Resolution::Matched { .. }
                ));
            });
        }
    }
}

/// The economic scan scores each instrument of the element's currency
/// once, borrowing its short name, so a match allocates nothing at 64
/// instruments as at 4,096; two equal bests allocate the one `Vec` of
/// their ISINs the answer holds, and nothing per instrument.
#[test]
fn an_isin_registry_economic_scan_allocates_nothing_per_instrument() {
    crate::install::installed();
    use yggdryl::{Ccy, Fisn, Isin, Mic};
    use yggdryl_market::graph::{Market, OrderEvent};
    use yggdryl_market::{IsinEntry, IsinRegistry, Resolution, Unmatched};
    for size in [64, 4_096] {
        let mut registry = IsinRegistry::new();
        for number in 0..size {
            registry
                .merge(
                    IsinEntry::new(Isin::new(numbered_isin("FR", number)).unwrap())
                        .with_miccode(Some(Mic::new("XPAR").unwrap()))
                        .with_fisn(Some(Fisn::new(format!("ISSUER {number}/SH")).unwrap())),
                )
                .unwrap();
        }
        let named = |name: &str| {
            let mut element = OrderEvent::at(2);
            element
                .insert_securityid(Identifier::new(IdKey::base(IdType::Fisn), name).unwrap())
                .unwrap();
            element.set_currency(Ccy::new("EUR").unwrap(), true);
            element
        };
        let one = named("ISSUER 7/SH");
        free(&format!("an economic match at {size} instruments"), || {
            assert!(matches!(
                registry.resolve(black_box(&one)),
                Resolution::Matched { .. }
            ));
        });
        registry
            .merge(
                IsinEntry::new(Isin::new(numbered_isin("BE", 1)).unwrap())
                    .with_miccode(Some(Mic::new("XPAR").unwrap()))
                    .with_fisn(Some(Fisn::new("ISSUER 7/SH").unwrap())),
            )
            .unwrap();
        let (allocations, ambiguous) = counted(|| {
            matches!(
                registry.resolve(black_box(&one)),
                Resolution::Unmatched(Unmatched::Ambiguous { .. })
            )
        });
        assert!(ambiguous);
        assert_eq!(
            allocations, 1,
            "two equal bests at {size} instruments: the one Vec"
        );
    }
}

/// A snapshot stream shares the table rather than copying it: opening one
/// costs the same eleven allocations at 64 instruments as at 4,096 - the
/// reader, its schema and its field, and the two root declarations the
/// schema carries, `PARTITION:by` and `SORT:by` - its two keys, `isin` and
/// `miccode`, one text as its one key was - six more than the five before
/// the row declared them - and draining it lays each row out once, one
/// allocation a row - the row's run, its forty-six cells written where the
/// run keeps them in column order, each code typed as its column holds it
/// so the canonicalization answers the run untouched, and the short texts
/// and codes held inline - plus one doubling of the batch's row vector each
/// time the rows double. It was eight a row while the snapshot built the
/// named row - a B-tree of the cells behind one `Arc` that canonicalized
/// into the run per row - and nine from the forty-fifth column, `origccy`,
/// on; the snapshot now yields the ordered row, which is what a row
/// is, so the column count moves no allocation. The cursor that walks one
/// instrument's listings holds its ISIN inline, so it allocates nothing a
/// row.
#[test]
fn an_isin_registry_snapshot_stream_is_constant_to_open_and_reads_by_row() {
    crate::install::installed();
    // The row's Arrow projection is built once per process, on first use.
    drop(
        yggdryl_market::IsinRegistry::new()
            .into_arrow_reader()
            .unwrap(),
    );
    for size in [64, 4_096] {
        let registry = isin_registry_of(size);
        let (opening, reader) = counted(|| registry.into_arrow_reader().unwrap());
        drop(reader);
        assert_eq!(opening, 11, "opening a snapshot of {size} instruments");
    }
    let drain = |size: usize| {
        let reader = isin_registry_of(size).into_arrow_reader().unwrap();
        let (draining, rows) =
            counted(move || reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>());
        assert_eq!(rows, size);
        draining
    };
    for size in [64, 256] {
        assert_eq!(
            drain(2 * size) - drain(size),
            size + 1,
            "{size} more rows cost other than one a row"
        );
    }
}

/// Reloading rows the registry already holds - a golden file read again -
/// costs each batch the same whatever its rows: one cast plan for the
/// stream, the landing per batch - one narrowing per column of the
/// forty-six, one more than the forty-five before `firstunix` was added,
/// two more than the forty-four before `origccy` was,
/// two more than the forty-three before `lastunix` was, three more than the
/// forty-two before `fisn` was, four more than the forty-one before
/// `eusipacode` was and five more than the forty before `underlyingisin`
/// was, seven more than the thirty-seven before `countrycode`, `forexcode`
/// and `currency` were - and a code cell adopted as the landing proved it,
/// so a row that moves nothing allocates nothing. The 43rd column, `fisn`,
/// landed one more buffer per batch, the 44th, `lastunix`, one more again,
/// the 45th, `origccy`, one more again, and the 46th, `firstunix`, one
/// more again: one column, one allocation, at both corpus sizes - a row
/// that states no `lastunix` or `firstunix` folds no instant, and one that
/// states no origin currency folds none.
#[test]
fn an_isin_registry_reloads_known_rows_at_a_cost_per_batch() {
    crate::install::installed();
    let mut each_at = Vec::new();
    for size in [64, 512] {
        let mut registry = isin_registry_of(size);
        let batch = registry
            .into_arrow_reader()
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let mut load = |batches: usize| {
            let reader = yggdryl::arrow::batch_reader(batch.schema(), vec![batch.clone(); batches]);
            let (allocations, read) =
                counted(|| registry.extend_from_arrow_reader(reader).unwrap());
            assert_eq!(read, size * batches);
            allocations
        };
        load(1);
        let (one, two, four) = (load(1), load(2), load(4));
        let each = two - one;
        assert_eq!(
            four - one,
            3 * each,
            "{size} rows: a batch after the first cost {each}, but four cost {four} and one {one}"
        );
        each_at.push(each);
    }
    assert_eq!(
        each_at,
        [56, 56],
        "a batch of 64 and of 512 known rows: a cost per row"
    );
}

/// The size of [`yggdryl_market::Identifiers`]: one vector, its pointer, length and
/// capacity.
const IDENTIFIERS_SIZE: usize = 24;

#[test]
fn security_identifier_construction_is_inline_for_every_checked_code() {
    crate::install::installed();
    for (kind, code) in [
        (IdType::Isin, "US0378331005"),
        (IdType::Cusip, "037833100"),
        (IdType::Sedol, "0263494"),
        (IdType::Figi, "BBG000B9XRY4"),
        (IdType::Wkn, "716460"),
        (IdType::Valor, "3886335"),
        (IdType::Bloomberg, "AAPL US Equity"),
    ] {
        free(&format!("constructing {kind}:{code}"), || {
            let id = Identifier::new(
                IdKey::new(black_box(IdSource::Base), black_box(kind.clone())),
                black_box(code),
            )
            .unwrap();
            assert_eq!(id.kind(), &kind);
            black_box(id.value());
        });
    }
    let widest = "B".repeat(32);
    costs("constructing a 32-byte Bloomberg identifier", 1, || {
        black_box(
            Identifier::new(IdKey::base(IdType::Bloomberg), black_box(widest.as_str())).unwrap(),
        );
    });
    assert!(
        Identifier::new(IdKey::base(IdType::Bloomberg), &"B".repeat(33)).is_err(),
        "33 bytes are refused"
    );
    let heap = Identifier::new(IdKey::base(IdType::Bloomberg), &widest).unwrap();
    free("cloning a heap Bloomberg identifier", || {
        black_box(heap.clone());
    });

    let ids: yggdryl_market::Identifiers = [
        Identifier::new(IdKey::base(IdType::Isin), "US0378331005").unwrap(),
        heap.clone(),
    ]
    .into_iter()
    .collect();
    free("an Identifiers read through every folded spelling", || {
        let read = |spelling: &str| spelling.parse::<IdType>().unwrap();
        assert_eq!(ids.get(&read(black_box("isin"))), Some("US0378331005"));
        assert_eq!(
            ids.get(&read(black_box("Bloomberg"))),
            Some(widest.as_str())
        );
        assert_eq!(ids.get(&read(black_box("sedol"))), None);
    });
}
