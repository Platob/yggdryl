'use strict'

/**
 * Generate the replay page's manifest from the real package, and copy the
 * component library beside it.
 *
 * The replay service is Node over a native addon, so a documentation page
 * cannot run it. What the page shows is therefore recorded here: this runs
 * `yggdryl/replay` over its two fixed sources - the synthetic scenario, and
 * the ULBridge capture the Rust suite pins, read from its bytes under the
 * committed dictionary and the seed clock - and writes what each route would
 * answer: the operations, every book of the walk per symbol and consolidated,
 * each element's lifecycle, every view - `orders` also with two lifts - and for the
 * synthetic source one scenario of two inserted events with its re-run and its
 * diff. `docs/assets/replay.js` mounts the application over these answers and
 * computes nothing of its own; `calls` holds, beside each answer, the
 * JavaScript and the route that give it.
 *
 * The page's application and components are the package's own files, so they
 * are copied, never rewritten: `docs/assets/web/` holds what the package ships
 * under `web/` - `*.js`, `*.css`, `package.json` and `app/*` - byte for byte,
 * and a copy whose source is gone is drift, removed on regeneration. The
 * application names its modules relative to itself, so the copies resolve
 * where they lie.
 *
 * Everything is committed, so the same build runs on any machine: fixed
 * sources, fixed instants, fixed key order, two-space JSON, LF, bigints as
 * decimal text, no timestamps and no paths. `--check` proves the tree still
 * matches what a regeneration writes, comparing the text rather than the
 * endings a checkout imposed on it, and the capture's line identities by the
 * part the capture decides (`comparable`).
 *
 * Usage:
 *     node scripts/build_docs_replay.js            # regenerate
 *     node scripts/build_docs_replay.js --check    # report drift, write nothing
 */

const fs = require('node:fs')
const path = require('node:path')
const { pathToFileURL } = require('node:url')

const { enums, graph } = require('../node/binding.js')
const {
  booksBetween,
  booksJson,
  createReplayServer,
  eventJson,
  indexBooks,
  leafFromJson,
  leavesJson,
  loadSource,
  refusalText,
  rerun,
  rowsOf,
  synthetic,
  toJson,
  walk,
} = require('../node/replay.js')

const ROOT = path.join(__dirname, '..')
const ASSETS = path.join(ROOT, 'docs', 'assets')
const MANIFEST = path.join(ASSETS, 'replay.json')
const WEB = path.join(ROOT, 'node', 'web')
const COPIES = path.join(ASSETS, 'web')
const VERSION = require('../node/package.json').version

// The capture the Rust suite pins (`rust/tests/fix/ulbridge.rs`): read from its
// bytes under the committed dictionary, an undated line taking the seed clock.
const CAPTURE = 'rust/tests/fix/ulbridge.log'
const REGISTRY = 'config/fix'
const SENDING = '2024-01-02T10:15:30Z'

// The `orders` view's lifts: a map key each, the dotted one a single key.
const LIFTS = Object.freeze(["securityids['ISIN'] as isin", "metadata['tech.clientid'] as clientid"])

// One scenario over the synthetic source: a better ALPHA bid two milliseconds
// in, then an ask at that same price, which locks the book.
const SCENARIO = 'alpha-lock'
const T0 = synthetic.T0
const MS = 1_000_000n
const INSERTED = Object.freeze([
  {
    kind: 'order_event',
    currunix: String(T0 + 2n * MS),
    facts: {
      crosscode: 'ALPHA-S-1',
      ticker: 'ALPHA',
      side: 'BUY',
      price: '82.25',
      quantity: '70',
      currency: 'USD',
      metadata: [['tech.clientid', 'DESK-7']],
    },
  },
  {
    kind: 'order_event',
    currunix: String(T0 + 3n * MS + MS / 2n),
    facts: { crosscode: 'ALPHA-S-2', ticker: 'ALPHA', side: 'SELL', price: '82.25', quantity: '30', currency: 'USD' },
  },
])

