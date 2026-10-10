# Book

A book is live depth over time: `BookEvent` one book at an instant - complete, holding the entries alive on both sides and each side read as its price levels, or a [delta book](#complete-books-and-delta-books) holding no sides: its `delta` - the orders and quotes it applied since the book before - its `events` - the executions and snapshot controls it recorded - and the top of book they settled on - `SnapshotEvent` the scope-replacing control, and `BookIterator` the fold of a sorted stream into books. Sorted books fold on into [candles](candle.md).

## Contract

| Type | Owns | Traits |
| --- | --- | --- |
| `BookEvent` | on a [complete](#complete-books-and-delta-books) book the orders and quotes alive on its bid and ask sides; on every book its `delta` - the orders and quotes applied since the book before - and its `events` - every other event its instant recorded - each in the order applied, and the top of book they settled on | `Element`, `Event`, `Market` |
| `SnapshotEvent` | an empty FIX `W`'s full-snapshot control: event + replaced scope, no entry | `Element`, `Event`, `Market` |
| `BookIterator` | the [fold](#book-fold) from a sorted stream to books | `Iterator<Item = Result<BookEvent>>` |
| `yggdryl_market::Limit` | one [price level](#limits) of a side, a root value type | - |

All in `yggdryl_market::graph::book`, with the `IdType` keys `ENTRY_ID` (`mdentryid`) and `ENTRY_REF_ID` (`mdentryrefid`); `Limit` in the market crate's root `limit.rs`. A book is keyed by its [book key](market.md#the-book-key) - the instrument's cross code the input states as its `instcode`, alone: a real ISIN, an FX pair's `IF:EUR/USD`, a derivative's `class:body` - stores it as its cross code `3:0:{key}` and states it as its own `instcode`; an input stating no `instcode` is pruned before the walk, as a kind a book does not record is. A book holds both sides - its side is `Side::Both` (`BOTH`), whatever it is set to, a side set on it moving no price or level - and neither a book nor a snapshot control is [sided](market.md#sides-and-cross-codes): its stored cross code states side `0`, and a snapshot control over an order takes the order's base code under its own kind. A book places no execution: a fill moves it through its order's or quote's own report, and an execution of its instant is recorded among its `events`, resting on no side; every input `booked` does not admit - a kind [`MarketDataKind::is_recorded`](../types/enum/marketdatakind.md) does not admit, a trade, a batch, and a recorded input stating no `instcode` - is pruned before it folds. Its row nests `alive`, `delta`, `events`, `bidlimits` and `asklimits`, and its `executions` cell is null ([Market data](market-data.md#arrow)).

## Complete books and delta books

| Key | Rule |
| --- | --- |
| `is_complete()` | whether the book holds its sides - every entry alive on it - rather than only its `delta` and `events`: a book a caller builds (`new`, `keyed`, `add_operations`), one a walk emits at a [snapshot tick](#book-fold), and one rebuilt by `with_previous` are complete. Python `book.is_complete`, JavaScript `book.isComplete` |
| A delta book | holds no sides and states its `delta()` and `events()` alone, beside the top-of-book facts they settled on - `bidpx`, `bidqty`, `bidccy`, `askpx`, `askqty`, `askccy`, `price`, `quantity` - and names the book it follows as its `prevuuid`, none for a code's first book, which follows the empty book; `alive()`, `alive_on`, `limits`, `depth` and `imbalance` answer nothing on it. An instant that recorded only an execution is a delta book whose `delta` is empty and whose `events` hold the execution |
| Read in both forms | `best_price`, `best_quantity`, `spread`, `bbo_midpoint`, `median_quantity`, `is_crossed` and `is_locked` read the book's own top-of-book facts, never a side, so a delta book answers them as its rebuilt book does |
| `with_previous(&previous)` | a delta book rebuilt over the complete book it follows: `previous`' sides taken, each `delta` entry replayed in the order applied, whole and under its own identity, its `events` kept and replaying nothing. A delta book naming no `prevuuid` - a code's first book - rebuilds over the empty book `BookEvent::keyed(unix, key)` every walk starts from, whatever `previous` holds. `None` where it cannot follow `previous`: another book code, a later instant than this book's, itself, another book than its `prevuuid` names, a `previous` holding no sides, a `delta` entry that does not replay, or a rebuild settling on another top of book than the one it states. A complete book is authoritative: it only takes the link - `prevuuid`, `prevunix`, `prevpx`, `prevqty` - and answers `None` where nothing moves |
| `BookEvent::keyed(unix, key)` | the one constructor: an empty, complete book keyed `key` - the instrument's cross code every input it takes states as its `instcode`, which the book states as its own - stating neither a ticker nor an ISIN until its first input states each; a key spelled as a stored book cross code (`3:0:IBM`) is read as one, its base the code; an empty `key` keys a book by nothing, whose cross code is empty, which takes nothing and which reads back from its row as written. The empty base a code's first book rebuilds over, `delta.with_previous(&BookEvent::keyed(unix, key))`. Python `graph.BookEvent(transunix, key)` and `graph.BookEvent.keyed(transunix, key)`, JavaScript `new graph.BookEvent(transunix, key)` and `graph.BookEvent.keyed(transunix, key)`, one door |
| `add_operations` on a delta book | refused at `$.alive` (`a delta book takes no operations: rebuild it with with_previous first`) |
| Identity | chain form: the book's market event - its state, the book it follows and every market fact, its top of book included - its instant, each side's digest only where it states a `snapunix`, then its `delta` count and each entry's operation word and `uuid`, then its `events` count and each event's kind and `uuid` - an execution's `uuid` digesting what it executed, its [`quantity`](market.md#one-quantity-per-kind). A delta book and the book rebuilt from it share one identity, and a book walks its sides only at a snapshot |
| Its row | a complete book states `alive` - an empty list included - and its `bidlimits` and `asklimits`; a delta book leaves those three null; every book states its `delta` and `events`. One distinction is lost in every table, because a table may store a null list as an empty one and the reader takes one rule: a complete book holding no live entry that states a `delta` or `events` entry and no `snapunix` - one built or rebuilt, never one a walk emits - reads back as a delta book, its identity and both lists kept. An event-only row - `delta` empty, `events` holding an execution, no `snapunix` - is a delta book, never a snapshot control ([the leaf a row names](market-data.md#the-leaf-a-row-names)) |

## Entries

| Key | Rule |
| --- | --- |
| `alive()` | every live order and quote once, as `&MarketData`: the bid side's, best price first, ties by `BookRef::position` then arrival, entries stating no price (market orders) last; then the ask side's the same way but those resting on the bid too - a two-sided quote is one entry, listed with the bids. Nothing on a delta book |
| `alive_on(side)` | the entries alive on the side `side` takes, borrowed from that side's store, an `ExactSizeIterator`: best price first and the unpriced last, a two-sided quote on both sides at its leg's price on each; nothing for a side that is neither a bid nor an ask - `UKNW`, `BOTH` - or on a delta book. Python `book.alive_on(side)`, JavaScript `book.aliveOn(side)` - a `Side` member, its code or any spelling it reads |
| `delta()` | the membership operations the book's instant applied since the book before - the orders and quotes placed, changed, ended, expired, withdrawn or range-deleted - in the order applied across both sides, each the very entry a side holds where it rests, an `ExactSizeIterator`: what a delta book states and what `with_previous` replays; a row stating anything but an order or a quote among it is refused at `$.delta[i]`. Python `book.delta` (`list[MarketData]`), JavaScript `book.delta()` |
| `events()` | every other event the book's instant recorded, in the order applied, an `ExactSizeIterator`: the executions - resting on no side and moving none, a fill having moved the book through its order's or quote's own report - and the snapshot controls whose replaced membership made the book complete, so a delta book states none. From FIX that is everything else a book records: the parse creates orders, quotes, executions and snapshot controls alone, and the fold [prunes](#book-fold) a trade. `with_previous` replays none of them; a row stating anything but an execution or a snapshot control among them is refused at `$.events[i]`. Python `book.events` (`list[MarketData]`), JavaScript `book.events()` |
| `ordlive()` | the orders resting on the book: every `alive()` entry that is an order, in its order, as `&OrderEvent`; nothing on a delta book - rebuild it with `with_previous` first. Python `book.ordlive` (`list[OrderEvent]`), JavaScript `book.ordlive()` |
| `orddelta()` | the orders among `delta()`, in the order applied, as `&OrderEvent`: placed, changed, ended, expired or withdrawn at the book's instant - an order placed then is also in `ordlive()`, the one entry both borrow, an order ended then in `orddelta()` alone. Python `book.orddelta`, JavaScript `book.orddelta()` |
| `quotes()` | the quotes among `delta()`, in the order applied, as `&QuoteEvent`; a quote resting since an earlier instant is `alive()`'s and not here. Python `book.quotes`, JavaScript `book.quotes()` |
| `executions()` | the executions among `events()`, in the order applied, as `&ExecutionEvent`: recorded at the book's instant, resting on no side. Python `book.executions`, JavaScript `book.executions()` |
| `controls()` | the snapshot controls among `events()`, in the order applied, as `&SnapshotEvent`: each `W` control whose replaced membership made the book complete, so only a complete book states one. Python `book.controls` (`list[SnapshotEvent]`), JavaScript `book.controls()` |
| By kind | `orddelta()` and `quotes()` partition `delta()`, `executions()` and `controls()` partition `events()` - `ordlive()` reads `alive()` and is outside both - and `alive()` holds orders and quotes alone; each reading borrows and allocates nothing, and is no storage of its own: the row keeps the two lists, `delta` and `events`, each in the order applied, and [`MarketData::delta_serie` and `MarketData::events_serie`](market-data.md#arrow) are the same split over a table of books |
| Legs | an entry rests on every side it states a leg for, as one entry: an order - [sided](market.md#sides-and-cross-codes) - its price, quantity and currency on the side its [`Side`](../types/enum/side.md) takes, the bid for `Side::is_bid` (`BUYS`, `BUYM`), the ask for `Side::is_ask` (`SELL`, `SELP`, `SSHT`, `SSEX`, `SELU`); a quote - which holds a bid and an ask and tags a side - each leg it states a price or a quantity of, in that leg's currency, else its own. A leg sized zero rests nowhere: a feed withdraws a level by sizing it zero |
| Every input a delta entry | every order and quote but a repeat is an entry of its book's `delta`, whether or not it rests anywhere: one resting on no side and continuing no live entry - an order of side `UKNW`, a quote stating no leg - warned of where it is live, one first seen ended, and one ending an entry the book no longer holds place nothing, are never refused, and still advance the book as its `delta` |
| Repeats | a statement repeating the live entry it continues - every fact the same but its identity, its digests and its place in its chain, under the same book control - records nothing in the `delta` and leaves the entry as it stood; a group of repeats changes nothing. A full snapshot restating an entry its scope held keeps that entry, identity and all |
| Live key | `(symbol, scope, cross identity)`, looked up on both sides: a same-symbol, same-scope `mdentryrefid` resolves first, else the input's own identity where it is live, else its `mdentryid`; a second, distinct destination is ambiguous and refused. An id names an entry of the input's entry type - the side an order takes or a quote tags, `UKNW` for a quote tagging none or `BOTH` - so a new offer never continues a bid going by its id: only a change, an overlay or a delete finding none of its type continues the entry of the other type, which it moves; an input stating no side names the one entry going by the id where just one does. The reference names one step, so a following entry never [takes the one its predecessor states](operation.md#following-and-merging) |
| New, change, overlay, delete | a new generation replaces and chains the live entry; a change or overlay (`is_partial`) - and a delete - inherits price and size only where it states none (zero stays zero); a change or overlay with no predecessor must state both, never fabricated |
| Orders among quotes | a change or overlay (`1`, `5`) keeps a matched `OrderEvent` when `orderid` is omitted and promotes an unidentified `QuoteEvent` when it is stated; an `orderid` other than the live entry's continues the entry only where a parent of `orderid` the input states - `parentorderid` or `origorderid`, [`Operation::parents_of`](operation.md#contract) of `orderid` ([Parentage](identifier.md#parentage)) - names the live order, as a replace under a new `OrderID` naming the one it replaced does, and is another order's otherwise; a contradiction - that unexplained `orderid`, at `$.operation.identifiers.orderid` (`expected the live order "O-1", got "O-2"`), or a continuation of the other kind no rule promotes, at `$.operation.kind` - is a located `InvalidRecord`, the book unchanged; a delete (`2`) keeps the matched kind and predecessor link. A [walked](market.md#sides-and-cross-codes) replace stands under its order's cross identity, and one under a new `OrderID` names the one before as its `parentorderid` ([`follow_parents`](identifier.md#parentage)), so either continues the entry where it stood |
| Anonymous new | a new (`0`) without `mdentryid` never inserts into an occupied position: it states `mdentryid` or arrives in a snapshot |
| Range deletes | delete-through and delete-from (`3`, `4`) take the positions of the range's symbol and scope on the side its entry type takes, each entry off every side it rests on; a missing or malformed position refuses atomically, and a range stating neither a bid nor an ask takes nothing off, warned of |
| Its book | an input stands in a book where its `instcode` is the book's [key](market.md#the-book-key); another is refused at `$.operation.instcode` (`expected book crosscode "US0378331005", got "CH0012214059"`), and so is an input stating none (`got none`). A book so takes two listings' tickers of one instrument, which stay apart inside it by their own partition |

## Limits

`Limit { price: Option<Decimal>, quantity: Decimal, uuids: Vec<Uuid>, tradable: bool }` is one price level of a side: its price (`None` folds the unpriced entries), the sum of its entries' quantities on that side - an order's quantity, a quote's leg - their `uuid`s in live order, and whether it can trade - read by equality and hash.

| Key | Rule |
| --- | --- |
| `limits(side)` | one `Limit` per level of the side `side` takes, held order - best first, unpriced last - `uuids` in position then arrival order; nothing for a side that is neither a bid nor an ask, or on a delta book; allocates only `uuids`; what a complete book's row states under `bidlimits` (`BUYS`) and `asklimits` (`SELL`) |
| `tradable` | whether any entry of the level does not state `tradable = false`: an entry stating nothing is a live order no venue halted, so it trades, and only a level every entry of which states `false` cannot; the entries' own facts stay as stated |
| `best_price(side)`, `best_quantity(side)` | the first tradable priced level's price, and the sum of what its entries state on the side (an unstated quantity counts 0); `None` for an empty side, one holding only unpriced entries, one no level of which can trade, or a side that is neither - a level that cannot trade is skipped, never answered. What the book states as `bidpx`/`bidqty` and `askpx`/`askqty`, and read from them, so a delta book answers them too |
| `depth(side, levels)` | the sum of the first `levels` limits' quantities, tradable or not, unpriced counted where reached: zero for an empty side or no level, `None` past [`decimal`](../types/numeric/decimal.md#decimal), for a side that is neither, or on a delta book |
| Readings | `is_locked()`: both best tradable prices equal; `is_crossed()`: the best tradable bid above the best tradable ask; `spread()`: best ask less best bid, negative if crossed, `None` if a side has no best; `imbalance(levels)`: `(bid - ask) / (bid + ask)` over `depth(levels)` - `1`/`-1` one-sided, `None` if zero total, both empty, no level, overflow, or on a delta book; none is a stored fact |
| A value | not a datatype: `Limit::dtype()` = `struct<price: decimal?, quantity: decimal, uuids: serie<uuid>, tradable: boolean>` (required `uuid` item); `Limit::field()` = the required `limit` item of a `bidlimits`/`asklimits` column; `into_scalar` = the named `Scalar::Struct` of the four cells |
| `Limit::from_scalar` | reads that struct, or what `dtype().scalar` canonicalizes it to, through `Limit::field()`'s value door, refused under `$.limit` as a column would refuse it (empty text, float, grouped digits, 19th fractional digit) - except that a name the struct lacks is null, so a missing quantity or `tradable` is refused, never a zero or a `false` |
| Bindings | `Limit` is Rust-only. A side is a `Side` member, its code or any spelling it reads. Python: `book.limits(side)` a list of struct `Scalar`s (`limit["price"]`; `.as_py()` a dict of `price`, `quantity`, `uuids`, `tradable`), `best_price(side)`, `best_quantity(side)`, `depth(side, levels)`, `imbalance(levels)` a `Scalar` or `None`, and the properties `spread`, `bbo_midpoint`, `median_quantity`, `is_locked`, `is_crossed`; JavaScript: `book.limits(side)` as `{ price, quantity, uuids, tradable }` with decimal text, `bestPrice(side)`, `bestQuantity(side)`, `depth(side, levels)`, `imbalance(levels)` as text or `null`, and the properties `spread`, `bboMidpoint`, `medianQuantity`, `isLocked`, `isCrossed` |

## Books

| Key | Rule |
| --- | --- |
| `BookEvent::keyed(unix, key)` | an empty, complete book at that nanosecond, state `NEW`, keyed `key` ([above](#complete-books-and-delta-books)) - stored `3:0:{key}`, its own `instcode` the key - taking the inputs stating that code as their `instcode` |
| Its instrument | a book takes its ticker and its ISIN - as its `isin` security identifier - from the first input stating each; no input moves them after |
| `add_operations` | any `IntoIterator` of `MarketData`/`Result<MarketData>`, no-op if empty; every input `MarketDataKind::is_recorded` does not admit - a `TradeEvent` - is pruned first, so a group of nothing else changes nothing: the instant does not advance and the `delta` and `events` stand; a recorded input stating no `instcode`, or another book's, is refused at `$.operation.instcode`. `OrderEvent`/`QuoteEvent` fold into the sides and the `delta`, a `SnapshotEvent` replaces membership and is recorded among the `events`, and an `ExecutionEvent` is recorded among the `events`, resting on no side; any other variant - an undated leaf, a `BookEvent` - is refused at `$.operations[index].kind`; every `transunix` must agree, regression refused, applied in source order; atomic - the book is unchanged on every error; the refusals on this page are this explicit call's, which [the fold](#book-fold) answers by leaving what a book refuses out, with a warning |
| A new instant | the book follows the book it was - its identity and instant as `prevuuid` and `prevunix`, its price and quantity as `prevpx` and `prevqty` - and its `delta`, `events`, place and snapshot stamp start again; live depth stays |
| Snapshots | a full-snapshot order/quote/`SnapshotEvent` clears only its `(symbol, scope)` partition on both sides (an empty FIX `W` is one) before its group applies, and the book states the group's instant as its `snapunix`; which partitions the last snapshot replaced is walk state - no row states it, and neither the digest nor equality reads it |
| Bounds | the highest place of its members at the book's own instant (reset when the instant advances), earliest creation instant and wire clock, latest execution instant (the book's [`execunix`](market.md#contract)) of the finalized generation, after any predecessor advanced it; a rehydrated book checks all four against every nested operation, the place only against those at its instant |
| As a market | `price`: `bbo_midpoint()` - the overflow-safe `(bid + offer) / 2` of a two-sided BBO, per [SEC](https://www.sec.gov/files/rules/sro/btnl/2026/34-106421-ex4.pdf) - else the one best price, `None` if crossed; `quantity`: `median_quantity()` - [NIST](https://www.itl.nist.gov/div898/handbook/eda/section3/eda351.htm) mean of the two best quantities, one-sided its own; currency the best legs agree on and unit the best entries agree on (one-sided: its own; else none); `bidpx`/`bidqty` and `askpx`/`askqty` the best tradable levels, `bidccy`/`askccy` the book's currency beside a best - nothing where no level of a side can trade |
| Following | `with_previous`, [above](#complete-books-and-delta-books): a delta book rebuilt over the book it follows, a complete one linked to it |
| Merging | only one book code at one instant, the reference chosen by its wire clock - the later `sendunix`, then `transunix`: two complete books join their sides - the reference's entries, then each of the other's identities it holds on neither side, unless the reference states a `snapunix`, which is authoritative - and union their `delta` and their `events`, each list apart; a complete book is authoritative over a delta book whichever records later; two delta books keep the reference's facts over the `delta` and `events` of both, the reference's first; self-merge changes nothing |
| Bindings | Python `graph.BookEvent(transunix, key)`, `graph.BookEvent.keyed(transunix, key)`, `with_operations(items)` (a new book), `with_previous(previous)`, the properties `alive`, `delta`, `events`, `ordlive`, `orddelta`, `quotes`, `executions`, `controls` and `is_complete`, the method `alive_on(side)`; JavaScript `new graph.BookEvent(transunix, key)`, `graph.BookEvent.keyed(transunix, key)`, `withOperations(items)`, `withPrevious(previous)`, the methods `alive()`, `delta()`, `events()`, `ordlive()`, `orddelta()`, `quotes()`, `executions()`, `controls()`, `aliveOn(side)` and the property `isComplete` |

## Snapshot controls

`SnapshotEvent::snapshot(&event, scope)` copies the event/market facts of any `Event + Market` (never an operation's), sets the scope under `MdUpdateAction::Snapshot`, finalizes through `digest_market_event`; `book()` reads the control. Python `graph.SnapshotEvent.snapshot(event, scope=None)`, JavaScript `graph.SnapshotEvent.snapshot(event, scope)`.

## Book fold

`BookIterator::new(values, snapshot_millis)` folds sorted `MarketData`/`Result<MarketData>` into `Result<BookEvent>`s, and `with_filter(filter)` narrows what it folds; Python `graph.BookIterator(items, snapshot_millis=0, filter=None)`, JavaScript `new graph.BookIterator(items, snapshotMillis = 0, filter = undefined)`.

| Key | Rule |
| --- | --- |
| Pruned | every input the walk's admission does not take is dropped where it is pulled, a FIX message's leaves once it is split - a `W`/`X` entry stating its own `SecurityID(48)` booked by the code it spells: a kind [`MarketDataKind::is_recorded`](../types/enum/marketdatakind.md) does not admit - a trade, a batch, a session message, `UKNW` - in silence, and a recorded input stating no `instcode` - a ticker-only line no lifecycle filled, a masked number such as `XX0000000001`, a line a full registry could not learn - with one deduplicated [warning](../fix/capture.md#warnings) per instrument it names; a pruned input touches no book, no instant and no grid, so an instant only a trade or a code-less input reached emits no book |
| Recorded | an execution reaches the book its instrument keys and stands among the `events` of its instant, resting on no side and moving none - its fill moved the book through its order's or quote's own report - and dates the book's `execunix`; a book of its instant alone is an event-only delta book, its `delta` empty and its `events` the execution, and `with_previous` replays nothing of it. A snapshot control is recorded among the `events` of the book whose membership it replaced |
| Input | what is kept and is no order, quote, execution or snapshot control - an undated leaf, a `BookEvent`, whatever `instcode` it states - is refused by kind at `$.operation.kind`: a value that is no operation of a book is the caller's mistake, not data; that refusal and a source failure each follow the completed prefix once and fuse the iterator |
| `with_filter(filter)` | an expression [`Filter`](../expression/filters.md) - a filter, a term or its text - over the [`marketdata` row](market-data.md#arrow), bound once, answered by the expression engine over one batch per 1,024 recorded inputs the walk pulls ahead; the kind rule prunes first, so a filter narrows what a book folds and never admits a trade. A filter that keeps every row installs nothing. Refused where it is bound: its own parse error, a column the row does not carry, an answer that is no boolean; a filter that cannot answer a batch ends the walk as a source failure does |
| Books | one per [book key](market.md#the-book-key), the input's `instcode` alone; each book opens keyed - `BookEvent::keyed` - stores its key as `3:0:{key}` and as its own `instcode`, and takes its ticker and ISIN from the first input stating each. `BookIterator` holds no registry and creates nothing: an element built by hand stating a real ISIN and no `instcode` is pruned until an [instruments fill](instrument.md#matching) or a stated `instcode` gives it a code, the FIX lifecycle fills every real-ISIN line's, so the book walk runs over lifecycle output, and a raw parse of ticker-only lines books nothing. A raw parse books each statement by what it states alone: an order booked by its ISIN whose cancel report states only its ticker stays alive, the report pruned - walk the lifecycle's output, which fills the report's code and ends the order |
| Moving between books | an order or quote identity restated under another key - a chain stated under a placeholder's code and then, its body learned, under its instrument's own - is withdrawn from the book it stood in by an entry of its `delta` there: the entry as it stood, deleted in state `REMOVED`, reporting no fill, no execution, no recording and no snapshot instant - a snapshot's member as any other statement - and opens in its own book at the same instant, so an entry rests in one book at a time |
| Groups | by effective instant (`snapunix`, else `transunix`) - one book per touched key, key order, atomic per instant and key; ordinary groups apply via `add_operations`, each chain where its first step arrived, its steps by place only where a step follows one of its chain at that instant, else in arrival order; supplied-membership stages replacement with its `delta` and expirations; a group the book refuses is left out whole, with a warning: none of its updates lands, the book and its pending expirations stand as they were, and the walk goes on |
| Emitted | a book is yielded where it records an entry of its `delta` - every order and quote folded into it but a repeat of the live entry it continues, resting anywhere or not - or an event - an execution, a snapshot control - or at a snapshot tick where it holds an entry; a snapshot emptying a book is yielded too, empty and complete, its control among its `events`, for the books after it to rebuild over, and so is an empty snapshot of an empty book. An instant whose inputs only repeat what the book holds yields no book, and neither does a grid tick finding a book empty that nothing changed there: the next book follows the last one yielded |
| Keyframes | a book is yielded complete - its alive entries, its limits, its instant as its `snapunix` - only at a snapshot tick: every grid tick `snapshot_millis` crosses, the ticks the walk catches up on and the quiet buckets included; and a group that replaced membership - a FIX `W` full refresh, an empty snapshot control, inputs stating `snapunix`. Every other book is a delta book naming the book it follows as its `prevuuid`; a code's first book names none and follows the empty book, `BookEvent::keyed(unix, key)`. With `snapshot_millis = 0` and no snapshot input, every book is a delta book |
| Rebuilding | `with_previous` over the complete book before rebuilds a delta book whole; with no grid, rebuilding the book at an instant reads back to the code's first book or its last full refresh |
| Grid | `snapshot_millis == 0` is none; a positive one yields at every crossed epoch-aligned tick every book holding an entry or changed at the tick, complete, with `snapunix` set - a book changed at the tick carries that instant's `delta` and `events`, any other none; a tick equal to a source or expiration instant applies those first, then one tick regardless |
| Expiry | a live order or quote stating `exprunix` is scheduled once by its identity, whichever sides it rests on: a later statement withdraws its predecessor's deadline, so the schedule holds at most one deadline per identity - never one per amendment - and a group replacing membership drops its book's and schedules every entry alive. At the deadline the walk applies a terminal delete of the scheduled generation, state `EXPIRED`, reporting no fill, placed first at that instant before equal-time source operations, without clearing its snapshot partition; a deadline a later statement, a range or a rename outdated is dropped unapplied and never moves the walk to its instant |
| Supplied membership | a stream stating `snapunix` is a membership view: orders and quotes replace only the `(symbol, scope)` partitions represented (create/update, purge omitted); `SnapshotEvent` is an empty one; later-than-snapshot components are left out with a warning, `add_operations` refusing them; distinct from a FIX `W` |
| Left out | each with a deduplicated [warning](../fix/capture.md#warnings) and never an `Err` item: operations dated before the book they would fold into, and a group the book refuses. A live order or quote resting on no side is warned of and still an entry of its `delta` |
| After each book | the walk hands its `delta` and `events` to the book it yields, so the next carries only its instant's changes; resting depth persists |
| FIX | [`FixCodec::market_data`](../fix/arrow.md#fix-market-books) sorts a capture's market data by the effective instant this fold checks; `FixCodec::book_arrow_reader(messages, snapshot_millis, filter)` - Python `codec.book_arrow_reader(messages, snapshot_millis=0, filter=None)`, JavaScript `codec.bookArrowReader(messages, snapshotMillis, filter)` - runs the fold to Arrow: it folds orders, quotes and `W`/`X` book messages, a quote one entry resting on each leg it states, records every execution among the `events` - an entry reporting a trade (`269=2`) as the execution it is, placing nothing - and prunes the rest before it is expanded |

## Cost

Each side of a complete book is one contiguous store of its live entries in book order, a level a run of equal prices, a two-sided quote sharing its one entry with the other side. A change costs a binary search over the side's levels, one scan of the level it touches - where the entry an identity goes by is found - and a move of the entry pointers behind the one that joins or leaves. Each level keeps its quantity and how many of its entries cannot trade as entries join, restate and leave, so checking a level against `decimal`, settling the top of book and every reading of a level read no entry. A delete by position range, an anonymous entry stating a position and an `MDEntryID` more than one live entry goes by scan the side.

The store is shared: a book a walk yields complete holds the store the walk keeps, so yielding a deep book costs a reference count, and the walk copies a side once at its next change - only while a consumer still holds that book. Between snapshot ticks the walk yields delta books and changes the store it alone holds in place. `with_previous` takes the sides of the book it rebuilds over, copying them once where the first `delta` entry changes them and only while another holder shares them.

Laid out as rows, a complete book repeats every alive entry it holds, so a fine grid over deep books multiplies: `alive entries x ticks` nested rows across the stream, whatever the input's own size. What bounds one batch is the writer's row and byte bounds alone - `FixCodec::with_batch_row_size` and `with_batch_byte_size`, each nested row charged - and a batch casts to a table's stored layout whole however many nested rows it holds: the [materialization budget](../types/serie.md#materialization-budgets) charges a kernel's output nothing. To hold fewer rows, coarsen `snapshot_millis` or narrow the fold with `with_filter`; to hold a batch in less memory, lower the byte bound.

## Examples

### A ladder

Three bids and three offers on Apple, plus a market order to buy 50.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, Market, MarketData, OrderEvent};
    use yggdryl::graph::Element;
    use yggdryl::Decimal;
    use yggdryl_market::{Limit, Side};
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let entry = |code: &str, side: Side, price: Option<&str>, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(T);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_instcode(Some("AAPL".into()), true);
        order.set_side(side, true);
        order.set_price(match price {
            Some(text) => Some(text.parse()?),
            None => None,
        }, true);
        order.set_quantity(Some(Decimal::from_int(quantity)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::keyed(T, "AAPL");
    book.add_operations([
        entry("B-1", Side::Buy, Some("189.48"), 300)?,
        entry("B-2", Side::Buy, Some("189.47"), 500)?,
        entry("B-3", Side::Buy, Some("189.45"), 200)?,
        entry("A-1", Side::Sell, Some("189.52"), 100)?,
        entry("A-2", Side::Sell, Some("189.53"), 400)?,
        entry("A-3", Side::Sell, Some("189.55"), 250)?,
        entry("MKT", Side::Buy, None, 50)?,
    ])?;
    let px = |text: &str| text.parse::<Decimal>();

    // The two bests, and what the book reads and states from them.
    assert_eq!(book.best_price(Side::Buy), Some(px("189.48")?));
    assert_eq!(book.best_quantity(Side::Sell), Some(Decimal::from_int(100)));
    assert_eq!((book.get_bidpx(), book.get_askpx()), (Some(px("189.48")?), Some(px("189.52")?)));
    assert_eq!(book.spread(), Some(px("0.04")?));
    assert_eq!(book.bbo_midpoint(), Some(px("189.50")?));
    assert_eq!(book.get_price(), book.bbo_midpoint());
    assert_eq!(book.median_quantity(), Some(Decimal::from_int(200)));
    assert!(!book.is_locked() && !book.is_crossed());
    assert_eq!(book.imbalance(1), Some(px("0.5")?));

    // One limit per price, best first, the market order last and unpriced.
    let limits: Vec<Limit> = book.limits(Side::Buy).collect();
    let prices: Vec<Option<Decimal>> = limits.iter().map(|limit| limit.price).collect();
    assert_eq!(prices, [Some(px("189.48")?), Some(px("189.47")?), Some(px("189.45")?), None]);
    assert!(limits.iter().all(|limit| limit.tradable), "an entry stating nothing trades");
    assert_eq!(book.depth(Side::Buy, 2), Some(Decimal::from_int(800)));
    assert_eq!(book.depth(Side::Buy, 4), Some(Decimal::from_int(1_050)));
    let best = book.alive().next().expect("the best bid");
    assert_eq!(best.as_order_event().map(Element::get_crosscode), Some("10:1:B-1"));
    // One side's entries alone: the three offers, best first.
    let asks: Vec<Option<Decimal>> = book.alive_on(Side::Sell).map(Market::get_price).collect();
    assert_eq!(asks, [Some(px("189.52")?), Some(px("189.53")?), Some(px("189.55")?)]);
    assert!(book.is_complete(), "a book a caller builds holds its sides");

    // A limit is a value of its own: its field, and its scalar both ways.
    assert_eq!(Limit::field().name(), "limit");
    assert_eq!(Limit::from_scalar(&limits[0].into_scalar())?, limits[0]);
    let row = Limit::dtype().scalar(limits[3].into_scalar())?;
    assert_eq!(Limit::from_scalar(&row)?.price, None);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def entry(code: str, side: str, price: str | None, quantity: int) -> graph.OrderEvent:
        return graph.OrderEvent(
            T,
            crosscode=code,
            ticker="AAPL", instcode="AAPL",
            side=side,
            price=None if price is None else Decimal(price),
            quantity=quantity,
        )

    book = graph.BookEvent(T, "AAPL").with_operations(
        [
            entry("B-1", "BUYS", "189.48", 300),
            entry("B-2", "BUYS", "189.47", 500),
            entry("B-3", "BUYS", "189.45", 200),
            entry("A-1", "SELL", "189.52", 100),
            entry("A-2", "SELL", "189.53", 400),
            entry("A-3", "SELL", "189.55", 250),
            entry("MKT", "BUYS", None, 50),
        ]
    )

    def value(scalar):
        assert scalar is not None
        return scalar.as_py()

    # The two bests, and what the book reads and states from them.
    assert value(book.best_price(Side.BUYS)) == Decimal("189.48")
    assert value(book.best_quantity(Side.SELL)) == 100
    assert (value(book.bidpx), value(book.askpx)) == (Decimal("189.48"), Decimal("189.52"))
    assert value(book.spread) == Decimal("0.04")
    assert value(book.bbo_midpoint) == Decimal("189.50")
    assert book.price == book.bbo_midpoint
    assert value(book.median_quantity) == 200
    assert not book.is_locked and not book.is_crossed
    assert value(book.imbalance(1)) == Decimal("0.5")

    # One limit per price, best first, the market order last and unpriced.
    limits = [limit.as_py() for limit in book.limits(Side.BUYS)]
    assert [limit["price"] for limit in limits] == [Decimal("189.48"), Decimal("189.47"), Decimal("189.45"), None]
    assert all(limit["tradable"] for limit in limits), "an entry stating nothing trades"
    assert value(book.depth(Side.BUYS, 2)) == 800
    assert value(book.depth("BUYS", 4)) == 1_050
    best = book.alive[0].as_order_event()
    assert best is not None and best.crosscode == "10:1:B-1"
    # One side's entries alone: the three offers, best first.
    asks = [entry.as_order_event() for entry in book.alive_on(Side.SELL)]
    assert [ask.price.as_py() for ask in asks if ask is not None] == [Decimal("189.52"), Decimal("189.53"), Decimal("189.55")]
    assert book.is_complete, "a book a caller builds holds its sides"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Side, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const entry = (code, side, price, quantity) => new graph.OrderEvent(T, {
      crosscode: code, ticker: 'AAPL', instcode: 'AAPL', side, price, quantity,
    })
    const book = new graph.BookEvent(T, 'AAPL').withOperations([
      entry('B-1', 'BUYS', '189.48', 300),
      entry('B-2', 'BUYS', '189.47', 500),
      entry('B-3', 'BUYS', '189.45', 200),
      entry('A-1', 'SELL', '189.52', 100),
      entry('A-2', 'SELL', '189.53', 400),
      entry('A-3', 'SELL', '189.55', 250),
      entry('MKT', 'BUYS', undefined, 50),
    ])

    // The two bests, and what the book reads and states from them.
    assert.equal(book.bestPrice('BUYS'), '189.48')
    assert.equal(book.bestQuantity(Side.SELL), '100')
    assert.deepEqual([book.bidpx, book.askpx], ['189.48', '189.52'])
    assert.equal(book.spread, '0.04')
    assert.equal(book.bboMidpoint, '189.5')
    assert.equal(book.price, book.bboMidpoint)
    assert.equal(book.medianQuantity, '200')
    assert.equal(book.isLocked || book.isCrossed, false)
    assert.equal(book.imbalance(1), '0.5')

    // One limit per price, best first, the market order last and unpriced.
    const limits = book.limits('BUYS')
    assert.deepEqual(limits.map((limit) => limit.price), ['189.48', '189.47', '189.45', null])
    assert.ok(limits.every((limit) => limit.tradable), 'an entry stating nothing trades')
    assert.equal(book.depth('BUYS', 2), '800')
    assert.equal(book.depth('BUYS', 4), '1050')
    assert.equal(book.alive()[0].asOrderEvent().crosscode, '10:1:B-1')
    // One side's entries alone: the three offers, best first.
    assert.deepEqual(book.aliveOn('SELL').map((entry) => entry.asOrderEvent().price), ['189.52', '189.53', '189.55'])
    assert.equal(book.isComplete, true, 'a book a caller builds holds its sides')
    ```

### Tradable levels

The best bid is the best level that can trade: a halted top level is skipped, never answered.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, Market, MarketData, Operation, OrderEvent};
    use yggdryl::graph::Element;
    use yggdryl::{Ccy, Decimal};
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let bid = |code: &str, price: &str, quantity: i64, tradable: Option<bool>| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(T);
        order.set_crosscode(code.to_owned());
        order.set_side(Side::Buy, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(quantity)), true);
        order.set_currency(Ccy::new("USD")?, true);
        order.set_instcode(Some("AAPL".into()), true);
        order.set_tradable(tradable, true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::keyed(T, "AAPL");
    book.add_operations([
        bid("B-1", "189.48", 300, Some(false)),
        bid("B-2", "189.47", 500, None),
        bid("B-3", "189.47", 100, Some(false)),
    ])?;

    let tradable: Vec<(String, bool)> = book
        .limits(Side::Buy)
        .map(|limit| (limit.price.map(|px| px.to_string()).unwrap_or_default(), limit.tradable))
        .collect();
    // Every entry at 189.48 says it cannot trade; one at 189.47 says nothing.
    assert_eq!(tradable, [("189.48".to_owned(), false), ("189.47".to_owned(), true)]);
    assert_eq!(book.best_price(Side::Buy), Some("189.47".parse()?));
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(600)));
    assert_eq!(book.get_bidpx(), book.best_price(Side::Buy));
    assert_eq!(book.get_bidccy().map(Ccy::as_str), Some("USD"));

    // No level that can trade: no best, and no bid.
    let mut halted = BookEvent::keyed(T, "AAPL");
    halted.add_operations([bid("B-1", "189.48", 300, Some(false))?])?;
    assert_eq!((halted.best_price(Side::Buy), halted.get_bidpx()), (None, None));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def bid(code: str, price: str, quantity: int, tradable: bool | None) -> graph.OrderEvent:
        return graph.OrderEvent(
            T,
            crosscode=code,
            side="BUYS",
            price=Decimal(price),
            quantity=quantity,
            currency="USD",
            instcode="AAPL",
            tradable=tradable,
        )

    book = graph.BookEvent(T, "AAPL").with_operations(
        [bid("B-1", "189.48", 300, False), bid("B-2", "189.47", 500, None), bid("B-3", "189.47", 100, False)]
    )
    levels = [(limit.as_py()["price"], limit.as_py()["tradable"]) for limit in book.limits(Side.BUYS)]
    # Every entry at 189.48 says it cannot trade; one at 189.47 says nothing.
    assert levels == [(Decimal("189.48"), False), (Decimal("189.47"), True)]
    best = book.best_price(Side.BUYS)
    assert best is not None and best.as_py() == Decimal("189.47")
    assert book.bidpx == best
    assert book.bidccy is not None and book.bidccy.as_py() == "USD"

    # No level that can trade: no best, and no bid.
    halted = graph.BookEvent(T, "AAPL").with_operations([bid("B-1", "189.48", 300, False)])
    assert halted.best_price(Side.BUYS) is None and halted.bidpx is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const bid = (code, price, quantity, tradable) => new graph.OrderEvent(T, {
      crosscode: code, side: 'BUYS', price, quantity, currency: 'USD', instcode: 'AAPL', tradable,
    })
    const book = new graph.BookEvent(T, 'AAPL').withOperations([
      bid('B-1', '189.48', 300, false),
      bid('B-2', '189.47', 500, undefined),
      bid('B-3', '189.47', 100, false),
    ])
    // Every entry at 189.48 says it cannot trade; one at 189.47 says nothing.
    assert.deepEqual(book.limits('BUYS').map((limit) => [limit.price, limit.tradable]), [
      ['189.48', false],
      ['189.47', true],
    ])
    assert.equal(book.bestPrice('BUYS'), '189.47')
    assert.equal(book.bestQuantity('BUYS'), '600')
    assert.equal(book.bidpx, '189.47')
    assert.equal(book.bidccy, 'USD')

    // No level that can trade: no best, and no bid.
    const halted = new graph.BookEvent(T, 'AAPL').withOperations([bid('B-1', '189.48', 300, false)])
    assert.equal(halted.bestPrice('BUYS'), null)
    assert.equal(halted.bidpx, null)
    ```

### Entries by kind

Two bids and an offer on Apple; a second later the first bid is cancelled, a fill is recorded and a better offer quoted.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, ExecutionEvent, Market, MarketData, OrderEvent, QuoteEvent};
    use yggdryl::graph::{Element, Event};
    use yggdryl::{Decimal, State};
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, code: &str, price: &str, state: State| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_instcode(Some("AAPL".into()), true);
        order.set_side(Side::Buy, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(100)), true);
        order.set_state(state);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let offer = |unix: i64, code: &str, price: &str| -> yggdryl::Result<MarketData> {
        let mut quote = QuoteEvent::at(unix);
        quote.set_crosscode(code.to_owned());
        quote.set_ticker(Some("AAPL".into()), true);
        quote.set_instcode(Some("AAPL".into()), true);
        quote.set_side(Side::Sell, true);
        quote.set_price(Some(price.parse()?), true);
        quote.set_quantity(Some(Decimal::from_int(200)), true);
        quote.finalize();
        Ok(MarketData::from(quote))
    };
    let mut fill = ExecutionEvent::at(T + 1);
    fill.set_crosscode("E-1".to_owned());
    fill.set_ticker(Some("AAPL".into()), true);
    fill.set_instcode(Some("AAPL".into()), true);
    fill.set_side(Side::Buy, true);
    fill.set_lastpx(Some("189.47".parse()?), true);
    fill.set_lastqty(Some(Decimal::from_int(100)), true);
    fill.finalize();

    let mut book = BookEvent::keyed(T, "AAPL");
    book.add_operations([
        order(T, "B-1", "189.48", State::New),
        order(T, "B-2", "189.47", State::New),
        offer(T, "Q-1", "189.53"),
    ])?;
    book.add_operations([
        order(T + 1, "B-1", "189.48", State::Canceled),
        Ok(MarketData::from(fill)),
        offer(T + 1, "Q-2", "189.52"),
    ])?;

    // Resting: the orders alive now. Changed: the orders this second applied.
    let resting: Vec<&str> = book.ordlive().map(Element::get_crosscode).collect();
    assert_eq!(resting, ["10:1:B-2"]);
    let changed: Vec<(&str, State)> = book.orddelta().map(|order| (order.get_crosscode(), *order.get_state())).collect();
    assert_eq!(changed, [("10:1:B-1", State::Canceled)]);
    // The quotes this second applied and the executions it recorded; Q-1 still rests.
    assert_eq!(book.quotes().map(Element::get_crosscode).collect::<Vec<_>>(), ["14:0:Q-2"]);
    assert_eq!(book.executions().map(Element::get_crosscode).collect::<Vec<_>>(), ["8:1:E-1"]);
    assert_eq!(book.alive().count(), 3);
    // The orders and quotes partition the delta; the executions and controls the events.
    assert_eq!(book.controls().count(), 0);
    assert_eq!(book.orddelta().count() + book.quotes().count(), book.delta().len());
    assert_eq!(book.executions().count() + book.controls().count(), book.events().len());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import State, graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, code: str, price: str, state: str = "NEW") -> graph.OrderEvent:
        return graph.OrderEvent(
            unix, crosscode=code, ticker="AAPL", instcode="AAPL", side="BUYS", price=Decimal(price), quantity=100, state=state
        )

    def offer(unix: int, code: str, price: str) -> graph.QuoteEvent:
        return graph.QuoteEvent(unix, crosscode=code, ticker="AAPL", instcode="AAPL", side="SELL", price=Decimal(price), quantity=200)

    fill = graph.ExecutionEvent(T + 1, crosscode="E-1", ticker="AAPL", instcode="AAPL", side="BUYS", lastpx=Decimal("189.47"), lastqty=100)
    book = (
        graph.BookEvent(T, "AAPL")
        .with_operations([order(T, "B-1", "189.48"), order(T, "B-2", "189.47"), offer(T, "Q-1", "189.53")])
        .with_operations([order(T + 1, "B-1", "189.48", "CANCELED"), fill, offer(T + 1, "Q-2", "189.52")])
    )

    # Resting: the orders alive now. Changed: the orders this second applied.
    assert [entry.crosscode for entry in book.ordlive] == ["10:1:B-2"]
    assert [(entry.crosscode, entry.state) for entry in book.orddelta] == [("10:1:B-1", State.CANCELED)]
    # The quotes this second applied and the executions it recorded; Q-1 still rests.
    assert [entry.crosscode for entry in book.quotes] == ["14:0:Q-2"]
    assert [entry.crosscode for entry in book.executions] == ["8:1:E-1"]
    assert len(book.alive) == 3
    # The orders and quotes partition the delta; the executions and controls the events.
    assert book.controls == []
    assert len(book.orddelta) + len(book.quotes) == len(book.delta)
    assert len(book.executions) + len(book.controls) == len(book.events)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, code, price, state = 'NEW') => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', instcode: 'AAPL', side: 'BUYS', price, quantity: 100, state,
    })
    const offer = (unix, code, price) => new graph.QuoteEvent(unix, {
      crosscode: code, ticker: 'AAPL', instcode: 'AAPL', side: 'SELL', price, quantity: 200,
    })
    const fill = new graph.ExecutionEvent(T + 1n, {
      crosscode: 'E-1', ticker: 'AAPL', instcode: 'AAPL', side: 'BUYS', lastpx: '189.47', lastqty: 100,
    })
    const book = new graph.BookEvent(T, 'AAPL')
      .withOperations([order(T, 'B-1', '189.48'), order(T, 'B-2', '189.47'), offer(T, 'Q-1', '189.53')])
      .withOperations([order(T + 1n, 'B-1', '189.48', 'CANCELED'), fill, offer(T + 1n, 'Q-2', '189.52')])
    const codes = (entries) => entries.map((entry) => entry.crosscode)

    // Resting: the orders alive now. Changed: the orders this second applied.
    assert.deepEqual(codes(book.ordlive()), ['10:1:B-2'])
    assert.deepEqual(book.orddelta().map((entry) => [entry.crosscode, entry.state]), [['10:1:B-1', 'CANCELED']])
    // The quotes this second applied and the executions it recorded; Q-1 still rests.
    assert.deepEqual(codes(book.quotes()), ['14:0:Q-2'])
    assert.deepEqual(codes(book.executions()), ['8:1:E-1'])
    assert.equal(book.alive().length, 3)
    // The orders and quotes partition the delta; the executions and controls the events.
    assert.deepEqual(book.controls(), [])
    assert.equal(book.orddelta().length + book.quotes().length, book.delta().length)
    assert.equal(book.executions().length + book.controls().length, book.events().length)
    ```

### A snapshot

An empty snapshot of the book's scope a second later: the stale depth goes, nothing is invented, and the control is recorded among the book's `events`.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, Market, MarketData, MdUpdateAction, OrderEvent, SnapshotEvent};
    use yggdryl::graph::Element;
    use yggdryl::Decimal;
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, code: &str, side: Side, price: &str| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_instcode(Some("AAPL".into()), true);
        order.set_side(side, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(100)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::keyed(T, "AAPL");
    book.add_operations([order(T, "B-1", Side::Buy, "189.48")?, order(T, "A-1", Side::Sell, "189.52")?])?;
    assert_eq!(book.alive().count(), 2);

    let mut at = OrderEvent::at(T + 1_000_000_000);
    at.set_ticker(Some("AAPL".into()), true);
    at.set_instcode(Some("AAPL".into()), true);
    at.finalize();
    let control = SnapshotEvent::snapshot(&at, None);
    assert_eq!(control.book().action, Some(MdUpdateAction::Snapshot));

    book.add_operations([MarketData::from(control)])?;
    assert_eq!(book.alive().count(), 0);
    assert_eq!((book.get_price(), book.get_bidpx(), book.get_askpx()), (None, None, None));
    assert_eq!(book.limits(Side::Buy).count(), 0);
    assert_eq!((book.delta().len(), book.controls().count()), (0, 1));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, code: str, side: str, price: str) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", instcode="AAPL", side=side, price=Decimal(price), quantity=100)

    book = graph.BookEvent(T, "AAPL").with_operations([order(T, "B-1", "BUYS", "189.48"), order(T, "A-1", "SELL", "189.52")])
    assert len(book.alive) == 2

    control = graph.SnapshotEvent.snapshot(graph.OrderEvent(T + 1_000_000_000, ticker="AAPL", instcode="AAPL"))
    assert control.book.action == "snapshot"

    after = book.with_operations([control])
    assert after.alive == []
    assert after.price is None and after.bidpx is None and after.askpx is None
    assert after.limits(Side.BUYS) == []
    assert after.delta == [] and len(after.controls) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, code, side, price) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', instcode: 'AAPL', side, price, quantity: 100,
    })
    const book = new graph.BookEvent(T, 'AAPL')
      .withOperations([order(T, 'B-1', 'BUYS', '189.48'), order(T, 'A-1', 'SELL', '189.52')])
    assert.equal(book.alive().length, 2)

    const control = graph.SnapshotEvent.snapshot(new graph.OrderEvent(T + 1_000_000_000n, { ticker: 'AAPL', instcode: 'AAPL' }))
    assert.equal(control.book.action, 'snapshot')

    const after = book.withOperations([control])
    assert.deepEqual(after.alive(), [])
    assert.equal(after.price, null)
    assert.equal(after.bidpx, null)
    assert.deepEqual(after.limits('BUYS'), [])
    assert.deepEqual([after.delta().length, after.controls().length], [0, 1])
    ```

### The book fold

A sorted stream: a bid, then a better bid and a fill a second later. With no grid every book is a delta book and is rebuilt whole over the one before; with a 500 ms grid every book is whole.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, BookIterator, ExecutionEvent, Market, MarketData, Order, OrderEvent};
    use yggdryl::graph::{Element, Event};
    use yggdryl::Decimal;
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    const SECOND: i64 = 1_000_000_000;
    let bid = |unix: i64, code: &str, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_instcode(Some("AAPL".into()), true);
        order.set_side(Side::Buy, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(quantity)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut fill = ExecutionEvent::at(T + SECOND);
    fill.set_crosscode("E-1".to_owned());
    fill.set_ticker(Some("AAPL".into()), true);
    fill.set_instcode(Some("AAPL".into()), true);
    fill.set_side(Side::Buy, true);
    fill.set_lastpx(Some("189.52".parse()?), true);
    fill.set_lastqty(Some(Decimal::from_int(100)), true);
    fill.finalize();
    let stream = || -> yggdryl::Result<Vec<MarketData>> {
        Ok(vec![bid(T, "B-1", "189.48", 300)?, bid(T + SECOND, "B-2", "189.49", 200)?, MarketData::from(fill.clone())])
    };

    // One book per instant an order touched; the fill is recorded among the
    // events beside the better bid in the delta. Each is a delta book: its
    // delta and events alone, beside the top of book they settled on.
    let books = BookIterator::new(stream()?.into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(books.len(), 2);
    assert!(books.iter().all(|book| !book.is_complete()));
    let last = &books[1];
    assert_eq!((last.get_transunix(), last.delta().len(), last.events().len(), last.alive().count()), (T + SECOND, 1, 1, 0));
    assert_eq!(last.best_price(Side::Buy), Some("189.49".parse()?));
    assert_eq!((books[0].get_prevuuid(), last.get_prevuuid()), (None, Some(books[0].get_uuid())));

    // Rebuilt: the first over the empty book a walk starts from, the next over it.
    let first = books[0].clone().with_previous(&BookEvent::keyed(T, "AAPL")).expect("a rebuild");
    let whole = last.clone().with_previous(&first).expect("a rebuild");
    assert!(whole.is_complete());
    assert_eq!(whole.alive().count(), 2, "depth persists");
    assert_eq!(whole.get_uuid(), last.get_uuid(), "one book, one identity");

    // A 500 ms grid yields every book whole, the crossed tick between included.
    let gridded = BookIterator::new(stream()?.into_iter(), 500)?.collect::<yggdryl::Result<Vec<_>>>()?;
    let ticks: Vec<(i64, usize, usize, bool)> = gridded
        .iter()
        .map(|book| (book.get_transunix() - T, book.delta().len(), book.events().len(), book.is_complete()))
        .collect();
    assert_eq!(ticks, [(0, 1, 0, true), (500_000_000, 0, 0, true), (SECOND, 1, 1, true)]);

    // A value a book does not fold is refused by its kind - stating its
    // instcode, so it is admitted rather than pruned as code-less.
    let mut undated = Order::new();
    undated.set_instcode(Some("AAPL".into()), true);
    undated.finalize();
    let mut refused = BookIterator::new([MarketData::from(undated)].into_iter(), 0)?;
    let error = refused.next().expect("one result").unwrap_err();
    assert!(error.to_string().contains("$.operation.kind"), "{error}");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000
    SECOND = 1_000_000_000

    def bid(unix: int, code: str, price: str, quantity: int) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", instcode="AAPL", side="BUYS", price=Decimal(price), quantity=quantity)

    fill = graph.ExecutionEvent(
        T + SECOND, crosscode="E-1", ticker="AAPL", instcode="AAPL", side="BUYS", lastpx=Decimal("189.52"), lastqty=100
    )
    stream = [bid(T, "B-1", "189.48", 300), bid(T + SECOND, "B-2", "189.49", 200), fill]

    # One book per instant an order touched; the fill is recorded among the
    # events beside the better bid in the delta. Each is a delta book: its
    # delta and events alone, beside the top of book they settled on.
    books = list(graph.BookIterator(stream))
    assert len(books) == 2
    assert not any(book.is_complete for book in books)
    last = books[1]
    assert (last.transunix, len(last.delta), len(last.events), len(last.alive)) == (T + SECOND, 1, 1, 0)
    best = last.best_price(Side.BUYS)
    assert best is not None and best.as_py() == Decimal("189.49")
    assert (books[0].prevuuid, last.prevuuid) == (None, books[0].uuid)

    # Rebuilt: the first over the empty book a walk starts from, the next over it.
    first = books[0].with_previous(graph.BookEvent.keyed(T, "AAPL"))
    assert first is not None
    whole = last.with_previous(first)
    assert whole is not None and whole.is_complete
    assert len(whole.alive) == 2, "depth persists"
    assert whole.uuid == last.uuid, "one book, one identity"

    # A 500 ms grid yields every book whole, the crossed tick between included.
    gridded = list(graph.BookIterator(stream, snapshot_millis=500))
    assert [(book.transunix - T, len(book.delta), len(book.events), book.is_complete) for book in gridded] == [
        (0, 1, 0, True),
        (500_000_000, 0, 0, True),
        (SECOND, 1, 1, True),
    ]

    # A value a book does not fold is refused by its kind - stating its
    # instcode, so it is admitted rather than pruned as code-less.
    try:
        list(graph.BookIterator([graph.Order(instcode="AAPL")]))
    except ValueError as error:
        assert "$.operation.kind" in str(error) and "got order" in str(error)
    else:
        raise AssertionError("an undated order was folded")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const SECOND = 1_000_000_000n
    const bid = (unix, code, price, quantity) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', instcode: 'AAPL', side: 'BUYS', price, quantity,
    })
    const fill = new graph.ExecutionEvent(T + SECOND, {
      crosscode: 'E-1', ticker: 'AAPL', instcode: 'AAPL', side: 'BUYS', lastpx: '189.52', lastqty: 100,
    })
    const stream = [bid(T, 'B-1', '189.48', 300), bid(T + SECOND, 'B-2', '189.49', 200), fill]

    // One book per instant an order touched; the fill is recorded among the
    // events beside the better bid in the delta. Each is a delta book: its
    // delta and events alone, beside the top of book they settled on.
    const books = [...new graph.BookIterator(stream)]
    assert.equal(books.length, 2)
    assert.ok(books.every((book) => !book.isComplete))
    const last = books[1]
    assert.equal(last.transunix, T + SECOND)
    assert.deepEqual([last.delta().length, last.events().length, last.alive().length], [1, 1, 0])
    assert.equal(last.bestPrice('BUYS'), '189.49')
    assert.equal(books[0].prevuuid, null)
    assert.equal(last.prevuuid, books[0].uuid)

    // Rebuilt: the first over the empty book a walk starts from, the next over it.
    const first = books[0].withPrevious(graph.BookEvent.keyed(T, 'AAPL'))
    const whole = last.withPrevious(first)
    assert.equal(whole.isComplete, true)
    assert.equal(whole.alive().length, 2, 'depth persists')
    assert.equal(whole.uuid, last.uuid, 'one book, one identity')

    // A 500 ms grid yields every book whole, the crossed tick between included.
    const gridded = [...new graph.BookIterator(stream, 500)]
    assert.deepEqual(gridded.map((book) => [book.transunix - T, book.delta().length, book.events().length, book.isComplete]), [
      [0n, 1, 0, true], [500_000_000n, 0, 0, true], [SECOND, 1, 1, true],
    ])

    // A value a book does not fold is refused by its kind - stating its
    // instcode, so it is admitted rather than pruned as code-less.
    assert.throws(() => [...new graph.BookIterator([new graph.Order({ instcode: 'AAPL' })])], /\$\.operation\.kind.*got order/)
    ```

### A filtered walk

A bid, an offer and a fill: the fill is recorded among its book's `events`, and a filter over the `marketdata` row narrows what folds.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookIterator, ExecutionEvent, Market, MarketData, OrderEvent};
    use yggdryl::graph::{Element, Event};
    use yggdryl::Decimal;
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, code: &str, side: Side| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_instcode(Some("AAPL".into()), true);
        order.set_side(side, true);
        order.set_price(Some("189.50".parse()?), true);
        order.set_quantity(Some(Decimal::from_int(100)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut fill = ExecutionEvent::at(T + 2);
    fill.set_crosscode("E-1".to_owned());
    fill.set_ticker(Some("AAPL".into()), true);
    fill.set_instcode(Some("AAPL".into()), true);
    fill.set_side(Side::Buy, true);
    fill.set_lastqty(Some(Decimal::from_int(100)), true);
    fill.finalize();
    let inputs = || -> yggdryl::Result<Vec<MarketData>> {
        Ok(vec![order(T, "B-1", Side::Buy)?, order(T + 1, "A-1", Side::Sell)?, MarketData::from(fill.clone())])
    };
    let walk = |filter: Option<&str>| -> yggdryl::Result<Vec<i64>> {
        let mut walk = BookIterator::new(inputs()?.into_iter(), 0)?;
        if let Some(filter) = filter {
            walk = walk.with_filter(filter)?;
        }
        walk.map(|book| book.map(|book| book.get_transunix() - T)).collect()
    };

    // The fill's instant yields a delta book: its delta empty, its events the execution.
    assert_eq!(walk(None)?, [0, 1, 2]);
    // The ask never folds, so its instant yields none; the buy-side fill does.
    assert_eq!(walk(Some("side = 'BUYS'"))?, [0, 2]);
    // A filter narrows: the execution's instant alone.
    assert_eq!(walk(Some("marketdatakind = 'EXEC'"))?, [2]);
    // A column the row does not carry is refused where the filter is bound.
    assert!(walk(Some("nope = 1")).is_err());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, code: str, side: str) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", instcode="AAPL", side=side, price=Decimal("189.50"), quantity=100)

    fill = graph.ExecutionEvent(T + 2, crosscode="E-1", ticker="AAPL", instcode="AAPL", side="BUYS", lastqty=100)
    inputs = [order(T, "B-1", "BUYS"), order(T + 1, "A-1", "SELL"), fill]

    def walk(filter: str | None = None) -> list[int]:
        return [book.transunix - T for book in graph.BookIterator(inputs, filter=filter)]

    # The fill's instant yields a delta book: its delta empty, its events the execution.
    assert walk() == [0, 1, 2]
    # The ask never folds, so its instant yields none; the buy-side fill does.
    assert walk("side = 'BUYS'") == [0, 2]
    # A filter narrows: the execution's instant alone.
    assert walk("marketdatakind = 'EXEC'") == [2]
    # A column the row does not carry is refused where the filter is bound.
    try:
        walk("nope = 1")
    except ValueError:
        pass
    else:
        raise AssertionError("an unknown column was bound")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, code, side) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', instcode: 'AAPL', side, price: '189.50', quantity: 100,
    })
    const fill = new graph.ExecutionEvent(T + 2n, { crosscode: 'E-1', ticker: 'AAPL', instcode: 'AAPL', side: 'BUYS', lastqty: 100 })
    const inputs = [order(T, 'B-1', 'BUYS'), order(T + 1n, 'A-1', 'SELL'), fill]
    const walk = (filter) => [...new graph.BookIterator(inputs, 0, filter)].map((book) => book.transunix - T)

    // The fill's instant yields a delta book: its delta empty, its events the execution.
    assert.deepEqual(walk(), [0n, 1n, 2n])
    // The ask never folds, so its instant yields none; the buy-side fill does.
    assert.deepEqual(walk("side = 'BUYS'"), [0n, 2n])
    // A filter narrows: the execution's instant alone.
    assert.deepEqual(walk("marketdatakind = 'EXEC'"), [2n])
    // A column the row does not carry is refused where the filter is bound.
    assert.throws(() => walk('nope = 1'))
    ```

`FixCodec::book_arrow_reader` takes the same filter as its third argument.

### Books by key

One instant, three inputs: a security stating its real ISIN as its code, an FX pair stating its class and body, and a ticker-only input no registry filled, which states no code and is pruned.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookIterator, Market, MarketData, OrderEvent};
    use yggdryl::graph::Element;
    use yggdryl::Decimal;
    use yggdryl_market::{IdKey, IdType, Identifier, Side};
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |code: &str, ticker: &str, isin: Option<&str>, instcode: Option<&str>| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(T);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some(ticker.into()), true);
        if let Some(isin) = isin {
            order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), isin)?)?;
        }
        order.set_instcode(instcode.map(Into::into), true);
        order.set_side(Side::Buy, true);
        order.set_price(Some("42.10".parse()?), true);
        order.set_quantity(Some(Decimal::from_int(10)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let inputs = [
        // A security: its real ISIN is its instrument's code.
        order("B-1", "AAPL", Some("US0378331005"), Some("US0378331005"))?,
        // An FX pair: its class and body, no ISIN stated.
        order("B-2", "EUR/USD", None, Some("IF:EUR/USD"))?,
        // A ticker nobody resolved: no code, no book.
        order("B-3", "HOLN", None, None)?,
    ];

    let books = BookIterator::new(inputs.into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
    let keys: Vec<(&str, Option<&str>, Option<&str>, Option<&str>)> = books
        .iter()
        .map(|book| (book.get_crosscode(), book.get_instcode(), book.get_ticker(), book.get_isincode()))
        .collect();
    // The instcode alone keys a book, which states it as its own, in key order;
    // each book states the ticker and the ISIN its first input stated, and the
    // code-less input is pruned before the walk.
    assert_eq!(
        keys,
        [
            ("3:0:IF:EUR/USD", Some("IF:EUR/USD"), Some("EUR/USD"), None),
            ("3:0:US0378331005", Some("US0378331005"), Some("AAPL"), Some("US0378331005")),
        ]
    );
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Identifier, graph

    T = 1_700_000_000_000_000_000

    def order(code: str, ticker: str, isin: str | None, instcode: str | None) -> graph.OrderEvent:
        facts: dict = {"ticker": ticker}
        if isin is not None:
            facts["securityids"] = [Identifier("isin", isin)]
        if instcode is not None:
            facts["instcode"] = instcode
        return graph.OrderEvent(T, crosscode=code, side="BUYS", price=Decimal("42.10"), quantity=10, **facts)

    books = list(graph.BookIterator([
        order("B-1", "AAPL", "US0378331005", "US0378331005"),  # a security: its real ISIN is its code
        order("B-2", "EUR/USD", None, "IF:EUR/USD"),  # an FX pair: its class and body
        order("B-3", "HOLN", None, None),  # a ticker nobody resolved: no code, no book
    ]))
    # The instcode alone keys a book, which states it as its own, in key order;
    # each book states the ticker and the ISIN its first input stated, and the
    # code-less input is pruned before the walk.
    assert [(book.crosscode, book.instcode, book.ticker, book.isincode) for book in books] == [
        ("3:0:IF:EUR/USD", "IF:EUR/USD", "EUR/USD", None),
        ("3:0:US0378331005", "US0378331005", "AAPL", "US0378331005"),
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (code, ticker, isin, instcode) => new graph.OrderEvent(T, {
      crosscode: code,
      side: 'BUYS',
      price: '42.10',
      quantity: 10,
      ticker,
      instcode,
      securityids: isin === undefined ? undefined : [new Identifier('isin', isin)],
    })

    const books = [...new graph.BookIterator([
      order('B-1', 'AAPL', 'US0378331005', 'US0378331005'), // a security: its real ISIN is its code
      order('B-2', 'EUR/USD', undefined, 'IF:EUR/USD'), // an FX pair: its class and body
      order('B-3', 'HOLN'), // a ticker nobody resolved: no code, no book
    ])]
    // The instcode alone keys a book, which states it as its own, in key order;
    // each book states the ticker and the ISIN its first input stated, and the
    // code-less input is pruned before the walk.
    assert.deepEqual(books.map((book) => [book.crosscode, book.instcode, book.ticker, book.isincode]), [
      ['3:0:IF:EUR/USD', 'IF:EUR/USD', 'EUR/USD', null],
      ['3:0:US0378331005', 'US0378331005', 'AAPL', 'US0378331005'],
    ])
    ```

### Moving between books

An option stated under a placeholder number, then restated a millisecond later under its own code once its body is known: it leaves the placeholder's book and rests in the code's.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookIterator, Market, MarketData, OrderEvent};
    use yggdryl::graph::{Element, Event};
    use yggdryl::{Decimal, State};
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    const MS: i64 = 1_000_000;
    const PLACEHOLDER: &str = "DE000C000001";
    const OPTION: &str = "OC:US0378331005:2026-12-18:200";
    let order = |unix: i64, price: &str, state: State, instcode: &str| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("C-1".to_owned());
        order.set_ticker(Some("ACME".into()), true);
        order.set_instcode(Some(instcode.into()), true);
        order.set_side(Side::Buy, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(10)), true);
        order.set_state(state);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let inputs = [
        order(T, "100", State::New, PLACEHOLDER)?,
        order(T + MS, "101", State::Replaced, OPTION)?,
    ];

    let books = BookIterator::new(inputs.into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
    let keyed: Vec<(i64, &str)> = books.iter().map(|book| (book.get_transunix() - T, book.get_crosscode())).collect();
    assert_eq!(keyed, [(0, "3:0:DE000C000001"), (MS, "3:0:DE000C000001"), (MS, "3:0:OC:US0378331005:2026-12-18:200")]);
    // The placeholder's book takes the entry off by one entry of its delta, reporting no fill.
    assert_eq!(books[1].delta().len(), 1);
    let withdrawn = books[1].delta().next().and_then(MarketData::as_order_event).expect("an order");
    assert_eq!((withdrawn.get_state(), withdrawn.get_lastqty()), (&State::Removed, None));
    // The entry rests in one book at a time.
    assert_eq!(books[1].best_price(Side::Buy), None);
    assert_eq!(books[2].best_price(Side::Buy), Some("101".parse()?));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, State, graph

    T = 1_700_000_000_000_000_000
    MS = 1_000_000
    PLACEHOLDER = "DE000C000001"
    OPTION = "OC:US0378331005:2026-12-18:200"
    placed = graph.OrderEvent(T, crosscode="C-1", ticker="ACME", instcode=PLACEHOLDER, side="BUYS", price=Decimal("100"), quantity=10, state="NEW")
    restated = graph.OrderEvent(
        T + MS,
        crosscode="C-1",
        ticker="ACME",
        instcode=OPTION,
        side="BUYS",
        price=Decimal("101"),
        quantity=10,
        state="REPLACED",
    )

    books = list(graph.BookIterator([placed, restated]))
    assert [(book.transunix - T, book.crosscode) for book in books] == [
        (0, "3:0:DE000C000001"),
        (MS, "3:0:DE000C000001"),
        (MS, "3:0:OC:US0378331005:2026-12-18:200"),
    ]
    # The placeholder's book takes the entry off by one entry of its delta, reporting no fill.
    [delta] = books[1].delta
    withdrawn = delta.as_order_event()
    assert withdrawn is not None and withdrawn.state is State.REMOVED and withdrawn.lastqty is None
    # The entry rests in one book at a time.
    assert books[1].best_price(Side.BUYS) is None
    best = books[2].best_price(Side.BUYS)
    assert best is not None and best.as_py() == Decimal("101")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const MS = 1_000_000n
    const PLACEHOLDER = 'DE000C000001'
    const OPTION = 'OC:US0378331005:2026-12-18:200'
    const placed = new graph.OrderEvent(T, {
      crosscode: 'C-1', ticker: 'ACME', instcode: PLACEHOLDER, side: 'BUYS', price: '100', quantity: 10, state: 'NEW',
    })
    const restated = new graph.OrderEvent(T + MS, {
      crosscode: 'C-1',
      ticker: 'ACME',
      instcode: OPTION,
      side: 'BUYS',
      price: '101',
      quantity: 10,
      state: 'REPLACED',
    })

    const books = [...new graph.BookIterator([placed, restated])]
    assert.deepEqual(books.map((book) => [book.transunix - T, book.crosscode]), [
      [0n, '3:0:DE000C000001'], [MS, '3:0:DE000C000001'], [MS, '3:0:OC:US0378331005:2026-12-18:200'],
    ])
    // The placeholder's book takes the entry off by one entry of its delta, reporting no fill.
    const [delta] = books[1].delta()
    const withdrawn = delta.asOrderEvent()
    assert.equal(withdrawn.state, 'REMOVED')
    assert.equal(withdrawn.lastqty, null)
    // The entry rests in one book at a time.
    assert.equal(books[1].bestPrice('BUYS'), null)
    assert.equal(books[2].bestPrice('BUYS'), '101')
    ```

## Edges

- An order or quote with no price (a market order) rests at its side's unpriced level: `best_price` skips it, `limits(side)` answers it last with no price, `depth` counts it once reached; a side of such entries states no best, a book of such sides states no `price`, `spread`, `bidpx` or `askpx`.
- A level whose aggregate quantity would pass `decimal` is refused atomically at `$.quantity`, naming the price (or unpriced level), the book unchanged; `imbalance`/`spread` answer `None` rather than overflow.
- A leg stating a negative quantity is refused at `$.quantity` (`expected a quantity no less than zero, got -5`), and in a book's row at `$.alive[i].quantity`: a level adds only what rests at it, so taking an entry off can only shrink it.
- A book folds a dated order or quote and a snapshot control, and records a dated execution: `add_operations` prunes a trade and refuses an undated leaf or a `BookEvent` at `$.operations[index].kind`, `BookIterator` at `$.operation.kind`, naming the kind it got (`expected order_event, quote_event, execution_event or snapshot_event, got order`).
- An order of side `UKNW`, or a quote stating no leg, rests on neither side: it is an entry of its book's `delta` that places nothing, warned of where it is live, never refused; the book it folded into stands with what else its instant stated.
- An execution rests on no side: a fill moves its book through its order's or quote's own report, which a FIX parse splits off the execution, and the execution is recorded among the book's `events`. A trade never reaches a book: its fills arrive as the executions the parse splits off it.
- A delta book takes no operations - `add_operations` refuses it at `$.alive` - and answers no side: rebuild it with `with_previous` first. A row cannot carry the completeness of a complete book holding no live entry that states a `delta` or `events` entry and no `snapunix`: it reads back as a delta book.
