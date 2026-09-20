# Book iterator

`market::BookIterator` reads one book per symbol per instant out of a stream of statements: the live orders and quotes of each instrument kept as its ladders, the prints against them kept as its last trade, volume and average price, and the book of every symbol an instant touched read once the stream moves past that instant.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/iterator.rs` the iterator, `rust/src/market/symbol.rs` the [`Symbol`](#symbol) a book is keyed by, `rust/src/market/statement.rs` the [`Statement`](#statement) a mixed stream holds. Nothing here reads a protocol: `FixCodec::statements` and `FixCodec::books` are the FIX reading, beside the [codec](../fix/index.md) |
| Instant | the statements of one instant are buffered and applied together, so a book is unique for its instant: the state after everything that instant said. The books of one instant are yielded in symbol order |
| Makers | an order's former lanes leave the ladder and, where it still [rests](order.md), its one lane rests at its limit for what it has left, under the identity its chain shares, so a later statement of the same order replaces the earlier. A live order stating nothing of the ladder - no price, no quantity, nothing filled or left, as a cancel request states nothing - leaves its lanes as they stand; a dead one - filled, cancelled, rejected, expired - leaves the ladder. A [quote](quote.md) rests both lanes it quotes under its identity, a restatement replacing both and a lane withdrawn to zero leaving |
| Prints | an [execution](execution.md) or a [trade](trade.md) states the last price and size, joins the volume and the turnover, and never rests or touches a level. A print the venue busted - `40TRDCXL` - moves nothing but the count |
| Expiry | an order past its expiration leaves at the first instant closed at or after it, with no statement of its own, and its symbol's book is read at that instant |
| Keying | a statement keys the symbol of the instrument it names, and every code it named is registered as an alias of that symbol, so an instrument named by its ticker first and its ISIN later stays one book; a print keys under the maker it fills where one rests; a maker keyed under a new symbol leaves the ladder it rested on before it rests on the new one; a statement naming no instrument keys `Symbol::GLOBAL`. `with_symbol(symbol)` keys every statement under one instead, which under the global symbol is the global book of the whole stream |
| Grid | `with_snapshot_ns(step)` reads one book per symbol per step it was touched in - the step's closing state, dated at the last instant that moved the symbol and stamped with the step. Zero or less takes the grid away, which is one book per instant |
| Held state | one maker per live order or quote and one level per distinct resting price, per symbol; a maker leaves on a statement of it that does not rest, on its expiry, or with the stream, exactly as the walk's live set is bounded; the statements of one open instant; and the books of one closed instant until they are yielded |
| Refusals | a source error is yielded where it is met and neither closes the instant nor moves a ladder. A statement before the open instant, under the caller's word that the stream is sorted, is refused naming `currunix` and moves nothing, because a ladder cannot be rewound - the one layer that refuses it, where a [walk](../fix/lifecycle.md) lets it through unchained |
| Bindings | Rust `BookIterator`, `Symbol`, `Statement`; Python `yggdryl.market.BookIterator`, `Symbol`, `FixCodec.statements`; JavaScript `market.BookIterator`, `market.Symbol`, `statements` |

## Use

The statements a capture makes, read into books; then the same statements under one symbol, which is the global book.

=== "Rust"

    ```rust
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Book, BookData, BookIterator, Statement, Symbol};
    use yggdryl::{Decimal18, FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|",
        b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.200|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:31.500|10=0|",
    ];
    // Every statement the capture makes, in instant order: the orders and
    // the quotes walked, and each fill in its order's chain.
    let statements: Vec<Statement> = reader
        .statements(reader.parse_lines(lines))
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(statements.len(), 5, "four orders and the fill the report states");
    assert_eq!(statements.iter().filter(|held| held.is_maker()).count(), 4);
    assert_eq!(statements.iter().filter(|held| held.is_print()).count(), 1);
    let fill = statements.iter().find(|held| held.is_print()).expect("the fill");
    let order = &statements[3];
    assert_eq!(fill.get_crossuuid(), order.get_crossuuid(), "a fill is in its order's chain");

    // One book per symbol per instant touched.
    let depth = NonZeroU32::new(5).expect("five");
    let books: Vec<BookData> = BookIterator::new(statements.clone().into_iter().map(Ok), depth, true)
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(books.len(), 4);
    assert_eq!(
        books.iter().map(Element::get_crosscode).collect::<Vec<_>>(),
        ["AAPL", "AAPL", "AAPL", "MSFT"]
    );
    assert_eq!(books[1].get_px(), "10.6".parse()?, "the mid once both sides rest");
    assert_eq!(books[2].get_bids(), [], "the fill retired the only bid");
    assert_eq!(books[2].get_lastqty(), Some(Decimal18::from_int(100)));
    assert_eq!(books[3].get_seqnum(), 0, "another symbol, another chain");

    // One symbol for the whole stream: the global book.
    let global: Vec<BookData> = BookIterator::new(statements.into_iter().map(Ok), depth, true)
        .with_symbol(Symbol::GLOBAL)
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(global.len(), 4);
    assert!(global.iter().all(|book| book.get_crosscode() == "GLOBAL"));
    assert_eq!(global[3].bid_count(), 1, "MSFT rests on the one ladder");
    assert_eq!(global[3].get_symbolticker(), None, "two tickers name no one ticker");

    // The symbol a statement keys, strongest code first.
    assert_eq!(Symbol::of(&books[0]).as_str(), "AAPL");
    assert_eq!(Symbol::new("  ").as_str(), "GLOBAL", "blank text names no instrument");
    assert!(Symbol::new("AAPL") < Symbol::new("MSFT"), "ordered by text");
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry
    from yggdryl.market import BookIterator, Execution, Order, Symbol

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|",
        b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.200|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:31.500|10=0|",
    ]
    parsed = list(reader.parse_lines(lines))
    # Every statement the capture makes, in instant order: the orders and
    # the quotes walked, and each fill in its order's chain.
    statements = list(reader.statements(parsed))
    assert len(statements) == 5  # four orders and the fill the report states
    assert [type(held).__name__ for held in statements] == ["Order"] * 3 + ["Execution", "Order"]
    fill = statements[3]
    assert isinstance(fill, Execution)
    assert fill.crossuuid == statements[2].crossuuid  # a fill is in its order's chain

    # One book per symbol per instant touched.
    books = list(BookIterator(reader.statements(parsed), 5))
    assert len(books) == 4
    assert [held.crosscode for held in books] == ["AAPL", "AAPL", "AAPL", "MSFT"]
    assert books[1].px.as_py() == Decimal("10.6")  # the mid once both sides rest
    assert books[2].bids == []  # the fill retired the only bid
    assert books[2].lastqty.as_py() == Decimal("100")
    assert books[3].seqnum == 0  # another symbol, another chain

    # One symbol for the whole stream: the global book.
    reader_global = BookIterator(reader.statements(parsed), 5, symbol=Symbol.GLOBAL)
    assert reader_global.depth == 5 and reader_global.snapshot_ns == 0
    globals_ = list(reader_global)
    assert [held.crosscode for held in globals_] == ["GLOBAL"] * 4
    assert globals_[3].bid_count == 1  # MSFT rests on the one ladder
    assert globals_[3].symbolticker is None  # two tickers name no one ticker

    # The symbol a statement keys, strongest code first.
    assert Symbol.of(books[0]) == Symbol("AAPL")
    assert Symbol("  ") == Symbol.GLOBAL  # blank text names no instrument
    assert Symbol("AAPL") < Symbol("MSFT")  # ordered by text
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
      '8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.200|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:31.100|10=0|',
      '8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:31.500|10=0|',
    ].map((line) => Buffer.from(line))
    const parsed = [...reader.parseLines(lines)]
    // Every statement the capture makes, in instant order: the orders and
    // the quotes walked, and each fill in its order's chain.
    const statements = [...reader.statements(parsed)]
    assert.equal(statements.length, 5) // four orders and the fill the report states
    const fill = statements[3]
    assert.equal(fill.crossuuid, statements[2].crossuuid) // a fill is in its order's chain

    // One book per symbol per instant touched.
    const books = [...new market.BookIterator(reader.statements(parsed), 5)]
    assert.equal(books.length, 4)
    assert.deepEqual(books.map((held) => held.crosscode), ['AAPL', 'AAPL', 'AAPL', 'MSFT'])
    assert.equal(books[1].px, '10.6') // the mid once both sides rest
    assert.deepEqual(books[2].bids, []) // the fill retired the only bid
    assert.equal(books[2].lastqty, '100')
    assert.equal(books[3].seqnum, 0) // another symbol, another chain

    // One symbol for the whole stream: the global book.
    const readerGlobal = new market.BookIterator(reader.statements(parsed), 5, 0n, market.Symbol.GLOBAL)
    assert.equal(readerGlobal.depth, 5)
    assert.equal(readerGlobal.snapshotNs, 0n)
    const globals = [...readerGlobal]
    assert.deepEqual(globals.map((held) => held.crosscode), ['GLOBAL', 'GLOBAL', 'GLOBAL', 'GLOBAL'])
    assert.equal(globals[3].bidCount, 1) // MSFT rests on the one ladder
    assert.equal(globals[3].symbolticker, null) // two tickers name no one ticker

    // The symbol a statement keys, strongest code first.
    assert.ok(market.Symbol.of(books[0]).equals(new market.Symbol('AAPL')))
    assert.ok(new market.Symbol('  ').equals(market.Symbol.GLOBAL)) // blank text names no instrument
    assert.equal(new market.Symbol('AAPL').text, 'AAPL')
    ```

## Symbol

`Symbol` is the one text an instrument's books are keyed by. `Symbol::of(element)` reads it off the codes a statement names, the strongest first: the ISIN, because it names the issue across every venue that trades it; else the ticker, which is what a venue spells; else the CUSIP, the SEDOL and the Bloomberg identifier, which are a nation's or a vendor's. A statement naming none keys `Symbol::GLOBAL`, which is also the one symbol a global book is read under. The text orders bytewise, which is the order the books of one instant are yielded in, and a book's own key is its `crosscode` rather than `Symbol::of` over it.

`GLOBAL` is spelled as the word because no standard reserves a code for it, and a venue ticker spelled `GLOBAL` keys the global book exactly as a currency spelled `XXX` is none.

## Statement

`Statement` is one value over the four products that state something to a ladder - an order, a quote, an execution or a trade - and it is a market event exactly as each arm is: every accessor and mutator reaches the arm's own event, its order is its instant, and the four readings a walk takes by value are the arm's own for two statements of one arm and refuse across arms, because an order is never another statement of a quote. It is Rust's alone: the bindings hand the reader the product classes themselves, and the reader converts each by its class.

`FixCodec::statements(messages)` is the one stream of every statement a capture makes. The lifecycle runs first, then two walks over the chained messages - the orders' and the quotes', which cannot share one, since an order and a quote spelling one identifier would take each other's place in it - merged by instant, the orders of one instant first; and each fill an execution report states is yielded right after the order statement the same report makes, in that order's chain, because the report restates the order and the walk has resolved its chain by then. Trades are not among them: a fill is the print, and the trade of the same report would print it again.

## The KPIs a book answers

Every reading is the ladders' and is computed on the call; `b` and `a` are the top prices, `Qb` and `Qa` the top sizes.

| Reading | Formula | Nothing where |
| --- | --- | --- |
| `mid` | `(b + a) / 2` | a top is missing |
| `spread` | `a - b` | a top is missing |
| `spread_bps` | `spread * 10 000 / mid` | there is no mid, or it is zero |
| `microprice` | `(b * Qa + a * Qb) / (Qb + Qa)` | a top is missing |
| `imbalance` | `(Qb - Qa) / (Qb + Qa)` at the tops | both tops are missing |
| `imbalance_to_depth(n)` | the same over the sizes summed across the first `n` levels of each ladder | both sums are zero |
| `bid_size`, `ask_size` | the size resting on that ladder, summed | never; an empty ladder rests nothing |
| `bid_count`, `ask_count` | the orders and quote lanes resting on that ladder | never |

`microprice` is the closed-form size-weighted mid, pulled toward the side with less resting on it. The facts that need history are not readings: the volume is `cumqty`, its average price `avgpx`, the last print `lastpx` and `lastqty`, and the previous book's mid and resting size `prevpx` and `prevqty`.

## Edges

- A stream carrying both the execution and the trade of one fill counts it twice, so a caller feeds one of the two; the FIX door feeds executions, and a capture that reports matches and no fills is fed trades instead.
- An order's resting size is the order's own statement - what it says is left, else what it ordered less what filled - and a print never reduces it: the report that carries a fill restates the order in the same message, and reducing twice is the defect every reconstructing builder guards against.
- Within an instant, the FIX door yields the prints of that instant after the order statements they belong to; the book of an instant does not depend on the order within it, because a print rests nothing and a maker prints nothing. Two prints at one instant leave the last one applied as `lastpx`.
- An expiry is applied at the first instant closed at or after it, so the book that shows the order gone is dated at that instant rather than at `expirunix` - a grid step already emitted cannot be rewritten.
- A book of a symbol whose makers disagree about an instrument fact states none of it: a global book over two currencies is priced in `XXX` and names no one MIC.

## Commands

```bash
cargo test -p yggdryl --test market iterator::
cargo test -p yggdryl --test market symbol::
cargo test -p yggdryl --test market statement::
cargo test -p yggdryl --doc market::iterator
cargo bench -p yggdryl --bench market -- market/iterator
```