/**
 * What the service itself answers at `GET /api/sources` - `{ snapshotMillis,
 * global, views, sources }` - and `GET /api/field`, served over both sources on
 * an ephemeral port and asked once: the routes that answer the whole service
 * rather than one walk.
 */
async function served(sources) {
  const service = createReplayServer({ sources })
  try {
    const url = await service.listen(0)
    const read = async (route) => {
      const answer = await fetch(new URL(route, url))
      if (!answer.ok) throw new Error(`${route}: ${answer.status} ${await answer.text()}`)
      return answer.json()
    }
    return { listing: await read('/api/sources'), field: await read('/api/field') }
  } finally {
    await service.close()
  }
}

/**
 * Every walk a request can name without a grid: the per-symbol walk's books of
 * each symbol, and the consolidated walk's `GLOBAL` books, each list in walk
 * order - what `GET .../books?symbol=` streams.
 */
function booksBySymbol(operations, symbols) {
  const index = indexBooks(walk(operations))
  const books = {}
  for (const symbol of symbols) books[symbol] = booksJson(booksBetween(index, symbol))
  books[graph.GLOBAL_SYMBOL] = booksJson(walk(operations, { global: true }))
  return books
}

/**
 * Each element's lifecycle, as `GET .../lifecycle?crosscode=` answers it: the
 * columns once, the rows per cross code, in the order the operations first
 * name each code.
 */
function lifecycles(operations) {
  const codes = [...new Set(leavesJson(operations).map((row) => row.crosscode))]
  let columns = null
  const rows = {}
  for (const code of codes) {
    const answer = rowsOf(graph.MarketData.applyView('lifecycle', graph.MarketData.arrowReader(operations), [], code))
    if (columns === null) columns = answer.columns
    else if (toJson(columns) !== toJson(answer.columns)) throw new Error(`lifecycle ${code}: the columns changed`)
    rows[code] = answer.rows
  }
  return { columns: columns ?? [], rows }
}

/**
 * Every view as `GET .../view?view=` answers it over the replay's whole stream
 * - the operations, then the books of the walk the request names - bare, and
 * `orders` also with `LIFTS`: `{ columns, rows }`, or the native refusal with
 * the status the route answers it under (`lifecycle` names no chain without a
 * cross code). An answer lists the walks it holds for (`global`): one entry
 * when the per-symbol and the consolidated walk answer alike, as the views
 * that keep no book do, two when they differ.
 */
function views(operations) {
  const answer = (view, lifts, global) => {
    try {
      const reader = graph.MarketData.arrowReader([...operations, ...walk(operations, { global })])
      return rowsOf(graph.MarketData.applyView(view, reader, [...lifts], null))
    } catch (error) {
      return { refusal: refusalText(error), status: 400 }
    }
  }
  const out = {}
  for (const view of enums.marketViews) {
    out[view] = []
    for (const lifts of view === 'orders' ? [[], LIFTS] : [[]]) {
      const [symbol, global] = [false, true].map((held) => answer(view, lifts, held))
      if (toJson(symbol) === toJson(global)) out[view].push({ lifts: [...lifts], global: [false, true], ...symbol })
      else out[view].push({ lifts: [...lifts], global: [false], ...symbol }, { lifts: [...lifts], global: [true], ...global })
    }
  }
  return out
}

/**
 * The scenario, stored as `POST /api/scenarios/:name/events` stores it - each
 * event built by its own constructor, admitted alone by the walk, placed at
 * its instant - then re-run and diffed against the base walk, per symbol and
 * consolidated, as the scenario's `books` and `diff` routes answer.
 */
