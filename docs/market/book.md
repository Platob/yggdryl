# Book

`market::Book` is the ladder for one instrument at one instant: levels of price, size and order count per side, to a declared depth, with what printed against it and every reading the two ladders imply.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/book.rs`: the `Book` trait, the `BookData` holder, `Level`, the row and the `Product` answers; the books are read by the [book iterator](iterator.md), and the FIX reading is `FixCodec::books` beside the [codec](../fix/index.md) |
| Chain | the symbol's: `crosscode` is the [`Symbol`](iterator.md#symbol) the book was read under - the instrument's ISIN, else its ticker, else another code the market named it by, else `GLOBAL` - and `crossuuid` the identity it derives, so every book of one symbol stands in one chain. The chain is flat: a book follows the one before it and descends from it alone, never from the whole line, because a stream reading one book per instant would otherwise carry a lineage as long as itself in every row; `prevpx` and `prevqty` are the previous book's mid and resting size |
| Ladders | `get_bids()` and `get_asks()`, each a `Level` per price - the size resting there, what the orders and quote lanes at that price have left, summed, and how many rest there - best first, to `get_depth()` levels; `set_bids` and `set_asks` sort a ladder best first and refuse two levels at one price or more levels than the depth. The top of each ladder is the lane the traits answer, so a book's `bidpx` is its best bid |
| Prints | the facts that need history, stated by whoever read the book: `lastpx` and `lastqty` the last print, `cumqty` the volume since the chain began and `avgpx` its average price, `get_updates()` how many statements have been applied - every order, quote and print among them |
| Reads | the ladders', computed on every call and never stored: `best_bid()`, `best_ask()`, `level(side, index)`; `mid()` and `spread()` across the tops; `spread_bps()` the spread as `spread * 10 000 / mid`; `microprice()` the size-weighted mid `(b * Qa + a * Qb) / (Qb + Qa)`, pulled toward the side with less resting on it; `imbalance()` `(Qb - Qa) / (Qb + Qa)` at the tops and `imbalance_to_depth(levels)` over the first `levels` of each; `bid_size()`, `ask_size()`, `bid_count()`, `ask_count()`; `is_two_sided()`, `is_locked()` and `is_crossed()` |
| Empty and one-sided | a missing top is nothing on every reading that divides by it; nothing is ever padded with a sentinel price; a one-sided book has no mid, no spread and an imbalance of one; an empty one has a price and a quantity of nothing. A locked or crossed book is stated, never refused |
| Doors | `FixCodec::books(messages, depth, snapshot_ns)`: one book per symbol per instant with a step of zero, one per grid step at its closing state with a positive one; `books_arrow_reader(source, depth, snapshot_ns)` under `BookData::field(depth)` |
| Writes back | refused: `FixMsg::from_book` answers the one sentence - a book is a reading of every order and quote resting at an instant, and no one message states a ladder, so the crate does not guess |
| Bindings | Rust `Book`/`BookData`; Python `yggdryl.market.Book`, `FixCodec.books`, `books_arrow_reader`, `FixMsg.from_book`; JavaScript `market.Book`, `books`, `booksArrowReader`, `FixMsg.fromBook` |

## Use

Three bids, an offer, a two-sided quote and a fill that retires one bid, then an order of another instrument: one book per symbol per instant.

=== "Rust"

    ```rust
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Book, BookData, Product};
    use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry, Side};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 7] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|",
        b"8=FIX.4.4|35=D|11=A2|55=AAPL|54=1|38=50|44=10.5|15=USD|52=20260102-10:15:30.200|10=0|",
        b"8=FIX.4.4|35=D|11=A3|55=AAPL|54=1|38=70|44=10.4|15=USD|52=20260102-10:15:30.300|10=0|",
        b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.400|10=0|",
        b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=30|133=10.6|135=40|15=USD|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:32.100|10=0|",
        b"8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:32.500|10=0|",
    ];
    // A step of zero reads one book per symbol per instant touched.
    let books: Vec<BookData> = reader
        .books(reader.parse_lines(lines), 2, 0)?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(books.len(), 7, "six instants of AAPL, one of MSFT");
    let (first, quoted, filled, other) = (&books[0], &books[4], &books[5], &books[6]);
    // The first instant: one bid, no offer, so no mid.
    assert_eq!(first.get_crosscode(), "AAPL");
    assert_eq!(first.get_bids().len(), 1);
    assert_eq!(first.get_bidpx(), Some("10.5".parse()?), "the top of the ladder is the lane");
    assert_eq!(first.get_px(), Decimal18::ZERO, "no mid on a one-sided book");
    assert_eq!((first.mid(), first.spread()), (None, None));
    assert_eq!(first.imbalance(), Some(Decimal18::from_int(1)), "a missing top counts as no size");
    assert!(first.get_identifiers().is_empty(), "a book goes by its symbol, which is its chain");
    // Two bids at one price are one level, summed and counted, and the
    // quote's lanes cut the ladder to the declared depth.
    assert_eq!(quoted.best_bid().map(|level| (level.qty, level.count)), Some((Decimal18::from_int(150), 2)));
    assert_eq!(quoted.get_bids().len(), 2);
    assert_eq!(quoted.level(&Side::read("Sell")?, 0), quoted.best_ask());
    assert_eq!(quoted.spread(), Some("0.1".parse()?));
    assert_eq!(quoted.spread_bps(), Some("94.786729857819905213".parse()?));
    assert_eq!(quoted.microprice(), Some("10.578947368421052631".parse()?));
    assert!(quoted.is_two_sided() && !quoted.is_locked() && !quoted.is_crossed());
    assert_eq!(quoted.bid_count(), 3);
    // The fill retires the first bid and prints against the book.
    assert_eq!(filled.get_lastpx(), Some("10.5".parse()?));
    assert_eq!(filled.get_cumqty(), Some(Decimal18::from_int(100)));
    assert_eq!(filled.get_avgpx(), Some("10.5".parse()?));
    assert_eq!(filled.get_updates(), 7, "five makers, the order's fill and the print");
    // The chain is the symbol's, and flat.
    assert_eq!(filled.get_prevuuid(), Some(quoted.get_curruuid()));
    assert_eq!(filled.get_parentuuids(), [quoted.get_curruuid()], "the predecessor alone");
    assert_eq!(other.get_crosscode(), "MSFT", "another symbol, another chain");
    assert_eq!(other.get_seqnum(), 0);
    // A grid reads one book per symbol per step, at the step's closing state.
    let stepped: Vec<BookData> = reader
        .books(reader.parse_lines(lines), 2, 1_000_000_000)?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(stepped.len(), 4);
    assert_eq!(stepped[0].get_snapunix(), Some(1_767_348_930_000_000_000));
    assert_eq!(stepped[0].get_currunix(), 1_767_348_930_400_000_000, "the last instant that moved it");
    // The row is fixed-width: two slots a side, the empty ones null.
    let depth = NonZeroU32::new(2).expect("two");
    let field = BookData::field(depth)?;
    assert_eq!(field.fields().last().map(yggdryl::Field::name), Some("updates"));
    let again = BookData::from_row(&field, &quoted.into_row()?)?;
    assert_eq!(again.get_bids(), quoted.get_bids());
    assert_eq!(again.get_px(), quoted.get_px(), "the mid is re-derived from the ladders");
    // Depth is a declaration, and no message states a book.
    assert!(reader.books(reader.parse_lines(lines), 0, 0).is_err());
    assert!(FixMsg::from_book(&reader, first).unwrap_err().to_string().contains("does not guess"));
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry
    from yggdryl.market import Book

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|",
        b"8=FIX.4.4|35=D|11=A2|55=AAPL|54=1|38=50|44=10.5|15=USD|52=20260102-10:15:30.200|10=0|",
        b"8=FIX.4.4|35=D|11=A3|55=AAPL|54=1|38=70|44=10.4|15=USD|52=20260102-10:15:30.300|10=0|",
        b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.400|10=0|",
        b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=30|133=10.6|135=40|15=USD|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:32.100|10=0|",
        b"8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:32.500|10=0|",
    ]
    # A step of zero, the default, reads one book per symbol per instant.
    books = list(reader.books(reader.parse_lines(lines), 2))
    assert len(books) == 7  # six instants of AAPL, one of MSFT
    first, quoted, filled, other = books[0], books[4], books[5], books[6]
    # The first instant: one bid, no offer, so no mid.
    assert first.crosscode == "AAPL"
    levels = lambda ladder: [(px.as_py(), qty.as_py(), count) for px, qty, count in ladder]
    assert levels(first.bids) == [(Decimal("10.5"), Decimal("100"), 1)]
    assert first.bidpx.as_py() == Decimal("10.5")  # the top of the ladder is the lane
    assert first.px.as_py() == Decimal("0")  # no mid on a one-sided book
    assert first.mid is None and first.spread is None
    assert first.imbalance.as_py() == Decimal("1")  # a missing top counts as no size
    assert first.identifiers == {}  # a book goes by its symbol, which is its chain
    # Two bids at one price are one level, summed and counted, and the
    # quote's lanes cut the ladder to the declared depth.
    assert levels([quoted.best_bid]) == [(Decimal("10.5"), Decimal("150"), 2)]
    assert len(quoted.bids) == 2
    assert quoted.level("Sell", 0) == quoted.best_ask
    assert quoted.spread.as_py() == Decimal("0.1")
    assert quoted.spread_bps.as_py() == Decimal("94.786729857819905213")
    assert quoted.microprice.as_py() == Decimal("10.578947368421052631")
    assert quoted.is_two_sided and not quoted.is_locked and not quoted.is_crossed
    assert quoted.bid_count == 3
    # The fill retires the first bid and prints against the book.
    assert filled.lastpx.as_py() == Decimal("10.5")
    assert filled.cumqty.as_py() == Decimal("100")
    assert filled.avgpx.as_py() == Decimal("10.5")
    assert filled.updates == 7  # five makers, the order's fill and the print
    # The chain is the symbol's, and flat.
    assert filled.prevuuid == quoted.curruuid
    assert filled.parentuuids == [quoted.curruuid]  # the predecessor alone
    assert other.crosscode == "MSFT"  # another symbol, another chain
    assert other.seqnum == 0
    # A grid reads one book per symbol per step, at the step's closing state.
    stepped = list(reader.books(reader.parse_lines(lines), 2, 1_000_000_000))
    assert len(stepped) == 4
    assert stepped[0].snapunix == 1_767_348_930_000_000_000
    assert stepped[0].currunix == 1_767_348_930_400_000_000  # the last instant that moved it
    # The row is fixed-width: two slots a side, the empty ones null.
    field = Book.field(2)
    assert [child.name for child in field][-1] == "updates"
    again = Book.from_row(field, quoted.into_row())
    assert levels(again.bids) == levels(quoted.bids)
    assert again.px == quoted.px  # the mid is re-derived from the ladders
    # Depth is a declaration, and no message states a book.
    with pytest.raises(ValueError, match="depth"):
        reader.books(reader.parse_lines(lines), 0)
    with pytest.raises(ValueError, match="does not guess"):
        FixMsg.from_book(reader, first)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, market } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|',
      '8=FIX.4.4|35=D|11=A2|55=AAPL|54=1|38=50|44=10.5|15=USD|52=20260102-10:15:30.200|10=0|',
      '8=FIX.4.4|35=D|11=A3|55=AAPL|54=1|38=70|44=10.4|15=USD|52=20260102-10:15:30.300|10=0|',
      '8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.400|10=0|',
      '8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=30|133=10.6|135=40|15=USD|52=20260102-10:15:31.100|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:32.100|10=0|',
      '8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:32.500|10=0|',
    ].map((line) => Buffer.from(line))
    // A step of zero, the default, reads one book per symbol per instant.
    const books = [...reader.books(reader.parseLines(lines), 2)]
    assert.equal(books.length, 7) // six instants of AAPL, one of MSFT
    const [first, quoted, filled, other] = [books[0], books[4], books[5], books[6]]
    // The first instant: one bid, no offer, so no mid.
    assert.equal(first.crosscode, 'AAPL')
    assert.deepEqual(first.bids, [{ px: '10.5', qty: '100', count: 1 }])
    assert.equal(first.bidpx, '10.5') // the top of the ladder is the lane
    assert.equal(first.px, '0') // no mid on a one-sided book
    assert.deepEqual([first.mid, first.spread], [null, null])
    assert.equal(first.imbalance, '1') // a missing top counts as no size
    assert.deepEqual(first.identifiers, {}) // a book goes by its symbol
    // Two bids at one price are one level, summed and counted, and the
    // quote's lanes cut the ladder to the declared depth.
    assert.deepEqual(quoted.bestBid, { px: '10.5', qty: '150', count: 2 })
    assert.equal(quoted.bids.length, 2)
    assert.deepEqual(quoted.level('Sell', 0), quoted.bestAsk)
    assert.equal(quoted.spread, '0.1')
    assert.equal(quoted.spreadBps, '94.786729857819905213')
    assert.equal(quoted.microprice, '10.578947368421052631')
    assert.ok(quoted.isTwoSided && !quoted.isLocked && !quoted.isCrossed)
    assert.equal(quoted.bidCount, 3)
    // The fill retires the first bid and prints against the book.
    assert.equal(filled.lastpx, '10.5')
    assert.equal(filled.cumqty, '100')
    assert.equal(filled.avgpx, '10.5')
    assert.equal(filled.updates, 7) // five makers, the order's fill and the print
    // The chain is the symbol's, and flat.
    assert.equal(filled.prevuuid, quoted.curruuid)
    assert.deepEqual(filled.parentuuids, [quoted.curruuid]) // the predecessor alone
    assert.equal(other.crosscode, 'MSFT') // another symbol, another chain
    assert.equal(other.seqnum, 0)
    // A grid reads one book per symbol per step, at the step's closing state.
    const stepped = [...reader.books(reader.parseLines(lines), 2, 1_000_000_000n)]
    assert.equal(stepped.length, 4)
    assert.equal(stepped[0].snapunix, 1_767_348_930_000_000_000n)
    assert.equal(stepped[0].currunix, 1_767_348_930_400_000_000n)
    // The row is fixed-width: two slots a side, the empty ones null.
    const field = market.Book.field(2)
    assert.equal(field.fieldAt(32).name, 'updates') // the last column
    const again = market.Book.fromRow(field, quoted.intoRow())
    assert.deepEqual(again.bids, quoted.bids)
    assert.equal(again.px, quoted.px) // the mid is re-derived from the ladders
    // Depth is a declaration, and no message states a book.
    assert.throws(() => reader.books(reader.parseLines(lines), 0), /depth/)
    assert.throws(() => fix.FixMsg.fromBook(reader, first), /does not guess/)
    ```

## Row schema

`BookData::field(depth)` opens with the sixteen [event columns](../graph.md#columns), then the prints, the instrument, then the two ladders and the count.

| column | datatype | value |
| --- | --- | --- |
| `lastpx`, `lastqty` | `decimal128(38, 18)` | nullable; the last print applied to the chain |
| `avgpx` | `decimal128(38, 18)` | nullable; the volume's average price since the chain began |
| `cumqty` | `decimal128(38, 18)` | nullable; the volume since the chain began |
| `tradable` | `bool` | nullable; the venue's word about the instrument, folded from the makers |
| `currency` | `currency` | required; `XXX` where the makers disagree or state none |
| `unit` | `utf8` | nullable; the unit the sizes are counted in |
| `symbolticker` | `utf8` | nullable; the ticker, where every statement that named one agrees |
| `isincode`, `cusipcode`, `sedolcode`, `bloombergcode` | `isin`, `cusip`, `sedol`, `bloomberg` | nullable; likewise |
| `cficode` | `cfi` | nullable; the instrument's classification |
| `miccode` | `mic` | nullable; the market, where the makers agree on one |
| `bids`, `asks` | `fixed_size_list<level: struct<px, qty, count: uint64>?, depth>` | required; each ladder best first, one slot per level to the declared depth, the slots past it null |
| `updates` | `uint64` | required; how many statements the chain has applied |

Each ladder is a fixed-size list rather than a variable one, so a depth-ten book is a fixed-width row: every slot of every row sits at an offset the depth decides, and a scan reads the best bid of a million books without an offsets pass. The mid, the resting size, the lanes and every other reading are the ladders' and are re-derived from the row's, so no column restates another; `prevpx` and `prevqty` are the predecessor book's, reached by `prevuuid`. A book's table joins the [message table](../fix/capture.md#the-crates-own-columns) on `srcuuids` to `curruuid` and its own table on `prevuuid`.

## Edges

- `depth` is a declaration: a depth of zero is no ladder and is refused naming it, and a ladder deeper than the book was opened for is refused rather than grown.
- A locked book (`spread` of zero) and a crossed one (a negative spread) are stated rather than refused, and `is_locked` and `is_crossed` read them; `tradable` stays the venue's word about the instrument and is never derived from the ladders.
- `spread_bps` and `microprice` answer nothing where a top is missing, because both divide by something a one-sided book does not have; `imbalance` counts a missing top as no size and so answers one or minus one.
- A book's `srcuuids` are the statements applied since the book before it, and what rested any maker it retired - so the book that shows an order gone names the statement that had rested it.
- `updates` counts every statement the chain applied, a print that moved nothing among them, which is why it is stated rather than derived.

## Commands

```bash
cargo test -p yggdryl --test market book::
cargo test -p yggdryl --doc market::book
cargo bench -p yggdryl --bench market -- "market/doors/books|market/rows/book"
```
