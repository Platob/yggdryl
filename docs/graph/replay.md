---
hide:
  - navigation
  - toc
---

# Replay

The trading replay as the package answered it: the application below walks two recorded sources book by book, and shows one scenario's re-run against the base.

## Contract

| | |
| --- | --- |
| Source | `scripts/build_docs_replay.js` runs [`yggdryl/replay`](components.md#the-replay-service) over the synthetic scenario and over `rust/tests/fix/ulbridge.log`, read from its bytes under `config/fix` with the seed clock 2024-01-02T10:15:30Z |
| Manifest | `docs/assets/replay.json`, committed and checked for drift by the addon build job, beside the component library the same script copies into `docs/assets/web/` |
| Browser | Renders the manifest only; every book, limit, spread and diff is what the package answered when the manifest was built |
| Application | The package's own `yggdryl/web/app/app.js`, mounted read-only in a frame over an `api` that answers from the manifest with the method surface [`createApi`](components.md#the-application) has; a route the manifest does not hold is refused, and says so |
| Manifest keys | `listing` and `field`: what the service itself answered at `/api/sources` (the sources, their symbols, the view names) and `/api/field`; `sources.<id>`: `operations` (every leaf's row), `books` (per symbol and `GLOBAL`, each in walk order), `lifecycle` (`columns` once, `rows` per cross code), `views` (every view bare and `orders` also with two lifts, each answer - or the refusal - with the walks it holds for), `calls` (the JavaScript and the route behind each answer); `sources.synthetic` adds `scenario` (`name`, `inserted`, `events`, `from`, `books`) and `diff` per symbol |

## The replay

<div class="ygg-rp" data-replay="app" markdown="1">
This section runs the recorded replay from `assets/replay.json` and needs JavaScript.
</div>

Choose the source in the header. `synthetic` is 23 operations over `ALPHA` and `BETA` at fixed instants - quote ladders, limit orders two of which stand one nanosecond apart, a market order resting at the unpriced limit, executions, a composite trade, an order that expires, a full snapshot of `BETA` - walked into 6 `ALPHA` books, 5 `BETA` books and 9 consolidated ones. `ulbridge` is the ULBridge capture the Rust suite reads: 11 market operations and one refusal, `invalid record value at $.NoSides(552)[0].Side(54): expected a bid or ask side, got no value`, walked into 7 books whose last `stableHash`, 4619727780541450139, is the one `rust/tests/fix/ulbridge.rs` pins.

## Views

<div class="ygg-rp" data-replay="views" markdown="1">
This section renders `assets/replay.json` and needs JavaScript.
</div>

The `orders` view with two lifts, each a map key read as a column of its own: `securityids['ISIN'] as isin` and `metadata['tech.clientid'] as clientid`. The dotted key is one key. No order in either source states `tech.clientid`, so that column is null in every row: an absent key is a null cell, never a refusal. The view keeps the order kinds alone, so it answers the same whichever walk the replay reads.

## Scenario

<div class="ygg-rp" data-replay="scenario" markdown="1">
This section renders `assets/replay.json` and needs JavaScript.
</div>

`alpha-lock` inserts two events into the synthetic stream, each built by its own constructor and admitted by the walk before it is stored: a bid at 82.25 two milliseconds in, which narrows the `ALPHA` spread to 0.25, and an ask at that same price at 3.5 milliseconds, which locks the book. The re-run starts at the earliest instant the scenario touches; before it every book is the base's, and a diff row names each change by the fact it changed - a limit by its price, an entry or a delta by its `curruuid`.

## Edges

- The page inserts nothing: the insert modal says why, and saving, renaming, deleting or removing an event is refused. [`node node/replay.js synthetic`](components.md#the-replay-service) is the same application with the service behind it.
- The manifest holds the walks without a snapshot grid: turning the grid on is refused, and the grid, the symbol and every panel stay on the walk shown.
- The manifest holds every view bare, and `orders` also with the two lifts above; other lifts are refused. The `lifecycle` tab follows the chain chosen in the ladder or the tape - the lifecycle the manifest records for that cross code, which is what the view answers for it - and with none chosen asks nothing and says how to choose one.
- An operation read from the capture names the lines it came from in `srcuuids`. A line's identity is seeded by the name of the buffer the capture is read into, which differs on every read, so those identities differ between two builds of the manifest in everything but their millisecond and row sequence; the drift check compares that part alone.
- The scenario is recorded over `synthetic` only; asked of `ulbridge`, it is refused.
- The application follows the page's colour scheme when it opens; its own theme switch decides after that.

## Commands

Change a source, the scenario or the application, then regenerate and check for drift.

```bash
node scripts/build_docs_replay.js
node scripts/build_docs_replay.js --check
```