async function scenarioOf(source, web) {
  let scenario = web.createScenario(SCENARIO)
  const leaves = new Map()
  for (const body of INSERTED) {
    const leaf = leafFromJson(body)
    walk([leaf])
    const event = eventJson(leaf)
    leaves.set(event.curruuid, leaf)
    scenario = web.insertEvent(scenario, event)
  }
  const operations = scenario.events.map((event) => leaves.get(event.curruuid))
  const books = {}
  const diff = {}
  let from = null
  const walks = [...source.symbols.map((symbol) => [symbol, {}]), [graph.GLOBAL_SYMBOL, { global: true }]]
  for (const [symbol, options] of walks) {
    const base = indexBooks(walk(source.operations, options))
    const other = await rerun(source.operations, scenario, options, operations)
    const index = indexBooks(other.books)
    from = other.from === null ? null : String(other.from)
    books[symbol] = booksJson(booksBetween(index, symbol))
    diff[symbol] = {
      symbol,
      from,
      instants: web.diffStreams(booksJson(booksBetween(base, symbol)), booksJson(booksBetween(index, symbol))),
    }
  }
  return {
    scenario: {
      name: scenario.name,
      inserted: INSERTED,
      // The stored events without their `native` text (the leaf's own
      // `toJSON`, a quarter of a megabyte each), which the page never shows.
      events: scenario.events.map(({ native, ...row }) => row),
      from,
      books,
    },
    diff,
  }
}

/** The JavaScript and the route behind each recorded answer. */
function callsOf(id, load, withScenario) {
  const calls = {
    load: { call: load, route: 'GET /api/sources' },
    field: {
      call:
        'const field = graph.MarketData.field(); ' +
        '({ columns: Array.from({ length: field.fieldLen }, (_, at) => field.fieldAt(at))' +
        '.map((child) => ({ name: child.name, dtype: child.dtype.toString(), nullable: child.nullable })), ' +
        "kinds: ['order_event', 'quote_event', 'execution_event', 'order', 'quote', 'execution'] })",
      route: 'GET /api/field',
    },
    operations: { call: 'leavesJson(source.operations)', route: null },
    books: {
      call: 'booksJson(booksBetween(indexBooks(walk(source.operations)), symbol))',
      route: `GET /api/sources/${id}/books?symbol=<symbol>`,
    },
    global: {
      call: 'booksJson(walk(source.operations, { global: true }))',
      route: `GET /api/sources/${id}/books?global=true`,
    },
    lifecycle: {
      call:
        "rowsOf(graph.MarketData.applyView('lifecycle', graph.MarketData.arrowReader(source.operations), [], crosscode))",
      route: `GET /api/sources/${id}/lifecycle?crosscode=<crosscode>`,
    },
    views: {
      call:
        'rowsOf(graph.MarketData.applyView(view, graph.MarketData.arrowReader([...source.operations, ...walk(source.operations, { global })]), ' +
        `lifts, null)), view each of enums.marketViews, lifts [] and for orders also ${JSON.stringify(LIFTS)}`,
      route: `GET /api/sources/${id}/view?view=<view>&global=<global>`,
    },
    orders: {
      call:
        "rowsOf(graph.MarketData.applyView('orders', graph.MarketData.arrowReader([...source.operations, ...walk(source.operations)]), " +
        `${JSON.stringify(LIFTS)}, null))`,
      route: `GET /api/sources/${id}/view?view=orders&${LIFTS.map((lift) => `lift=${encodeURIComponent(lift)}`).join('&')}`,
    },
  }
  if (withScenario) {
    calls.scenario = {
      call:
        `let scenario = createScenario('${SCENARIO}'); ` +
        'for (const body of inserted) { const leaf = leafFromJson(body); walk([leaf]); scenario = insertEvent(scenario, eventJson(leaf)) }; ' +
        'const { books, from } = await rerun(source.operations, scenario, options)',
      route:
        `POST /api/scenarios/${SCENARIO}/events once per inserted body, ` +
        `then GET /api/sources/${id}/scenarios/${SCENARIO}/books?symbol=<symbol>&from=0`,
    }
    calls.diff = {
      call:
        'diffStreams(booksJson(booksBetween(indexBooks(walk(source.operations, options)), symbol)), ' +
        'booksJson(booksBetween(indexBooks(books), symbol)))',
      route: `GET /api/sources/${id}/scenarios/${SCENARIO}/diff?symbol=<symbol>`,
    }
  }
  return calls
}

