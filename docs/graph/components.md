# Components

`yggdryl/web/*` is the browser component library of the trading replay and `yggdryl/replay` the service that feeds it: every book, limit, reading, refusal and diff on screen is what the native package answered through the Node binding.

## Contract

| Surface | Contract |
| --- | --- |
| Package | `yggdryl/web/<name>.js` are ES modules shipped in the Node package (`exports["./web/*"]`; `web/package.json` is `{"type":"module"}`), with no dependency, bundler or CDN; `yggdryl/web/theme.css` holds the tokens and every `ygg-ui` class; `yggdryl/replay` is the service, CommonJS beside `yggdryl` |
| Facts | Rendered, never computed: no fold, no ladder aggregation, no spread or imbalance, no identity, no digest, no FIX parse and no re-ordering happen in JavaScript. A book is the [`BookIterator`](index.md#book-fold) walk's, a reading is the native book's, a refusal is the native constructor's, verbatim |
| Values | An instant is `bigint` nanoseconds, crossing as its decimal text; a decimal is its exact text, read as a float only to place a pixel; a map is its `[key, value]` pairs in the native key order |
| Components | One class over `Component`: `new X(props)`, `mount(parent)`, `update(state)`, `destroy()`; one root element `el`; at most one redraw per animation frame; intents leave as bubbling `ygg:*` `CustomEvent`s and a component never reaches into another |
| Service | Node `http` over loaded sources: JSON routes under `/api/`, books as server-sent events, named scenarios kept as files, the components at `/web/`, the application at `/web/app/` |
| Runtime | JavaScript only: the Rust core and the Python package hold the graph the service walks; see [Graph](index.md) |

## Use

Serve the synthetic scenario on an ephemeral port, list its source, and ask for the ALPHA book standing at its first instant.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { createReplayServer, loadSource } = require('yggdryl/replay')

    async function main() {
      // Serve the synthetic scenario on an ephemeral port.
      const service = createReplayServer({ sources: [loadSource('synthetic')] })
      const url = await service.listen(0)
      try {
        const answer = await fetch(new URL('/api/sources', url))
        const { sources } = await answer.json()
        assert.deepEqual(
          sources.map((source) => [source.id, source.operations, source.symbols]),
          [['synthetic', 23, ['ALPHA', 'BETA', 'GLOBAL']]],
        )
        // The book standing at an instant: the latest at or before it.
        const book = await (await fetch(new URL('/api/sources/synthetic/book?symbol=ALPHA&at=1700000000000000000', url))).json()
        const [best] = book.bid.limits
        assert.equal(best.price, '82')
        assert.equal(best.quantity, '100')
        assert.equal(book.spread, '0.5')
        assert.equal(book.stableHash, '16242326805217559900')
      } finally {
        await service.close()
      }
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

## The replay service

`loadSource(spec, options)` reads one source through the native package's own door and `createReplayServer({ sources })` serves every source it is handed.

| `spec` | Read through | Options |
| --- | --- | --- |
| `'synthetic'` | the deterministic leaves over `ALPHA` and `BETA`: 23 operations at fixed instants, two of them one nanosecond apart | none |
| a `.arrow`, `.ipc`, `.feather` or `.parquet` file of `marketdata` rows | `IOBase.from(path).readArrowReader()` into `graph.MarketData.fromArrowReader` | `id` |
| any other file, a FIX capture | its bytes, `readTextLines` under the row header, `FixCodec.parseTextLine`, `lifecycle`, `marketOperations` | `rowheader` (the ULBridge header), `sendingTime` (2024-01-02T10:15:30Z; `null` for UTC now), `registry` (a `FixRegistry` or its folder; the process default when `null`), `timezone` (`UTC`), `id` |

A loaded source is `{ id, kind, name, operations, symbols, refusals }`: the operations as the core answered them, the symbols the per-symbol walk emits books for, and every message a capture refused, verbatim. Instants that go back are refused at load, naming the two operations; a missing file reads as no operations.

`createReplayServer({ sources, stateDir?, webDir?, snapshotMillis = 0, global = false })` answers `{ server, stateDir, listen(port = 0, host = '127.0.0.1'), close() }`: `listen` answers the base URL, `close` ends every open stream and removes the state folder when the service made it itself.

| Method | Route | Answers |
| --- | --- | --- |
| `GET` | `/api/field` | `{ columns: [{ name, dtype, nullable }], kinds }`: the `marketdata` columns and the six kinds an inserted event may name |
| `GET` | `/api/sources` | `{ snapshotMillis, global, views, sources: [{ id, kind, name, operations, symbols, refusals }] }`: `views` the view names the package lists, `GLOBAL` listed once after each source's symbols |
| `GET` | `/api/sources/:source/books?symbol&from&to&snapshotMillis&global` | server-sent events: `event: book` per book in walk order, then `event: end` with `{ count }` |
| `GET` | `/api/sources/:source/book?symbol&at&snapshotMillis&global` | the book standing at `at` - the latest at or before it, the last without `at` |
| `GET` | `/api/sources/:source/lifecycle?crosscode` | `{ columns, rows }`: the `lifecycle` view over the operations |
| `GET` | `/api/sources/:source/view?view&lift&crosscode&snapshotMillis&global` | `{ columns, rows }`: the named [view](index.md#views) over the operations, then the walk's books, each `lift` a column the plan names |
| `GET` | `/api/scenarios` | `{ scenarios: [{ name, events }] }` |
| `GET`, `PUT`, `DELETE` | `/api/scenarios/:name` | the scenario; `PUT` stores `{ events }`, each event rebuilt, admitted by the walk and re-rendered; `DELETE` answers `204` |
| `POST` | `/api/scenarios/:name/events` | body `{ kind, currunix, facts }`: `201` and the stored event, the scenario created on its first event; `400` and the native refusal |
| `DELETE` | `/api/scenarios/:name/events/:curruuid` | `204` |
| `GET` | `/api/sources/:source/scenarios/:name/books?symbol&from&to&snapshotMillis&global` | server-sent events over the merged stream walked afresh, from `from`, else from the earliest instant the scenario affects; `end` carries `{ count, from }` |
| `GET` | `/api/sources/:source/scenarios/:name/diff?symbol&from&to` | `{ symbol, from, instants: [{ at, base, scenario, changes }] }` |
| `GET` | `/web/*` | the component library; a path ending in `/` answers that folder's `index.html` |
| `GET` | `/web/app/` | the application; `/`, `/app`, `/app/` and `/web/app` answer `308` to it, the query kept |

A refusal is `{ error }`: `400` for a request the native package or the route refused, with its message verbatim; `404` for an unknown route, source, symbol, scenario or event; `405` with `Allow`; `413` for a body over 64 MiB; `403` for a path leaving the served folder; `500` for a stored scenario that cannot be read back. A scenario name is lowercase, `[a-z0-9][a-z0-9._-]{0,127}`.

The command line serves one source and prints the URL:

```bash
node node/replay.js synthetic --port 8080
node node/replay.js rust/tests/fix/ulbridge.log --registry config/fix --sending-time 2024-01-02T10:15:30Z
```

| Option | Default | Reads |
| --- | --- | --- |
| `--port` | `0`, an ephemeral port | the port to listen on |
| `--snapshot-millis` | `0` | the grid a request that names none walks at |
| `--global` | off | walk one consolidated `GLOBAL` book |
| `--rowheader` | the ULBridge header | a FIX capture's row header |
| `--sending-time` | 2024-01-02T10:15:30Z | the clock an undated FIX message takes: nanoseconds or ISO 8601 |
| `--registry` | the process default | the FIX dictionary folder |
| `--state` | `~/.config/yggdryl/replay` | where scenarios are kept |
| `--web` | the package's `web/` | the application and the components |

## JSON shapes

One renderer, `node/replay/json.js`, reads a native stream's Arrow columns once per column type: a decimal is its exact text, an instant the decimal text of its nanoseconds, a UUID its hyphenated text, a 64-bit integer its decimal text, a narrower integer a number, a map its `[key, value]` pairs, and `stableHash` its decimal text. What no column holds - a side's depth, a book's imbalance, midpoint and median - is asked of the native book. The typings are `node/replay.d.ts`.

```typescript
type Pairs = [Json, Json][]                  // a map column, in the native key order
type Rows = { columns: { name: string; dtype: string; nullable: boolean }[]; rows: Row[] }
type PerDepth = { '1': string | null; '5': string | null; '10': string | null }
type LimitJson = { price: string | null; quantity: string; uuids: string[] }   // unpriced last
type SideJson = Row & { limits: LimitJson[]; live: Row[]; deltas: Row[]; depth: PerDepth; length: number }
type BookJson = Row & {                      // the book's marketdata row
  kind: 'book_event'; currunix: string; snapunix: string | null; crosscode: string
  spread: string | null; crossed: boolean | null; locked: boolean | null
  executions: Row[] | null; snapshotpartitions: Row[] | null
  bid: SideJson; ask: SideJson               // the side rows, in place of the two lanes
  imbalance: PerDepth; bboMidpoint: string | null; medianQuantity: string | null
  stableHash: string; isTick: boolean        // isTick: a grid step that changed nothing
}
type LeafJson = Row & { kind: string; curruuid: string; stableHash: string }
type EventJson = LeafJson & { currunix: string; native: string }   // native: the leaf's toJSON text
type Scenario = { name: string; events: EventJson[] }
```

## Pure modules

These hold no DOM and no addon, so Node runs them as they are.

| Module | Exports | Owns |
| --- | --- | --- |
| `instant.js` | `parseInstant`, `instantText`, `instantParts`, `formatInstant`, `formatClock`, `elapsed`, `toPixels`, `fromPixels`, `nanosPerPixel`, `compareInstant` | `bigint` nanosecond instants: a difference is taken first, and only it is scaled to pixels |
| `decimal.js` | `decimalParts`, `isDecimalText`, `formatDecimal`, `compareDecimal`, `maxDecimal`, `minDecimal`, `toFloat`, `sumDecimal` | decimal text, compared exactly; a float only for a pixel |
| `scales.js` | `linear`, `ticks`, `niceStep`, `padded` | linear scales over floats and their round ticks |
| `candles.js` | `bucketOf`, `aggregateCandles` | executions into candles by `lastpx`, per bigint bucket |
| `diff.js` | `sameValue`, `diffFacts`, `diffKeyed`, `diffSides`, `diffBooks`, `diffStreams` | two served books compared by their native facts |
| `scenario.js` | `createScenario`, `eventInstant`, `insertEvent`, `removeEvent`, `earliestAffected`, `serializeScenario`, `parseScenario` | a named list of inserted events in instant order |
| `shortcuts.js` | `SHORTCUTS`, `chordOf`, `commandFor`, `displayChord` | one chord, one command id |
| `store.js` | `createStore` | one frozen state: `get`, `set`, `subscribe`, `select` |

### Instants

Two instants one nanosecond apart stay two instants: an instant is never a `Number`.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')

    async function main() {
      const { compareInstant, formatClock, formatInstant, parseInstant, toPixels } = await import('yggdryl/web/instant.js')

      const first = parseInstant('1700000000001000000')
      const second = parseInstant('1700000000001000001')
      assert.equal(second - first, 1n)
      assert.equal(compareInstant(first, second), -1)
      assert.equal(formatInstant(second), '2023-11-14T22:13:20.001000001Z')
      assert.equal(formatClock(second), '22:13:20.001')
      // Only the difference from the origin is scaled: one millisecond at one
      // microsecond per pixel is a thousand pixels.
      assert.equal(toPixels(first, '1700000000000000000', 1000n), 1000)
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

### Decimals

A decimal is compared digit by digit, past 2^53 included, and displayed without losing a digit it states.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')

    async function main() {
      const { compareDecimal, formatDecimal, maxDecimal, toFloat } = await import('yggdryl/web/decimal.js')

      assert.equal(compareDecimal('82.5', '82.50'), 0)
      assert.equal(compareDecimal('9007199254740993', '9007199254740992'), 1)
      assert.equal(maxDecimal('81.5', '82', '81.75'), '82')
      assert.equal(formatDecimal('82.5', { places: 2 }), '82.50')
      assert.equal(formatDecimal('82.125', { places: 2 }), '82.125')
      // Grouped by a narrow no-break space, which a line never breaks at.
      assert.equal(formatDecimal('1234567.5', { group: true }), '1\u202f234\u202f567.5')
      // A float is read for a pixel position and nothing else.
      assert.equal(toFloat('82.5'), 82.5)
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

### Candles

The price chart folds the executions the books carry into candles: each reads the price it last executed at, `lastpx`, and nothing else.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')

    async function main() {
      const { aggregateCandles } = await import('yggdryl/web/candles.js')

      const executions = [
        { currunix: '1700000000003000000', lastpx: '82.5' },
        { currunix: '1700000000004500000', lastpx: '82.75' },
        { currunix: '1700000000005000000', lastpx: '82.25' },
        // An execution stating no executed price adds no candle.
        { currunix: '1700000000005000001', lastpx: null },
      ]
      const candles = aggregateCandles(executions, { origin: '1700000000000000000', bucketNs: 2_000_000n })
      assert.deepEqual(
        candles.map(({ open, first, high, low, last, count }) => [open, first, high, low, last, count]),
        [
          [1700000000002000000n, '82.5', '82.5', '82.5', '82.5', 1],
          [1700000000004000000n, '82.75', '82.75', '82.25', '82.25', 2],
        ],
      )
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

### Diff

Two served books are the same only when the native `stableHash` agrees and every fact does; a limit is keyed by its price text, a live entry, a delta and an execution by `curruuid`, never by what is drawn.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { booksJson, synthetic } = require('yggdryl/replay')

    async function main() {
      const { diffBooks, diffStreams } = await import('yggdryl/web/diff.js')

      const books = booksJson(synthetic.books()).filter((book) => book.crosscode === 'ALPHA')
      // A stream against itself: every instant the same.
      assert.ok(diffStreams(books, books).every((instant) => instant.changes.same))

      // One limit's quantity moved: named by its price, the fact it changed.
      const [first] = books
      const moved = structuredClone(first)
      moved.bid.limits[0].quantity = '170'
      moved.stableHash = '0'
      const changes = diffBooks(first, moved)
      assert.equal(changes.same, false)
      assert.deepEqual(changes.bid.limits.changed, [
        { key: '82', facts: [{ name: 'quantity', base: '100', scenario: '170' }] },
      ])
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

### Scenarios

A scenario is a named list of events the service validated, each its leaf's row beside `native`, the leaf's own `toJSON` text; the list stays in the order the walk reads its events, a tie keeping the earlier insert first.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { eventJson, leafFromJson } = require('yggdryl/replay')

    async function main() {
      const { createScenario, earliestAffected, insertEvent, parseScenario, removeEvent, serializeScenario } =
        await import('yggdryl/web/scenario.js')

      // An event is what the native constructor built from the body a person typed.
      const event = (currunix, crosscode, side) =>
        eventJson(
          leafFromJson({
            kind: 'order_event',
            currunix,
            facts: { crosscode, ticker: 'ALPHA', side, price: '82.25', quantity: '30', currency: 'USD' },
          }),
        )
      const ask = event('1700000000003500000', 'ALPHA-S-2', 'SELL')
      const bid = event('1700000000002000000', 'ALPHA-S-1', 'BUY')

      let scenario = createScenario('alpha-lock')
      scenario = insertEvent(scenario, ask)
      scenario = insertEvent(scenario, bid)
      assert.deepEqual(scenario.events.map((held) => held.crosscode), ['ALPHA-S-1', 'ALPHA-S-2'])
      // The replay re-runs from the earliest instant the scenario touches.
      assert.equal(earliestAffected(createScenario('alpha-lock'), scenario), 1700000000002000000n)
      assert.deepEqual(parseScenario(serializeScenario(scenario)), scenario)
      assert.equal(removeEvent(scenario, bid.curruuid).events.length, 1)
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

### Keyboard

`commandFor(event, mac)` names the command a key event is: a component reads the command and emits the intent, and the application decides what it does. Typing in a field names none but Escape and the palette, and Space or Enter on a button, a link, a summary or an element with a role of its own names none.

=== "Rust"

    JavaScript only: the components and the replay service live in the Node package.

=== "Python"

    JavaScript only: the components and the replay service live in the Node package.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')

    async function main() {
      const { SHORTCUTS, commandFor, displayChord } = await import('yggdryl/web/shortcuts.js')

      const key = (name, held = {}) => ({ key: name, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, target: null, ...held })
      assert.equal(commandFor(key(' '), false), 'transport.toggle')
      assert.equal(commandFor(key('ArrowRight', { shiftKey: true }), false), 'transport.forwardSource')
      assert.equal(commandFor(key('k', { ctrlKey: true }), false), 'palette.open')
      assert.equal(commandFor(key('k', { metaKey: true }), true), 'palette.open')
      assert.equal(commandFor(key('i', { target: { tagName: 'INPUT' } }), false), undefined)
      assert.equal(commandFor(key('Escape', { target: { tagName: 'INPUT' } }), false), 'ui.close')
      // Space on a button presses the button.
      assert.equal(commandFor(key(' ', { target: { tagName: 'BUTTON', closest: () => ({}) } }), false), undefined)
      assert.equal(displayChord('Mod+k', false), 'Ctrl+k')
      assert.equal(displayChord('Mod+k', true), '⌘k')
      assert.equal(SHORTCUTS.length, 18)
    }

    main().catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
    ```

`Mod` is Ctrl on Windows and Linux and Cmd on macOS; the shortcuts sheet (`?`) lists the same table as the platform spells it.

| Chord | Command | Does |
| --- | --- | --- |
| `Space` | `transport.toggle` | Play or pause |
| `ArrowRight` | `transport.forward` | Step one book forward |
| `ArrowLeft` | `transport.back` | Step one book back |
| `Shift+ArrowRight` | `transport.forwardSource` | Step to the next source instant, skipping grid ticks |
| `Shift+ArrowLeft` | `transport.backSource` | Step to the previous source instant, skipping grid ticks |
| `Home` | `transport.first` | Jump to the first book |
| `End` | `transport.last` | Jump to the last book |
| `]` | `transport.faster` | Speed up |
| `[` | `transport.slower` | Slow down |
| `g` | `transport.grid` | Snapshot grid on or off |
| `j` | `transport.jump` | Jump to an instant |
| `i` | `scenario.insert` | Insert an event |
| `s` | `scenario.drawer` | Open the scenarios |
| `d` | `scenario.diff` | Show the diff against the base replay |
| `t` | `theme.toggle` | Switch the theme |
| `Mod+k` | `palette.open` | Open the command palette |
| `?` | `help.shortcuts` | Show this sheet |
| `Escape` | `ui.close` | Close the open panel |

## Components

Every component extends `Component` (`component.js`): the constructor takes its props, `mount(parent)` builds the root element once and adds the `ygg-ui` class, `update(state)` takes the slice of state it renders and redraws on the next animation frame however often it is called, `destroy()` releases every listener it added. It emits with `emit(name, detail)`, a bubbling `CustomEvent`.

| Module | Class | Constructor | `update(state)` | Emits | Accessibility |
| --- | --- | --- | --- | --- | --- |
| `ladder.js` | `Ladder` | `{ side = 'bid', rows = 40, rowHeight = 24, tooltip? }`, `side` `'bid'` or `'ask'` | `{ side: book.bid, symbol, global }` | `ygg:limit-select` | `role="grid"`, only the rows in view in the DOM with `aria-rowindex`/`aria-rowcount`, the active row by arrows, Page, Home and End; a polite summary of the served best; the side carried by column order and a glyph as well as colour; the unpriced limit last as `∅`; a limit's `uuids` in a tooltip, grouped by ticker on a consolidated book |
| `depth-chart.js` | `DepthChart`, `cumulative` | `{ height = 220 }` | `{ bid: book.bid, ask: book.ask }` | none | a canvas `role="img"` whose label states both sides' depth; the running total is the exact text sum of the served quantities, the unpriced limit counted and not drawn |
| `price-chart.js` | `PriceChart` | `{ bucketNs = 1_000_000_000n, height = 240 }` | `{ books, window: { from, to } }` | `ygg:seek` | the served `bboMidpoint` as a step line, candles from `candles.js`, one marker per execution at its `lastpx` - no last-price line, since a book row states no `lastpx`; the crosshair is a `role="slider"` readout, arrows one bucket, Page ten, Enter seeks |
| `tape.js` | `Tape` | `{ rows = 30, rowHeight = 22 }` | `{ executions }` | `ygg:select-element` | a virtual grid, newest first as served; the clock shows the six digits below the millisecond, the whole instant in `<time datetime>` |
| `lifecycle.js` | `Lifecycle` | `{}` | `{ rows, crosscode }` | `ygg:select-element` | the chain in the served order, `prevuuid` a link to the statement it follows; `metadata` and `securityids` as key-value tables in their served pair order |
| `metrics.js` | `Metrics` | `{}` | `{ book }` | none | the spread, midpoint, median, `crossed`, `locked`, imbalance and depth at 1, 5 and 10 levels as served; only the best bid and ask line is a live region |
| `transport.js` | `Transport`, `SPEEDS` | `{}` | `{ index, count, at, first, last, playing, speed, grid, isTick }` | `ygg:transport` | play is one pressed toggle; an immovable step is `aria-disabled`; the instant is text with every digit; `focusJump()` opens the jump field |
| `symbol-select.js` | `SymbolSelect`, `GLOBAL` | `{ label = 'Symbol' }` | `{ symbols, symbol, global }` | `ygg:symbol` | a labelled `<select>`; the consolidated book named as such |
| `insert-form.js` | `InsertForm` | `{}` | `{ columns, kinds, at }` | `ygg:insert` | the columns `/api/field` served, each added once as a labelled input - a map, serie or struct column typed as JSON and sent as that value, any other as its text; `showRefusal(text, { beside = true })` shows the native refusal verbatim beside the column its path names (`$.<name>`, `$.operation.<name>`, or a path below one), which takes focus so the refusal is read with it, else on the form's `role="alert"` line; an instant typed is kept until `reset()`, which starts the next insert at the instant shown |
| `scenario-drawer.js` | `ScenarioDrawer` | `{ side = 'right', title = 'Scenarios' }` | `{ scenarios, active }` | `ygg:scenario`, `ygg:open`, `ygg:close` | a `Drawer`; a taken or empty name refused before anything is sent; a delete asked twice |
| `diff-view.js` | `DiffView`, `countChanges`, `changeLines` | `{}` | `{ rows: answer.instants }` | `ygg:seek` | one row per served instant, unchanged instants behind "Show unchanged instants", a book only in one stream read as added or removed |
| `modal.js` | `Modal` | `{ title = '' }` | none: `open(opener)`, `close()` | `ygg:close` | the native `<dialog>` by `showModal()`, `aria-labelledby`, Tab kept inside, the page `inert`, Escape and a backdrop click close, the opener refocused |
| `drawer.js` | `Drawer` | `{ side = 'right', title = '' }` | none: `bind(toggle)`, `open(opener)`, `close()`, `toggle(opener)` | `ygg:open`, `ygg:close` | non-modal; toggles carry `aria-expanded` and `aria-controls`; Escape closes and refocuses the toggle |
| `palette.js` | `Palette`, `COMMANDS`, `fuzzy` | `{ commands = COMMANDS, title = 'Commands' }` | none: `open(opener)` | `ygg:command` | a `Modal` holding a combobox over a listbox; substring matches first, then subsequence; the command emitted after focus is back on the opener |
| `shortcuts-sheet.js` | `ShortcutsSheet` | `{ title, shortcuts = SHORTCUTS, mac }` | none: `open(opener)` | `ygg:close` | a `Modal` table of every chord as the platform spells it |
| `tabs.js` | `Tabs` | `{ tabs = [], selected, label = 'Tabs' }` | `{ tabs, selected }` | `ygg:tab` | WAI-ARIA tabs with roving focus and automatic activation; `update` emits nothing |
| `split.js` | `Split` | `{ direction = 'horizontal', sizes = [50, 50], min = 10, step = 2, label = 'panes' }` | `{ sizes }` | `ygg:resize` | `role="separator"` between panes, dragged or moved by the arrows, Home and End |
| `toast.js` | `Toasts` | `{ timeoutMs = 5000 }` | none: `push({ text, kind })`, `dismiss(toast)` | none | two live regions from the start: a refusal is an alert and stays, information is polite and leaves |
| `tooltip.js` | `Tooltip` | `{}` | none: `attach(target, content)`, `show`, `hide` | none | `role="tooltip"`, named by the target's `aria-describedby` while it shows; Escape hides it |
| `theme-toggle.js` | `ThemeToggle`, `STORAGE_KEY` | `{ label = 'Dark theme' }` | none: `toggle()` | `ygg:theme` | pressed while dark; `prefers-color-scheme` decides until a choice sets `data-theme` on `<html>` |

`virtual.js` (`VirtualRows`) and `canvas.js` (`fitCanvas`, `tokens`, `watchSize`, `watchTheme`, `tickLabel`) are what the ladder, the tape and the two charts share.

| Event | Detail | From |
| --- | --- | --- |
| `ygg:limit-select` | `{ price, uuids }` | `Ladder` |
| `ygg:seek` | `{ at }` | `PriceChart`, `DiffView` |
| `ygg:select-element` | `{ crosscode, curruuid }` | `Tape`, `Lifecycle` |
| `ygg:transport` | `{ command, value? }`: a `shortcuts.js` id, or `transport.seek` with an index and `transport.speed` with a number | `Transport` |
| `ygg:open`, `ygg:close` | `{}` | `Modal`, `Drawer` and what extends them |
| `ygg:tab` | `{ id }` | `Tabs` |
| `ygg:resize` | `{ sizes }` | `Split` |
| `ygg:command` | `{ id }` | `Palette` |
| `ygg:symbol` | `{ symbol, global }` | `SymbolSelect` |
| `ygg:theme` | `{ theme }` | `ThemeToggle` |
| `ygg:insert` | `{ kind, currunix, facts }` | `InsertForm` |
| `ygg:scenario` | `{ command, name, curruuid?, to? }`: `create`, `select`, `rename`, `delete` or `remove-event` | `ScenarioDrawer` |

## The application

`yggdryl/web/app/` is the one page composing them, served at `/web/app/`, every file it names relative to itself: `index.html` the shell, `app.js` the composition over one `store.js` state, and `api.js`, whose `createApi(baseUrl)` is the whole contract with the wire - one method per route, the served JSON untouched, a refusal as `{ refusal, status }`. The [Replay](replay.md) page mounts the same composition over recorded answers of that shape. Play holds each book for its instant gap divided by the speed, counted from the frame it is shown in, one book a frame at most: two books one nanosecond apart are two frames, and no book is shown for less than its gap. While the replay plays, the main area and the transport are `aria-busy`, so the regions a book changes are not read book by book; a load is announced as it starts and as it ends; a refusal is told once - by the view panel or the insert form that shows it, else by a toast.

## Design tokens

`theme.css` sets the tokens on `:root`, switched by `data-theme="dark"` or `"light"` on `<html>`, with `prefers-color-scheme` deciding while neither is set. Contrast is the WCAG 2.x relative-luminance ratio `theme.css` states for each pair; every text token clears AA (4.5) on both surfaces of its theme.

| Token | Dark | Light | Contrast, or use |
| --- | --- | --- | --- |
| `--ygg-ui-page` | `#000000` | `#ffffff` | the page |
| `--ygg-ui-surface` | `#0a0a0b` | `#f6f6f7` | a component |
| `--ygg-ui-surface-2` | `#131316` | `#ececef` | a raised row or field |
| `--ygg-ui-text` | `#f2f2f3` | `#0b0b0c` | dark 18.77 on the page; light 19.67 on the page, 16.69 on surface-2 |
| `--ygg-ui-muted` | `#9a9aa3` | `#5f5f68` | dark 7.09 on the surface, 6.65 on surface-2; light 5.85 on the surface, 5.36 on surface-2 |
| `--ygg-ui-bid` | `#ff8a00` | `#ff8a00` | dark text 8.89 on the page, 7.85 on surface-2; light a fill only (2.36 on white) |
| `--ygg-ui-bid-strong` | `#ffa033` | `#9a4d00` | dark 9.73 on the surface, 9.12 on surface-2; light 6.11 on the page, 5.66 on the surface, 5.18 on surface-2 |
| `--ygg-ui-ask` | `#ff5e5b` | `#ff5e5b` | dark text 7.00 on the page, 6.19 on surface-2; light a fill only (3.00 on white) |
| `--ygg-ui-ask-strong` | `#ff9391` | `#a83430` | dark 9.27 on the surface, 8.68 on surface-2; light 6.56 on the page, 6.08 on the surface, 5.57 on surface-2 |
| `--ygg-ui-focus` | `#ff8a00` | `#9a4d00` | the focus ring, 2px |
| `--ygg-ui-bid-soft`, `--ygg-ui-ask-soft` | 16% of the side colour | 18% | a depth bar or a fill behind text |
| `--ygg-ui-line` | white at 8% | black at 8% | a rule, and the hover of a plain button laid over its surface: text reads 14.96 and 13.51 on it dark, 15.24 and 13.98 light |
| `--ygg-ui-on-fill` | `#000000` | `#000000` | text on a fill: 8.89 on the primary fill `#ff8a00` and 10.33 on its hover `--ygg-ui-primary-hover` `#ffa033`; 7.00 on the danger fill `#ff5e5b` and 9.83 on its hover `--ygg-ui-danger-hover` `#ff9391` |
| `--ygg-ui-backdrop` | black at 55% | black at 55% | behind a modal |

The fills and what sits on them are the same in both themes. `--ygg-ui-radius` is 6px, `--ygg-ui-gap` 8px, `--ygg-ui-motion` 120ms and 0ms under `prefers-reduced-motion`; numbers are monospace with tabular figures.

## Edges

- A component renders the served order: the ladder never re-sorts `limits`, the tape never re-sorts `executions`, a map's pairs keep their order - an integer-like key and a `__proto__` key included.
- A decimal is never summed, sorted or re-scaled in the browser but for the depth chart's running total, which is the exact text sum of the served quantities, and a float read only to place a pixel.
- A grid tick - `snapunix` set, no delta, no execution - is `isTick`, and a step to the next source instant skips it.
- An inserted event's refusal is the native constructor's message, verbatim, placed beside the field its path names.
- The scenario list serves every event's `native` text, about a quarter of a megabyte each.
- `node/replay.js` is CommonJS and the components are ES modules: a CommonJS caller loads a component with `import()`.

## Commands

```bash
node --test node/tests/web/<module>.test.js       # one component or module in headless Chromium
node --test node/tests/replay/<file>.test.js      # one service file
node --test node/tests/web/app.test.js            # the application through the service
node node/replay.js synthetic                     # serve it and open the printed URL
```

## Performance

Measured in the ladder test with `performance.now()` in the page, headless Chrome 154, unthrottled, on an AMD Ryzen 5 150 under Windows 11 with Node 24.18; two runs each, no baseline.

| Measured | Runs |
| --- | --- |
| a 1001-limit ladder, from `update` to the frame after its draw | 31.9 ms, 31.5 ms |
| a 1001-limit ladder, from End to the frame after its draw | 22.0 ms, 26.2 ms |

```bash
node --test node/tests/web/ladder.test.js
```
