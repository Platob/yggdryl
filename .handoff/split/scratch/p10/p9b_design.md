# P9b: design - books keyed by instcode

The user's decision 16 (`$S/user_decisions.md:61-72`, 2026-10-10): "Make then book use the instcode
as crosscode and check correct book iterator generations", refined to "Only instcode, filterout
messages with non attributed instcode. Since all isin should have clear [auto] created instrument in
registry". Read as: the book key is the element's `instcode` **alone** - no fallback to the ISIN, the
ticker or `Isin::NONE`; an element with no `instcode` is pruned before the book walk, as an
unrecorded kind is; every element stating a real ISIN has its instrument auto-created and its
`instcode` filled, so only a ticker-only or code-less element goes unbooked; the book iterator's
generation is verified adversarially. It supersedes the cross-code decision's item 5
(`$S/p9/crosscode_decision.md:351-353`, books keyed by the code deferred). `$S` is
`.handoff/split/scratch`. Its own commit right after P9, before P10 (`$S/p10/d44_design.md`).

Read on the mid-P9 working tree (2026-10-10 ~07:45Z); every line number is of that tree. No command
was run to write this.

## What the tree holds today

| Fact | Where |
| --- | --- |
| `Market::book_crosscode()`, provided: the `isin` identifier whatever its rank (a masked `XX0000000001` keys a book), else a non-empty ticker, else `Isin::NONE` (`XX0000000000`), every arm borrowed | `rust/market/src/graph/market.rs:460-493` |
| `BookIterator` keys books by it: `let symbol = input.book_crosscode().to_owned()` per pulled input, the `touched` set, the snapshot members/controls/partitions, the expirations and the keyframes all per symbol string | `rust/market/src/graph/book.rs:4183-4235,3943-4062,4238-4345` |
| Pruning before the fold: by kind alone - `recorded(input) = marketdatakind().is_recorded()` in `pull`, a FIX message's leaves once split; `foldable` refuses every variant but the four | `book.rs:3846-3882,4400-4425` |
| The FIX admission: `contributes_to_market(&message) && message.marketdatakind().is_recorded()` on the message before it is expanded | `rust/fix/src/market.rs:391-398,436-460` |
| `BookEvent::keyed(unix, key)` the empty base a code's first book rebuilds over; `BookEvent::new(unix, symbol)` keys by the ticker and states it, an empty symbol keying `Isin::NONE`; `Default` is `new(0, "")`; `adopt_instrument` takes the ticker and the ISIN from the first input | `book.rs:1946-2036` |
| `book_mismatch`: an input stands in a book where its own key is the book's **or** it states neither an ISIN nor a ticker (`stated == Isin::NONE` stands in any book) | `book.rs:4903-4912` |
| The withdrawal: an identity (`LiveKey`) restated under another key leaves the book it stood in by a `REMOVED` delta and opens in its own; `booked: HashMap<LiveKey, String>` records where a live identity stands | `book.rs:4065-4110` |
| The FIX book scope's last fallback is `event.book_crosscode()` (`Symbol=XX0000000000` for a message stating neither ticker nor identifier) | `rust/fix/src/market.rs:2185-2215` |
| A candle is one OHLC per book cross code (`book.get_crosscode()`, `3:0:{key}`) | `rust/market/src/graph/candle.rs:829-835` |
| `instcode`: a `Market` holder, written at the parse where the code is a function of the message alone (a stated real ISIN under no body class, a detected pair's `IF:EUR/USD`), filled by the lifecycle from the resolved instrument (`fill_unsettled`, `overwrite = false`), followed along a chain, fed to no digest | `rust/fix/src/msg.rs:4038-4069`; `rust/market/src/instrument.rs:3737-3792`; `rust/market/src/graph/market.rs:1357-1363`; `d42_design.md:266-310` |
| Every element stating a real ISIN **already** auto-creates its instrument: `learn_stating` keys the statement by the stated real ISIN where the class keys no body (`Production::Isin`), and `fill_unsettled` writes `instcode` where none | `instrument.rs:4562-4659` (`isin = is_stated(Isin).filter(is_real)`), `:405-507` (`write_code`), `:3787-3790` |
| The capture's books: keys `CH0012005267`, `CH0012214059`, `CH0012221716`, `EZN11TD1F7K3`, `TW0001605004`, `TW0002454006`, `XX0000000001` (a masked line), eleven books over 21 operations; the HOLN ticker-only line stands in Holcim's book through the ticker index | `rust/fix/tests/root/ulbridge.rs:1565-1695`; `python/tests/test_fix.py:800-808` |
| The equivalence snapshot holds **no** book row (`grep -c "3:0:"` = 0) | `rust/fix/tests/root/equivalence.snapshot` |

## The decision

1. **The key is `Market::get_instcode()`**, and nothing else. `book_crosscode()` is retired
   (`market.rs:460-493`, its doc example, `.api-inventory.txt:1594`): a provided reading that
   answered `get_instcode()` would be a second spelling of one verb. `BookIterator::fill_pending`
   reads `input.get_instcode()` (`book.rs:4183`); `book_mismatch` compares `operation.get_instcode()`
   to the book's key, an input stating none refused at `$.operation.instcode` ("expected book
   crosscode `X`, got none") - the "stands in any book" arm (`:4909-4911`) is gone with the number
   that states none; `BookEvent::keyed(unix, key)` sets the book's own `instcode` to `key`
   (`event.set_instcode(Some(key), true)` beside `set_crosscode("3:0:{key}")`, `:2005-2020`), so a
   book row's `instcode` cell is its key and a reader joins books to the instruments table as every
   other market table does; `adopt_instrument` (`:2023-2036`) keeps taking the ticker and the ISIN
   from the first input as facts of the book.
2. **`BookEvent::new(unix, symbol)` is deleted in the core** (`:1946-1962`): it keyed a book by a
   ticker and stated the ticker, two things the key no longer does; `keyed` is the one core
   constructor, `Default` `keyed(0, "")` - a book keyed by nothing, which takes nothing, since no
   element states an empty `instcode` (`stored_crosscode` keeps an empty code empty,
   `market.rs:732-737`, so its cross code is empty). **The bindings' constructors are re-spelled, not
   retired**: Python's `#[new] BookEvent(transunix, symbol)` (`python/src/graph/book.rs:63-66`) and
   Node's `#[napi(constructor)] new BookEvent(transunix, symbol)` (`node/src/graph/book.rs:86-90`)
   both redirect to `CoreBookEvent::new` today, and a door is re-spelled where the core signature it
   redirects to moved (AGENTS §4: never retired unasked) - each becomes `BookEvent(transunix, key)`
   redirecting to `BookEvent::keyed(transunix, key)`, the same door as the `keyed` static, its doc
   saying so; the alternative, their removal in favour of the static alone, is put to the user (#2).
   The call sites - `BookEvent::new(` and `BookEvent.new(`/`BookEvent(`/`new BookEvent(` occur 164
   times across 26 files (`grep -rn "BookEvent::new(\|BookEvent.new(\|new BookEvent(\|BookEvent("`,
   2026-10-10, `target/` and `node_modules/` out): `rust/market/tests/graph/{book (76), arrow (16),
   candle (13), market (6), market_data (2), view (1), facts (1)}.rs`, `rust/market/src/graph/{book
   (12), market_data (11), arrow (4), market (1)}.rs` (doc examples), `rust/market/benchmarks/graph/
   {book, candle, view}.rs`, `rust/market/tests/allocations.rs` (5), `rust/fix/tests/root/market.rs`
   (6), `rust/fix/tests/graph/market_data.rs` (2), `rust/fix/benchmarks/fix/pipeline.rs` (2),
   `python/tests/graph/{test_book (22), test_candle (10), test_market_data (3), test_operation (2)}.py`,
   `python/src/graph/market_data.rs` (2), `node/tests/graph/{book (14), candle (13), market_data (4),
   index (4), iterator (2)}.test.js`, `node/tests/fix.test.js` (2), `node/benchmarks/graph.js` (2),
   `docs/graph/{book (17), quote (3), market-data (3), candle (3)}.md`, `docs/types/enum/side.md` (3),
   `skills/yggdryl-market-data/SKILL.md` (1) and `references/{javascript (5), rust (3), python (3)}.md`
   - are one exact-string sweep to `keyed` in Rust and to the re-spelled constructor or `keyed` in the
   bindings, driven by the compiler's list, the Python and Node fixtures stating `instcode = symbol`
   as the Rust ones do (decision 7: their `order()`/`quote()` helpers state only a ticker today, so
   `with_operations` would refuse at `$.operation.instcode` without it); a site that asserted the
   ticker `new` stated asserts the ticker the first input stated instead (`rust/market/tests/graph/arrow.rs`'s
   ticker pins among them).
3. **Pruning**: one predicate beside `recorded` in `book.rs:4400-4402` - `booked(input) =
   input.marketdatakind().is_recorded() && input.get_instcode().is_some()` - read by `pull`
   (`:3864,3869`) for every leaf, a FIX message's once split; and by the FIX admission
   (`fix/market.rs:450-452`) on the message before it is expanded, so a code-less message costs no
   expansion (a message's leaves carry its `instcode`: `MarketFacts` is copied into each leaf by
   `into_market_data`). A pruned input touches no book, no instant and no grid, exactly as an
   unrecorded kind does today (`.api-inventory.txt:1769`'s sentence, kept with `instcode` added).
4. **Every real ISIN auto-creates its instrument** - already the lifecycle's rule (`learn_stating`,
   above) - so the pin to write is the contract: a walk over a line stating a real ISIN and nothing
   else learns the instrument and fills `instcode`, and the book walk books it; a ticker-only line
   whose ticker the registry does not know, a masked `XX...` line and a `ZZ`/typo ISIN line
   (`Isin::rank_of` 1 or 0, `rust/src/isin.rs:233-238`) hold no `instcode` and are pruned. The book
   walk therefore runs over **lifecycle output** (the medallion's `silver.fix_messages`,
   `python/tests/medallion.py:345-361`; `codec.book_arrow_reader(codec.lifecycle(..))` in the FIX
   tests) or over parses stating real ISINs or pairs; a raw parse of ticker-only lines books nothing,
   which `docs/graph/book.md` and `docs/fix/arrow.md` say in one line each. The auto-creation is the
   **FIX lifecycle's** (`learn_and_fill`): a market element built in Rust, Python or JavaScript
   stating a real ISIN and no `instcode`, walked through `BookIterator` directly, is pruned, and is
   booked once `Instruments::enrich`/`fill` (or a stated `instcode`) has given it a code -
   `docs/graph/book.md` says so beside the pruning rule, it is put to the user (#8), and
   `a_hand_built_real_isin_element_is_pruned_until_enriched` pins both halves. While the registry is
   **full** (`max_instruments` reached, `learn` skipped, no `instcode` filled) every real-ISIN line
   of an unknown instrument is pruned too: the admission warns once per key naming the dropped book
   input (`"book input pruned: {isin} states no instcode"`), through `warned!`, so the disappearance
   is never silent; pinned over a registry of `max_instruments` 1.
5. **The FIX book scope** (`fix/market.rs:2194-2205`): the last fallback becomes `Isin::NONE`
   alone - today's `event.book_crosscode()` there is reached only after the symbol, the ticker, the
   ISIN and the pair arms failed, so it always answered `XX0000000000`, and `Isin::NONE` keeps the
   scope byte for byte; an `instcode` rung is **not** added, since the scope is a component of an
   entry's identity (`crossuuid`) and a message stating a lifecycle-filled code and none of the four
   would move its entry's identity between a raw parse and a walked one. `XX0000000000` keeps meaning
   "a message stating neither ticker nor identifier" there (`docs/fix/message.md:53` unchanged).
6. **Candles** follow: one OHLC per `3:0:{instcode}`; nothing in `candle.rs` changes but its
   fixtures' inputs.
7. **The hand-built fixtures**: `rust/market/tests/graph/book.rs:33-65`'s `operation(kind, symbol,
   ...)` helper states `instcode = symbol` - the fixture's own instrument code, as it states its own
   `crosscode` and `ENTRY_ID` - so the ~100 book-mechanics pins (`3:0:IBM`, `3:0:ACME`, one book per
   symbol and instant, the sides, the deltas) **stand unchanged** and test what they tested; the few
   tests **about the key** (below) are re-spelled with real ISIN and `class:body` codes. The
   alternative - a fixture table ticker → ISIN moving every `3:0:IBM` pin - is put to the user.

## The pins that move, each with its sentence

| Pin | Moves to | Sentence |
| --- | --- | --- |
| `rust/market/tests/graph/market.rs:1018-1056` `book_crosscode_is_the_isin_else_the_ticker_else_the_default`, `:426,477` | deleted; `the_book_key_is_the_instcode_alone`: an element's book is its `instcode`, borrowed (`std::ptr::eq` against `get_instcode()`), a masked ISIN, a ticker, a market and a class key nothing | "the book key is the instcode alone, decision 16" |
| `rust/market/tests/allocations.rs:893` `a_book_crosscode_allocates_nothing_for_any_input` | `the_book_key_is_borrowed_off_the_instcode`: 0 allocations for an inline code and for the 30-byte `OC:US0378331005:2026-12-18:200` (one refcount's clone happens in `fill_pending`'s `to_owned`, as today, once per input - unchanged) | "decision 16" |
| `rust/market/tests/graph/book.rs:2874-2918` `an_input_is_booked_by_its_isin_else_its_ticker_else_the_default` | `an_input_is_booked_by_its_instcode_and_a_codeless_one_is_pruned`: inputs with `instcode` `US0378331005`, `CH0012214059`, `IF:EUR/USD` are three books `3:0:US0378331005`, `3:0:CH0012214059`, `3:0:IF:EUR/USD` (the FX book under its **code**, no longer its minted number); the `unticked` inputs (`:2838-2846`) and a ticker-only input are no book; the book's `instcode` cell is its key | "the FX book took its code, decision 16" |
| `:2920-2998`, `:3000-3041`, `:3043-3074` (a chain restated under its ISIN leaves its ticker book, by a snapshot, member then restatement) | re-spelled as the **re-key** case: a chain first stated under the placeholder code `DE000C000001` (a real-ISIN placeholder) and then under `OC:US0378331005:2026-12-18:200` (the body learned) leaves the first book by a `REMOVED` delta reporting no fill and opens in the second at the same instant, within one instant and across two, and under a snapshot member - the same withdrawal mechanics (`book.rs:4065-4110`), the key moved | "an entry restated under another instcode is withdrawn, decision 16" |
| `:969-1019` `a_ticker_a_registry_filled_files_the_element_under_the_instruments_book` | the unfilled ticker-only quote is **pruned** (no book), the filled one (`Instruments::enrich` writes `instcode`) stands in `3:0:CH0012214059` | "a code-less element is pruned before the walk, decision 16" |
| `:3136-3209` `each_book_refuses_an_input_keyed_elsewhere` | the refusal names `$.operation.instcode`; the "neither ISIN nor ticker stands in any book" clauses become "no instcode is refused in every book"; `BookEvent::new(1, "")` → `keyed(1, "")`, cross code empty | "decision 16" |
| `:3211` `a_default_books_entry_expires_in_its_own_book` | a book keyed by a code; "default book" is no more | "decision 16" |
| `:942-966`, `:3975` and every `3:0:IBM`/`3:0:ACME` pin | **stand** (the fixture states `instcode = symbol`) | - |
| `rust/market/tests/graph/candle.rs:253,363-366,385` (`3:0:ACME`, `3:0:AAPL`/`3:0:IBM`, `3:0:XX0000000000`) | the first two stand (the `quote` fixture `:34` states `instcode = symbol`); `:374-389` `a_book_stating_no_ticker_states_none_on_its_candle` keys its quotes by a code and asserts the ticker none | "decision 16" |
| `rust/market/tests/graph/facts.rs:203,214`, `market.rs:822-829` (`3:0:ACME`, `3:0:AAPL`) | stand (`keyed`) | - |
| `rust/fix/tests/root/forex.rs:141` `message.book_crosscode() == "QYLTVIRYHNX5"` | `get_instcode() == Some("IF:EUR/USD")` is the key; the `isincode` `QYLTVIRYHNX5` stays a fact | "the FX book took its code, decision 16" |
| `rust/fix/tests/root/market.rs:897-938` `an_fx_pairs_book_is_keyed_by_its_minted_number` | `..._by_its_code`: `[("3:0:IF:EUR/USD", 1, 0), ("3:0:IF:EUR/USD", 1, 1)]` | "the FX book took its code, decision 16" |
| `rust/fix/tests/root/market.rs:940-993` `a_chain_restated_under_its_isin_rests_in_the_instruments_book_alone` | the first line (`Symbol(55)` alone, no instrument) is pruned; the chain is booked from the statement whose `instcode` the lifecycle fills; the `3:0:ACME` rows go: `[("3:0:US0378331005", 1, 0), ("3:0:US0378331005", 1, 1)]` | "a code-less element is pruned, decision 16" |
| `rust/fix/tests/root/codec.rs:53` `last.book_crosscode() == "US0378331005"` | `last.get_instcode() == Some("US0378331005")` | - |
| `rust/fix/tests/root/ulbridge.rs:1565-1695` (eleven books, `stood == booked`, the keys set of seven, `last.book_crosscode()`), `:1804` | `booked` filters `is_recorded() && get_instcode().is_some()`; the masked line's `XX0000000001` book leaves the set (six keys: `CH0012005267`, `CH0012214059`, `CH0012221716`, `EZN11TD1F7K3`, `TW0001605004`, `TW0002454006`). **The expected count is derived from the capture, not from the run**: `rust/tests/support/ulbridge.log` holds exactly **one** line stating `XX0000000001` (`grep -c`, 2026-10-10: `35=UL`, `52=20260814-12:46:58`), so the masked key has one instant and, where that line is one recorded input, one delta book - eleven books → **ten**, the operations 21 → 21 less that line's recorded inputs; phase 2 counts that line's inputs (`codec.market_data` over it alone) before the sweep and writes the two numbers here; a run answering anything else is a defect. `last.get_instcode() == Some("TW0002454006")` | "a masked number keys no book, decision 16" |
| `python/tests/test_fix.py:788,800-808` | the same six keys; `(last.isincode, last.crosscode)` unchanged | "decision 16" |
| `python/tests/test_fix.py:5775-5796` (medallion `silver.books` rows, delta/executions sums) | the rows less the masked key's books, the delta and executions sums less that line's recorded inputs - the same two numbers as the `ulbridge.rs` row, written before the run; `silver.books` keeps `instcode` and gains `crosscode == f"3:0:{instcode}"` | "decision 16" |
| `.api-inventory.txt:1105,1594,1744,1747,1769`; `.api-bindings.txt:266` (Python's constructor row) and the Node `graph` row | `book_crosscode` row gone; `BookEvent`, `keyed`, `BookIterator` sentences re-spelled (the key the `instcode`, a code-less input pruned, the core `new` gone, the bindings' constructor `BookEvent(transunix, key)` redirecting to `keyed`) | - |
| `docs/graph/market.md:18,160-175` ("The book key" table), `:359,484-488,545,929`; `docs/graph/book.md:14,24,51,72-73,82,98,967-969,995-997,1021-1023,1063,1096-1098,1133` (and the one line on a non-FIX element booked only once enriched); `docs/graph/candle.md:39,125,177,221,362,405,444`; `docs/graph/market-data.md:188`; `docs/graph/quote.md`, `docs/types/enum/side.md` (their `BookEvent::new` examples); `docs/graph/instrument.md:322,357,951,1433`; `docs/fix/message.md:53`; **`docs/fix/arrow.md`**: `:17` ("does not infer or apply lifecycle enrichment" gains "so a parse whose lines state no instcode books nothing"), `:185` (the old key rule - the ISIN whatever its rank, else the ticker, else `XX0000000000` - becomes the `instcode` alone with a code-less input pruned) and its three examples (`:200-256` Rust, `:292-303` Python, `:338-348` JavaScript), which run `BookIterator` and `book_arrow_reader` over a raw parse of `55=AAPL` lines and assert `books.len() == 2` and `rows == 1` - under P9b those inputs hold no `instcode`, are pruned and yield 0, so each is rewritten over lines stating a real ISIN (`48=US0378331005|22=4`) and asserts the same counts under `3:0:US0378331005`; `docs/fix/index.md:18` (the Arrow row's sentence); **`skills/yggdryl-fix/SKILL.md:98,148`** and `references/{rust.md:687, python.md:603-638, javascript.md:581-612}` (the same `55=AAPL` book recipes, rewritten the same way, `num_rows == 1` kept over a real-ISIN line); `skills/yggdryl-market-data/SKILL.md:53,168,208`, `references/{rust:485-508,756,811, python:383-399,618,687, javascript:348-366,552,602-607}.md`; `AGENTS.md:480` (the `graph/` row's `book_crosscode` and `BookIterator` sentences); `$S/p7/d40_design.md:121` (D40.4's "`book_crosscode` reads the lifted `isin`" amended to the `instcode` key, since P7 lands after P9b) | the book key is `instcode`; `keyed` the one core constructor; a code-less element pruned; `book_arrow_reader`'s contract restated on its page | - |

**Pins that must not move**: every market element's `crossuuid`/`crosshashcode` and `uuid` (the
key reads a holder and feeds nothing); the equivalence snapshot (no book row in it: a moved cell is a
defect); the FIX `FIX_PIPELINE_COSTS` (`rust/fix/tests/allocations.rs:1946-1986`; the book walk is
not in them); every instrument pin of P9; the FX detection's wire pins (`forex.rs` loses the one
`book_crosscode` line and gains the `instcode` key).

## The adversarial verification of the book iterator's generation

Each a test red first, in `rust/market/tests/graph/book.rs` unless named otherwise:

1. **One book stream per instcode.** Three instruments interleaved over four instants - a security
   (`US0378331005`), an FX pair (`IF:EUR/USD`, its `isincode` the minted `QYLTVIRYHNX5`) and an
   option (`OC:US0378331005:2026-12-18:200`, a 30-byte code) - give exactly three book chains, each
   book's `prevuuid` the previous book of **its own key** (`the_books_of_one_key_chain_by_prevuuid`,
   `:3975`, extended), no book of one key naming another's, the option's book apart from its
   underlying's.
2. **An entry restated under another key is withdrawn.** The placeholder re-key (above): the entry
   leaves `3:0:DE000C000001` by one `REMOVED` delta reporting no fill (`lastpx`/`lastqty` none,
   `MdUpdateAction::Delete`) and opens in `3:0:OC:...` at the same instant; `booked` holds the
   identity under the new key; a third statement under the new key moves nothing; across two
   instants, within one, and with the first key's snapshot member pending at that instant (the
   member leaves the snapshot, `:4089-4092`).
3. **Snapshot ticks whole.** With a grid, every instcode-keyed book is emitted complete at each tick
   (`emit_snapshot`, `:3906-3927`), a pruned code-less input between two ticks moves no book and
   opens no key, and an empty book no group changed at the tick is skipped as today.
4. **A code-less input is pruned at the pull**: a quote with a ticker and no `instcode`, an order
   with a masked ISIN, a `W` book message's entry stating no `instcode`, each between two booked
   inputs of one instant - the instant's book is the two inputs' and the pruned one is in no delta,
   no event, no `touched` set; `BookIterator::with_filter` (`:3816`) narrows what remains and never
   admits a pruned input.
5. **Candles keyed by the book.** `rust/market/tests/graph/candle.rs`: two codes interleaved emit in
   code order under `3:0:{instcode}`; a code-less quote is in no candle.
6. **The FIX walk end to end** (`rust/fix/tests/root/market.rs`): `book_arrow_reader(codec.lifecycle(..))`
   over three lines - a real ISIN, a ticker-only line whose instrument the registry knows (booked by
   the fill's `instcode`), a ticker-only line nobody knows (pruned) - gives two books of one key;
   the admission filter prunes the third before expansion (an `internals` counter of expanded
   messages, as `fix::forex::internals` counts parses).
7. **The capture** (`ulbridge.rs`, `test_fix.py`): six keys, every execution recorded in exactly one
   book, no book records an order among its events, `stood == booked` under the new predicate, the
   HOLN ticker-only line in Holcim's book through the ticker index's fill; ten books where the
   masked line is one recorded input (derived above).
7b. **A hand-built element** (`rust/market/tests/graph/book.rs`): an order stating a real ISIN and no
   `instcode`, walked directly, is pruned; the same order after `Instruments::enrich` over a
   registry that learned it stands in `3:0:<isin>`; over a registry of `max_instruments` 1 already
   full, it is pruned with one warning naming the dropped input.
8. **The medallion's books table** (`test_fix.py:5775-5796`): `silver.books` rows, delta and
   executions sums re-pinned from the run; every book row's `instcode` equals its `crosscode`'s key
   (`crosscode == f"3:0:{instcode}"`) - a new assertion; `silver.orders`/`quotes`/`executions` rows'
   `instcode` non-null.

## Implementation plan

One commit, one push, CI read to `CI result`; phases in AGENTS order; the compiler's list drives the
`BookEvent::new` sweep.

| Phase | Files | Smoke (exact) |
| --- | --- | --- |
| 1 market core | `rust/market/src/graph/market.rs` (`book_crosscode` deleted), `graph/book.rs` (`booked`, `pull`, `fill_pending`, `book_mismatch`, `keyed` setting `instcode`, `new` deleted, docs), `graph/{candle,market_data,arrow}.rs` (doc examples); tests `rust/market/tests/graph/{book,market,candle,facts,arrow,view,market_data}.rs` (the sweep `BookEvent::new(` → `BookEvent::keyed(` by one script asserting each anchor once; the `operation`/`quote` fixtures stating `instcode = symbol`; the pins above), `rust/market/tests/allocations.rs:893`, `rust/market/benchmarks/graph/{book,candle,view}.rs` | `cargo check -p yggdryl-market --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-market --test graph book`; `--test graph market`; `--test graph candle`; `--test graph facts`; `--test graph market_data`; `--test allocations book`; `cargo test -p yggdryl-market --doc graph::book::BookEvent::keyed`; `cargo test -p yggdryl-market --doc graph::book::BookIterator`; `RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-market --no-deps` |
| 2 FIX | `rust/fix/src/market.rs` (the admission `&& message.get_instcode().is_some()` with its warning, `book_scope`'s fallback `Isin::NONE`); tests `rust/fix/tests/root/{market,forex,codec,ulbridge}.rs`, `rust/fix/tests/graph/market_data.rs`, `rust/fix/benchmarks/fix/pipeline.rs` | `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-fix --test root market`; `--test root forex`; `--test root codec`; `--test root ulbridge`; `cargo test -p yggdryl-fix --test root the_codec_answers_what_it_answered` (the snapshot **must not move**); `cargo test -p yggdryl-fix --test allocations a_real_line_costs_the_same_at_every_stage_every_time` (must not move); `cargo test -p yggdryl-fix --test scale_ulbridge` (prints `SKIPPED`, must compile) |
| 3 Python | `python/src/graph/book.rs:63-66` (the constructor re-spelled to `(transunix, key)` over `CoreBookEvent::keyed`), `python/src/graph/market_data.rs` (its doc examples), `python/tests/graph/{test_book,test_candle,test_iterator,test_market_data,test_operation}.py` (the fixtures stating `instcode = symbol`), `python/tests/typing_bindings.py`, `python/tests/test_fix.py` (`:788,800-808,5775-5796`), `python/yggdryl/_native.pyi` (the constructor's doc: a book keyed `key`, no ticker stated) | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/graph/test_book.py python/tests/graph/test_candle.py -x -q`; `... -m pytest python/tests/test_fix.py -k "books or medallion or capture" -x -q`; §3's `mypy --strict` line |
| 4 Node | `node/src/graph/book.rs:86-90` (the constructor re-spelled to `(transunix, key)` over `CoreBookEvent::keyed`), `node/binding.{js,d.ts}`, `node/tests/graph/{book,candle,market_data,index,iterator}.test.js` (the fixtures stating `instcode = symbol`), `node/tests/fix.test.js`, `node/benchmarks/graph.js`; then `node/index.js`, `node/index.d.ts` | `npm run --prefix node build:debug`; `node --test node/tests/graph/book.test.js`; `node --test node/tests/graph/candle.test.js`; `node --test node/tests/fix.test.js`; `npm run --prefix node test:package:debug` |
| 5 docs, skills, inventories, contract | the pages and skills named above - `docs/fix/arrow.md`, `docs/fix/index.md`, `skills/yggdryl-fix/SKILL.md` and `references/{rust,python,javascript}.md` among them, their book examples rewritten over real-ISIN lines; `.api-inventory.txt`, `.api-bindings.txt` (the constructor rows re-spelled, `book_crosscode` gone); `AGENTS.md:480`; `$S/p7/d40_design.md:121` | `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_api_inventory.py`; the three `python scripts/check_docs_examples.py --lang {rust,python,javascript}` as chain steps; `node scripts/build_docs_playground.js --check`; `node scripts/build_docs_fix.js --check` |
| 6 the chain | `cargo test -p yggdryl-market --all-targets --all-features --no-fail-fast`; `-p yggdryl-fix` the same; clippy both crates `-D warnings`; `cargo doc -D warnings`; `pytest python/tests`; `npm test --prefix node`; the docs runners | one background script, one log, read once |

Then `cargo fmt --all` once, the commit (the attribution lines the harness states), one push, the CI
run read to `CI result`, the results commit (DESIGN.md gains the P9b section and results,
`MARKET_SPLIT_NEXT.md`'s `State`/`Checks`/`Next`).

## Put to the user (interpretations taken; say if another was meant)

1. **`book_crosscode()` is retired**, the key read as `get_instcode()` wherever a book keys; the
   alternative keeps it as an alias (two spellings of one verb).
2. **The core's `BookEvent::new(unix, symbol)` is deleted**, `keyed` the one core constructor,
   `Default` a book keyed by nothing that takes nothing; 164 call sites in 26 files swept. **The
   bindings' constructors `BookEvent(transunix, key)` / `new BookEvent(transunix, key)` are kept and
   re-spelled** to redirect to `keyed` (a Node door is never retired unasked); the alternative removes
   them in favour of the `keyed` static.
3. **The hand-built book fixtures state `instcode = symbol`**, so the mechanics pins stand; the
   alternative moves every `3:0:IBM` pin to a real ISIN.
4. **The FX book is keyed `3:0:IF:EUR/USD`** (the code), its `isincode` the minted number as a fact
   - decision 16's own example.
5. **The book walk runs over lifecycle output**: a raw parse of ticker-only lines books nothing; the
   docs say so in one line.
6. **The FIX book scope keeps `XX0000000000`** as the symbol of a message stating neither ticker nor
   identifier nor `instcode`: a scope component, not a book key.
7. **Not in P9b**: a book keyed by `(instcode, mic)` (P10's rows are per market; a book is per
   instrument - decision 16 says "only instcode"); any instrument change (P10).
8. **Auto-creation is the FIX lifecycle's**: a market element built by hand in Rust, Python or
   JavaScript stating a real ISIN and no `instcode` is booked only once `Instruments::enrich`/`fill`
   or a stated `instcode` gave it a code - `BookIterator` holds no registry and creates nothing. The
   alternative, a registry handed to `BookIterator`, is a second fill door.
9. **A full registry prunes the real-ISIN lines it could not learn**, each dropped book input warned
   once per key; the alternative books them under their ISIN, a second key spelling.