/** One source's recorded answers. */
async function recorded(source, load, web) {
  const out = {
    operations: leavesJson(source.operations),
    books: booksBySymbol(source.operations, source.symbols),
    lifecycle: lifecycles(source.operations),
    views: views(source.operations),
  }
  if (web !== null) Object.assign(out, await scenarioOf(source, web))
  out.calls = callsOf(source.id, load, web !== null)
  return out
}

/** Build the whole manifest, in the order it is written. */
async function manifest() {
  const web = Object.freeze({
    ...(await import(pathToFileURL(path.join(WEB, 'diff.js')).href)),
    ...(await import(pathToFileURL(path.join(WEB, 'scenario.js')).href)),
  })
  const synthetic = loadSource('synthetic')
  const ulbridge = loadSource(path.join(ROOT, CAPTURE), {
    registry: path.join(ROOT, REGISTRY),
    sendingTime: SENDING,
    id: 'ulbridge',
  })
  const { listing, field } = await served([synthetic, ulbridge])
  return {
    version: VERSION,
    // `GET /api/sources`: the sources, their symbols and the view names.
    listing,
    // `GET /api/field`: the columns an inserted event may state, and its kinds.
    field,
    sources: {
      synthetic: await recorded(synthetic, "const source = loadSource('synthetic')", web),
      ulbridge: await recorded(
        ulbridge,
        `const source = loadSource('${CAPTURE}', { registry: '${REGISTRY}', sendingTime: '${SENDING}', id: 'ulbridge' })`,
        null,
      ),
    },
  }
}

/** What the package ships under `web/`: `*.js`, `*.css`, `package.json` and `app/*`, by path under it. */
function shipped() {
  const files = []
  const list = (folder) => {
    try {
      return fs.readdirSync(folder, { withFileTypes: true }).filter((entry) => entry.isFile())
    } catch (error) {
      if (error.code === 'ENOENT') return []
      throw error
    }
  }
  for (const entry of list(WEB)) {
    if (/\.(js|css)$/.test(entry.name) || entry.name === 'package.json') files.push(entry.name)
  }
  for (const entry of list(path.join(WEB, 'app'))) files.push(`app/${entry.name}`)
  return files.sort()
}

/** Every file under `folder`, as paths relative to it with `/` separators. */
function present(folder, prefix = '') {
  let entries
  try {
    entries = fs.readdirSync(folder, { withFileTypes: true })
  } catch (error) {
    if (error.code === 'ENOENT') return []
    throw error
  }
  return entries.flatMap((entry) =>
    entry.isDirectory()
      ? present(path.join(folder, entry.name), `${prefix}${entry.name}/`)
      : [`${prefix}${entry.name}`],
  )
}

const TEXT = /\.(js|css|html|json|svg|txt|md)$/

/** Report the first line two renderings differ on, so drift names itself. */
function difference(target, current, wanted) {
  if (current === null) return `${path.relative(ROOT, target)} is missing`
  const was = current.split('\n')
  const now = wanted.split('\n')
  for (let line = 0; line < Math.max(was.length, now.length); line += 1) {
    if (was[line] === now[line]) continue
    return (
      `${path.relative(ROOT, target)} is out of date at line ${line + 1}:\n` +
      `  committed: ${(was[line] ?? '<end of file>').slice(0, 160)}\n` +
      `  generated: ${(now[line] ?? '<end of file>').slice(0, 160)}`
    )
  }
  return `${path.relative(ROOT, target)} is out of date`
}

