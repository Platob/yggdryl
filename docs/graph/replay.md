---
hide:
  - navigation
  - toc
---

# Replay

The order book along its timeline, as the package walked it: one component, `BookTimeline`, over the books the native `BookIterator` answers - a scrubber over their instants, the live limits standing at the chosen one, and an audit of the whole book.

## Contract

| | |
| --- | --- |
| Component | `yggdryl/web/book-timeline.js`: `new BookTimeline({ books, index, title }).mount(parent)`, `update({ books, index })`, `select(index)`, `openAudit()`, `destroy()`; each book is `bookJson` of a walk's `BookEvent`, in walk order. Emits `ygg:book-select` with `{ index, curruuid, currunix }` when the instant moves |
| Timeline | A scrubber and previous/next buttons over the walk, one mark per book at its instant's place in the walk's span (every n-th past 400 books); `←`, `→`, `Home` and `End` move it. It stands at the last book when books arrive |
| Live limits | Bid and ask side by side, best first as the native side answers them: price, quantity and the live entries resting there, each bar as deep as its quantity against the deepest limit of the book; a limit new since the book before, or whose quantity or live entries moved, is marked. The unpriced limit, where market orders rest, reads `market` |
| Audit | `Audit book` opens the book standing now in a dialog sized to the viewport - at most 90% of its height, its body scrolling: the book's facts, each side with each limit and the live entries at it (joined by `curruuid`), the executions and the deltas, every one a collapsible item built when it first opens; `Expand all` and `Collapse all` act on every item |
| Service | `node node/replay.js <source>` serves `GET /api/sources` and `GET /api/sources/:source/books?symbol=` (server-sent `book` events in walk order, then `end` with `{ count }`), and the page at `/web/app/`: a source and a symbol chosen in its header, the component below |
| Facts | Every price, quantity, instant, spread, midpoint and hash is what the package answered; the page folds, orders and computes nothing |

## The replay

<div class="ygg-rp" data-replay="app" markdown="1">
This section runs the recorded replay from `assets/replay.json` and needs JavaScript.
</div>

`synthetic` is 23 operations over `ALPHA` and `BETA` at fixed instants - quote ladders, limit orders two of which stand one nanosecond apart, a market order resting at the unpriced limit, executions, a composite trade, an order that expires, a full snapshot of `BETA` - walked into 6 `ALPHA` books, 5 `BETA` books and 9 consolidated ones. `ulbridge` is the ULBridge capture the Rust suite reads: 11 market operations and one refusal, walked into 7 books whose last `stableHash`, 4619727780541450139, is the one `rust/tests/fix/ulbridge.rs` pins.

## Serve a source

```bash
node node/replay.js synthetic
node node/replay.js rust/tests/fix/ulbridge.log --registry config/fix
node node/replay.js marketdata.parquet --global
```

A source is a `.arrow`/`.ipc`/`.feather`/`.parquet` file of `marketdata` rows, the word `synthetic`, or any other file as a FIX capture read under the row header (the ULBridge one by default) and dated by `--sending-time` where a line states none. `--snapshot-millis` walks on a grid, `--global` walks one consolidated book, `--port` picks the port; the command prints the URL it serves at.

## Edges

- A capture's refused messages are counted in the header's status, whose tooltip lists each as the package worded it; the walk reads the rest.
- An operation read from a capture names the lines it came from in `srcuuids`. A line's identity is seeded by the name of the buffer the capture is read into, which differs on every read, so the recorded identities differ between two builds in everything but their millisecond and row sequence; the drift check compares that part alone.
- The page follows the documentation's colour scheme when it opens.

## Commands

```bash
node --test node/tests/web/book-timeline.test.js node/tests/web/app.test.js node/tests/replay/server.test.js
node scripts/build_docs_replay.js
node scripts/build_docs_replay.js --check
```