/**
 * The one answer that is not the same on every read: a capture is read into
 * an anonymous buffer the package names `mem://<process>/<address>`, a text
 * line's identity is seeded by that name, and an operation's `srcuuids` are
 * the identities of the lines it was read from. Their millisecond and row
 * sequence (`00000000-0000-7xxx`) are the capture's; the 64 bits after them
 * are the buffer's. A comparison reads the first part and leaves the second,
 * so the committed record is one real read and a new read of the same bytes
 * is not drift.
 */
const BUFFER_SEEDED = /"(00000000-0000-7[0-9a-f]{3})-[0-9a-f]{4}-[0-9a-f]{12}"/g

/** Text as the drift check compares it. */
function comparable(text) {
  return text.replace(BUFFER_SEEDED, '"$1-<buffer>"')
}

/** Write one text file, or report its drift; answers whether it was current. */
function settle(target, wanted, check, compare = (text) => text) {
  // A checkout under `core.autocrlf` materialises the committed LF file as
  // CRLF, which is not drift, so compare the text and keep the file's endings.
  const raw = fs.existsSync(target) ? fs.readFileSync(target, 'utf8') : null
  const current = raw === null ? null : raw.replace(/\r\n/g, '\n')
  if (current !== null && compare(current) === compare(wanted)) return true
  if (check) {
    console.log(`  ${difference(target, current === null ? null : compare(current), compare(wanted))}`)
    return false
  }
  fs.mkdirSync(path.dirname(target), { recursive: true })
  const eol = raw !== null && raw.includes('\r\n') ? '\r\n' : '\n'
  fs.writeFileSync(target, wanted.replace(/\n/g, eol))
  return true
}

/** Copy one file that is not text byte for byte, or report its drift. */
function settleBytes(target, wanted, check) {
  const current = fs.existsSync(target) ? fs.readFileSync(target) : null
  if (current !== null && current.equals(wanted)) return true
  if (check) {
    console.log(`  ${path.relative(ROOT, target)} ${current === null ? 'is missing' : 'is out of date'}`)
    return false
  }
  fs.mkdirSync(path.dirname(target), { recursive: true })
  fs.writeFileSync(target, wanted)
  return true
}

/** Copy the component library and the application; a copy with no source is drift. */
function settleCopies(check) {
  const files = shipped()
  let current = true
  for (const name of files) {
    const source = path.join(WEB, ...name.split('/'))
    const target = path.join(COPIES, ...name.split('/'))
    let settled
    if (TEXT.test(name)) {
      const text = fs.readFileSync(source, 'utf8').replace(/\r\n/g, '\n')
      settled = settle(target, text, check)
    } else {
      settled = settleBytes(target, fs.readFileSync(source), check)
    }
    current = settled && current
  }
  const wanted = new Set(files)
  for (const name of present(COPIES)) {
    if (wanted.has(name)) continue
    const target = path.join(COPIES, ...name.split('/'))
    if (check) {
      console.log(`  ${path.relative(ROOT, target)} has no source in node/web`)
      current = false
    } else {
      fs.rmSync(target)
    }
  }
  return { current, count: files.length }
}

async function main(argv) {
  const check = argv.includes('--check')
  const built = await manifest()
  const { synthetic: made, ulbridge } = built.sources
  const count = (books) => Object.values(books).reduce((total, list) => total + list.length, 0)
  const current = settle(MANIFEST, `${toJson(built, 2)}\n`, check, comparable)
  const copies = settleCopies(check)
  console.log(
    `replay: synthetic ${made.operations.length} operations, ${count(made.books)} books, ` +
      `scenario ${made.scenario.events.length} events; ulbridge ${ulbridge.operations.length} operations, ` +
      `${count(ulbridge.books)} books; ${copies.count} web files${check ? ' checked' : ' generated'}`,
  )
  return current && copies.current ? 0 : 1
}

main(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code
  },
  (error) => {
    console.error(error)
    process.exitCode = 1
  },
)
