'use strict'

// The replay service on an ephemeral port: `node/replay/server.js`.

const assert = require('node:assert/strict')
const { EventEmitter } = require('node:events')
const fs = require('node:fs')
const http = require('node:http')
const os = require('node:os')
const path = require('node:path')
const { after, before, test } = require('node:test')

const { graph } = require('yggdryl')

const { DATED_KINDS, KINDS, MARKETDATA_COLUMNS, bookJson, eventJson, rowsOf } = require('../../replay/json.js')
const { createReplayServer, drained, streamBooks, WEB_DIR } = require('../../replay/server.js')
const { loadSource } = require('../../replay/sources.js')
const { books, synthetic, T0 } = require('../../replay/synthetic.js')

const MS = 1_000_000n
const WEB = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-replay-web-'))
const SECRET = path.join(path.dirname(WEB), `${path.basename(WEB)}-secret.txt`)
const FILES = {
  'app/index.html': '<!doctype html><title>replay</title>',
  'app/app.js': 'export const app = 1\n',
  'app/app.css': 'body { margin: 0 }\n',
  'theme.css': ':root { --ygg-ui: 1 }\n',
  'component.js': 'export class Component {}\n',
  'data.json': '{"a":1}',
  'blob.bin': 'raw',
}
for (const [name, text] of Object.entries(FILES)) {
  fs.mkdirSync(path.dirname(path.join(WEB, name)), { recursive: true })
  fs.writeFileSync(path.join(WEB, name), text)
}
// A file beside the served folder, which no request may reach.
fs.writeFileSync(SECRET, 'secret')

const STATE = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-replay-state-')), 'state')

let service
let base

before(async () => {
  service = createReplayServer({ sources: [loadSource('synthetic')], stateDir: STATE, webDir: WEB })
  base = await service.listen(0)
})

after(async () => {
  await service.close()
  for (const made of [WEB, SECRET, path.dirname(STATE)]) fs.rmSync(made, { recursive: true, force: true })
})

async function get(route, init) {
  const response = await fetch(new URL(route, base), init)
  const text = await response.text()
  const type = response.headers.get('content-type') ?? ''
  return { status: response.status, headers: response.headers, text, json: type.startsWith('application/json') ? JSON.parse(text) : undefined }
}

function send(method, route, body) {
  return get(route, {
    method,
    headers: { 'content-type': 'application/json' },
    body: typeof body === 'string' ? body : JSON.stringify(body),
  })
}

/** A raw request, its path sent exactly as written. */
function raw(method, rawPath) {
  return new Promise((resolve, reject) => {
    const { hostname, port } = new URL(base)
    const request = http.request({ hostname, port, method, path: rawPath }, (response) => {
      const chunks = []
      response.on('data', (chunk) => chunks.push(chunk))
      response.on('end', () =>
        resolve({ status: response.statusCode, headers: response.headers, text: Buffer.concat(chunks).toString('utf8') }),
      )
    })
    request.on('error', reject)
    request.end()
  })
}

/** The events of a server-sent event stream. */
function events(text) {
  return text
    .split('\n\n')
    .filter(Boolean)
    .map((block) => {
      const lines = block.split('\n')
      return {
        event: lines.find((line) => line.startsWith('event: ')).slice('event: '.length),
        data: JSON.parse(lines.find((line) => line.startsWith('data: ')).slice('data: '.length)),
      }
    })
}

async function stream(route) {
  const answer = await get(route)
  assert.equal(answer.status, 200, answer.text)
  assert.equal(answer.headers.get('content-type'), 'text/event-stream; charset=utf-8')
  assert.equal(answer.headers.get('cache-control'), 'no-store')
  const all = events(answer.text)
  const end = all.at(-1)
  assert.equal(end.event, 'end')
  const booksServed = all.slice(0, -1)
  assert.ok(booksServed.every((item) => item.event === 'book'))
  assert.equal(end.data.count, booksServed.length)
  return { books: booksServed.map((item) => item.data), end: end.data }
}

const hashes = (items) => items.map((item) => String(item.stableHash()))

function order(at, code, price) {
  return { kind: 'order_event', currunix: String(at), facts: { crosscode: code, ticker: 'ALPHA', side: 'BUY', price, quantity: '5' } }
}

test('GET /api/field: the marketdata columns and the kinds the insert form reads', async () => {
  const { status, json } = await get('/api/field')
  assert.equal(status, 200)
  assert.deepEqual(json, {
    columns: MARKETDATA_COLUMNS,
    kinds: ['order_event', 'quote_event', 'execution_event'],
  })
  // The kinds are the dated ones an inserted event may name: admission is the walk, which reads events.
  assert.deepEqual(json.kinds, DATED_KINDS)
  assert.ok(json.kinds.every((kind) => kind in KINDS))
})

test('GET /api/sources: the sources, their symbols and GLOBAL once', async () => {
  const { status, json } = await get('/api/sources')
  assert.equal(status, 200)
  assert.deepEqual(json, {
    snapshotMillis: 0,
    global: false,
    views: ['orders', 'quotes', 'executions', 'trades', 'book_sides', 'books', 'lifecycle'],
    sources: [
      { id: 'synthetic', kind: 'synthetic', name: 'synthetic', operations: 23, symbols: ['ALPHA', 'BETA', 'GLOBAL'], refusals: [] },
    ],
  })
})

test('the books stream is the walk, in walk order', async () => {
  const served = await stream('/api/sources/synthetic/books')
  assert.deepEqual(served.books.map((book) => book.stableHash), hashes(books()))
  assert.deepEqual(served.books[0], bookJson(books()[0]))

  // A symbol and inclusive bounds: two books one nanosecond apart.
  const window = await stream(`/api/sources/synthetic/books?symbol=ALPHA&from=${T0 + MS}&to=${T0 + MS + 1n}`)
  assert.deepEqual(window.books.map((book) => book.currunix), [String(T0 + MS), String(T0 + MS + 1n)])

  // The grid and the consolidated walk a request names.
  const grid = await stream('/api/sources/synthetic/books?snapshotMillis=2')
  assert.deepEqual(grid.books.map((book) => book.stableHash), hashes(books(2)))
  assert.equal(grid.books.filter((book) => book.isTick).length, 3)
  const global = await stream('/api/sources/synthetic/books?global=true')
  assert.deepEqual(global.books.map((book) => book.stableHash), hashes(books(0, true)))
  assert.ok(global.books.every((book) => book.crosscode === graph.GLOBAL_SYMBOL))
})

test('the books stream refuses what it cannot read', async () => {
  const unknown = await get('/api/sources/synthetic/books?symbol=GAMMA')
  assert.equal(unknown.status, 404)
  assert.equal(unknown.json.error, 'no symbol "GAMMA" in this walk; its symbols are ALPHA, BETA')
  const instant = await get('/api/sources/synthetic/books?from=yesterday')
  assert.equal(instant.status, 400)
  assert.equal(instant.json.error, 'from: expected an instant as the decimal text of nanoseconds, got "yesterday"')
  const grid = await get('/api/sources/synthetic/books?snapshotMillis=-1')
  assert.equal(grid.status, 400)
  const flag = await get('/api/sources/synthetic/books?global=maybe')
  assert.equal(flag.json.error, 'global: expected true or false, got "maybe"')
  const source = await get('/api/sources/nope/books')
  assert.equal(source.status, 404)
  assert.equal(source.json.error, 'no source "nope"; the sources are synthetic')
})

test('GET book: the book standing at an instant', async () => {
  const walked = books()
  const standing = await get(`/api/sources/synthetic/book?symbol=ALPHA&at=${T0 + 2n * MS}`)
  assert.equal(standing.status, 200)
  const expected = walked.find((book) => book.crosscode === 'ALPHA' && book.currunix === T0 + MS + 1n)
  assert.deepEqual(standing.json, bookJson(expected))
  const last = await get('/api/sources/synthetic/book?symbol=BETA')
  assert.equal(last.json.stableHash, String(walked.at(-1).stableHash()))
  const global = await get('/api/sources/synthetic/book?global=1')
  assert.equal(global.json.crosscode, graph.GLOBAL_SYMBOL)
  const early = await get(`/api/sources/synthetic/book?symbol=ALPHA&at=${T0 - 1n}`)
  assert.equal(early.status, 404)
  const unnamed = await get('/api/sources/synthetic/book')
  assert.equal(unnamed.status, 400)
  assert.equal(unnamed.json.error, 'symbol: expected one of ALPHA, BETA')
})

test('GET lifecycle: the native view of one chain', async () => {
  const { status, json } = await get('/api/sources/synthetic/lifecycle?crosscode=BETA-O-1')
  assert.equal(status, 200)
  const expected = rowsOf(
    graph.MarketData.applyView('lifecycle', graph.MarketData.arrowReader(synthetic()), [], 'BETA-O-1'),
  )
  assert.deepEqual(json, expected)
  assert.deepEqual(json.rows.map((row) => row.state), ['20NEW', '95EXPIRED'])
  const refused = await get('/api/sources/synthetic/lifecycle')
  assert.equal(refused.status, 400)
  assert.match(refused.json.error, /\$\.crosscode: expected the crosscode of the chain a lifecycle view follows, got none/)
})

test('GET view: a named view with its lifts, and the native refusals', async () => {
  const lift = "securityids['ISIN'] as isin"
  const { status, json } = await get(`/api/sources/synthetic/view?view=orders&lift=${encodeURIComponent(lift)}`)
  assert.equal(status, 200)
  const expected = rowsOf(graph.MarketData.applyView('orders', graph.MarketData.arrowReader(synthetic()), [lift]))
  assert.deepEqual(json, expected)
  assert.equal(json.columns.at(-1).name, 'isin')
  assert.equal(json.rows.length, 5)
  assert.ok(json.rows.every((row) => row.isin === null))
  // The stream a view reads is the operations, then the books of the walk
  // the request names: the book views have rows too.
  const walked = await get('/api/sources/synthetic/view?view=books&snapshotMillis=2')
  assert.deepEqual(walked.json.rows.map((row) => row.currhashcode), books(2).map((book) => String(book.currhashcode)))
  const sides = await get('/api/sources/synthetic/view?view=book_sides')
  assert.equal(sides.json.rows.length, 2 * books().length)

  const refusal = (build) => {
    try {
      build()
    } catch (error) {
      return error.message
    }
    assert.fail('expected a native refusal')
  }
  const badLift = await get(`/api/sources/synthetic/view?view=orders&lift=${encodeURIComponent("nosuch['X'] as y")}`)
  assert.equal(badLift.status, 400)
  assert.equal(
    badLift.json.error,
    refusal(() => graph.MarketData.applyView('orders', graph.MarketData.arrowReader(synthetic()), ["nosuch['X'] as y"])),
  )
  const badView = await get('/api/sources/synthetic/view?view=nope')
  assert.equal(badView.status, 400)
  assert.equal(
    badView.json.error,
    'invalid record value at $.view: expected one of orders, quotes, executions, trades, book_sides, books, lifecycle, got "nope"',
  )
})

test('scenarios: create, read, list and delete', async () => {
  assert.deepEqual((await get('/api/scenarios')).json, { scenarios: [] })
  const created = await send('PUT', '/api/scenarios/empty', {})
  assert.equal(created.status, 200)
  assert.deepEqual(created.json, { name: 'empty', events: [] })
  assert.deepEqual((await get('/api/scenarios/empty')).json, { name: 'empty', events: [] })
  assert.deepEqual((await get('/api/scenarios')).json, { scenarios: [{ name: 'empty', events: [] }] })
  const renamed = await send('PUT', '/api/scenarios/empty', { name: 'other' })
  assert.equal(renamed.status, 400)
  assert.equal((await send('DELETE', '/api/scenarios/empty')).status, 204)
  assert.equal((await send('DELETE', '/api/scenarios/empty')).status, 404)
  assert.equal((await get('/api/scenarios/empty')).status, 404)
  const traversal = await get('/api/scenarios/..%2Fescape')
  assert.equal(traversal.status, 400)
  assert.match(traversal.json.error, /expected a scenario name/)
  const upper = await send('PUT', '/api/scenarios/Empty', {})
  assert.equal(upper.status, 400)
  assert.match(upper.json.error, /lowercase letters, digits.*got "Empty"/)
})

test('inserting an event: the native leaf\'s JSON, or its refusal verbatim', async () => {
  const inserted = await send('POST', '/api/scenarios/insert/events', order(T0 + 3n * MS + 500_000n, 'ALPHA-X-1', '82.25'))
  assert.equal(inserted.status, 201, inserted.text)
  assert.equal(inserted.json.kind, 'order_event')
  assert.equal(inserted.json.price, '82.25')
  assert.equal(inserted.json.currunix, String(T0 + 3n * MS + 500_000n))
  assert.ok(graph.MarketData.fromJSON(inserted.json.native).equals(new graph.MarketData(
    new graph.OrderEvent(T0 + 3n * MS + 500_000n, order(0n, 'ALPHA-X-1', '82.25').facts),
  )))
  const held = (await get('/api/scenarios/insert')).json
  assert.deepEqual(held.events.map((event) => event.curruuid), [inserted.json.curruuid])
  // The list serves every scenario whole, its events as the scenario itself serves them.
  const listed = (await get('/api/scenarios')).json.scenarios
  assert.deepEqual(listed.find((scenario) => scenario.name === 'insert'), held)
  assert.deepEqual(held.events, [inserted.json])

  // The constructor's refusal, verbatim.
  let expected
  try {
    new graph.OrderEvent(T0, { crosscode: 'N', price: 'abc' })
  } catch (error) {
    expected = error.message
  }
  const refused = await send('POST', '/api/scenarios/insert/events', { kind: 'order_event', currunix: String(T0), facts: { crosscode: 'N', price: 'abc' } })
  assert.equal(refused.status, 400)
  assert.equal(refused.json.error, expected)
  // An undated leaf is refused by the walk, by name.
  const undated = await send('POST', '/api/scenarios/insert/events', { kind: 'order', facts: { crosscode: 'U' } })
  assert.equal(undated.status, 400)
  assert.match(undated.json.error, /\$\.operation\.kind: expected order_event, quote_event, execution_event, trade_event or snapshot_event, got order/)
  // The same event twice is refused.
  const again = await send('POST', '/api/scenarios/insert/events', order(T0 + 3n * MS + 500_000n, 'ALPHA-X-1', '82.25'))
  assert.equal(again.status, 400)
  assert.match(again.json.error, /is already in scenario insert/)
  // Not JSON.
  const garbled = await send('POST', '/api/scenarios/insert/events', '{')
  assert.equal(garbled.status, 400)
  assert.match(garbled.json.error, /the request body is not JSON/)

  // Removing it restores every base book exactly, through the service; removing it again is refused.
  const changed = await stream('/api/sources/synthetic/scenarios/insert/books?from=0')
  assert.equal(changed.books.length, books().length + 1)
  assert.equal((await send('DELETE', `/api/scenarios/insert/events/${inserted.json.curruuid}`)).status, 204)
  assert.deepEqual((await get('/api/scenarios/insert')).json.events, [])
  const restored = await stream('/api/sources/synthetic/scenarios/insert/books?from=0')
  assert.deepEqual(restored.books.map((book) => book.stableHash), hashes(books()))
  assert.equal(restored.end.from, null)
  const gone = await send('DELETE', `/api/scenarios/insert/events/${inserted.json.curruuid}`)
  assert.equal(gone.status, 404)
})

test('PUT refuses what POST refuses: each event is admitted alone by the walk', async () => {
  const posted = await send('POST', '/api/scenarios/admitted/events', { kind: 'order', facts: { crosscode: 'U' } })
  assert.equal(posted.status, 400)
  const put = await send('PUT', '/api/scenarios/admitted', { events: [eventJson(new graph.Order({ crosscode: 'U' }))] })
  assert.equal(put.status, 400)
  assert.equal(put.json.error, posted.json.error)
  assert.equal((await get('/api/scenarios/admitted')).status, 404)

  // Two events admitted one by one - the second read at a grid step later
  // than the first's clock - re-run together, and a PUT stores them alike.
  const plain = await send('POST', '/api/scenarios/stepped/events', order(T0 + 3n * MS + 500_000n, 'ALPHA-X-1', '82.25'))
  assert.equal(plain.status, 201, plain.text)
  const stepped = order(T0 + 3n * MS, 'ALPHA-X-2', '82.1')
  stepped.facts.snapunix = String(T0 + 4n * MS)
  const steppedAnswer = await send('POST', '/api/scenarios/stepped/events', stepped)
  assert.equal(steppedAnswer.status, 201, steppedAnswer.text)
  const held = (await get('/api/scenarios/stepped')).json
  assert.deepEqual(held.events.map((event) => event.crosscode), ['ALPHA-X-1', 'ALPHA-X-2'])
  const replayed = await stream('/api/sources/synthetic/scenarios/stepped/books?from=0')
  assert.equal(replayed.books.length, books().length + 2)
  assert.equal((await get('/api/sources/synthetic/scenarios/stepped/diff?symbol=ALPHA')).status, 200)
  const again = await send('PUT', '/api/scenarios/stepped-again', { events: [...held.events].reverse() })
  assert.equal(again.status, 200, again.text)
  assert.deepEqual(again.json.events, held.events)
})

test('a POST renders its own event alone: the held events are neither decoded nor re-rendered', async () => {
  for (let at = 0n; at < 3n; at += 1n) {
    const posted = await send('POST', '/api/scenarios/grown/events', order(T0 + at * MS, `ALPHA-G-${at}`, '82.25'))
    assert.equal(posted.status, 201, posted.text)
  }
  // What is held, edited by hand: a fact, and a native text that is no leaf.
  const file = path.join(STATE, 'grown.json')
  const held = JSON.parse(fs.readFileSync(file, 'utf8'))
  held.events[0].price = '1'
  held.events[1].native = 'bm90IGFycm93'
  fs.writeFileSync(file, JSON.stringify(held))
  // Every native encoding the service runs, counted.
  const encoded = []
  const restore = []
  for (const Leaf of Object.values(graph)) {
    if (typeof Leaf !== 'function' || !Object.hasOwn(Leaf.prototype ?? {}, 'toJSON')) continue
    const { toJSON } = Leaf.prototype
    Leaf.prototype.toJSON = function counted() {
      encoded.push(Leaf.name)
      return toJSON.call(this)
    }
    restore.push(() => {
      Leaf.prototype.toJSON = toJSON
    })
  }
  let fourth
  try {
    fourth = await send('POST', '/api/scenarios/grown/events', order(T0 + 3n * MS, 'ALPHA-G-3', '82.25'))
  } finally {
    for (const undo of restore) undo()
  }
  assert.ok(restore.length > 0)
  assert.equal(fourth.status, 201, fourth.text)
  assert.deepEqual(encoded, ['OrderEvent'])
  const after = JSON.parse(fs.readFileSync(file, 'utf8'))
  assert.deepEqual(after.events.slice(0, 3), held.events)
  assert.deepEqual(after.events[3], fourth.json)
  // A read that decodes finds the text that is no leaf: the file is the service's fault.
  assert.equal((await get('/api/scenarios/grown')).status, 500)
})

test("a scenario file that cannot be read back is the service's fault: 500 with its message", async () => {
  fs.mkdirSync(STATE, { recursive: true })
  fs.writeFileSync(path.join(STATE, 'corrupt.json'), '{')
  let expected
  try {
    JSON.parse('{')
  } catch (error) {
    expected = error.message
  }
  for (const [method, route, body] of [
    ['GET', '/api/scenarios/corrupt'],
    ['GET', '/api/sources/synthetic/scenarios/corrupt/books'],
    ['GET', '/api/sources/synthetic/scenarios/corrupt/diff?symbol=ALPHA'],
    ['POST', '/api/scenarios/corrupt/events', order(T0, 'ALPHA-C-1', '82')],
    ['DELETE', '/api/scenarios/corrupt/events/00000000-0000-0000-0000-000000000000'],
  ]) {
    const answer = body === undefined ? await get(route, { method }) : await send(method, route, body)
    assert.equal(answer.status, 500, route)
    assert.equal(answer.json.error, expected, route)
  }
  // The list reads every file: it names the one it cannot read back.
  const listed = await get('/api/scenarios')
  assert.equal(listed.status, 500)
  assert.equal(listed.json.error, `scenario "corrupt": ${expected}`)
  // A file holding another scenario's name is refused by name.
  fs.writeFileSync(path.join(STATE, 'misnamed.json'), JSON.stringify({ name: 'other', events: [] }))
  const misnamed = await get('/api/scenarios/misnamed')
  assert.equal(misnamed.status, 500)
  assert.equal(misnamed.json.error, 'scenario file misnamed.json holds the scenario "other"')
  // A name that names no file stays the request's fault.
  assert.equal((await get('/api/scenarios/..%2Fcorrupt')).status, 400)
  assert.equal((await send('DELETE', '/api/scenarios/corrupt')).status, 204)
  assert.equal((await send('DELETE', '/api/scenarios/misnamed')).status, 204)
  assert.equal((await get('/api/scenarios')).status, 200)
})

test('an instant written as a JSON number past 2^53 arrives exact', async () => {
  const at = T0 + 3n * MS + 1n
  const body = `{"kind":"order_event","currunix":${at},"facts":{"crosscode":"EXACT","ticker":"ALPHA","side":"BUY","price":"82","quantity":"1","exprtime":${at + 7n}}}`
  const inserted = await send('POST', '/api/scenarios/exact/events', body)
  assert.equal(inserted.status, 201, inserted.text)
  assert.equal(inserted.json.currunix, String(at))
  assert.equal(inserted.json.exprtime, String(at + 7n))
})

test('a scenario re-runs from its earliest affected instant, and its diff names the change', async () => {
  // An empty scenario changes nothing: the diff is all `same`.
  await send('PUT', '/api/scenarios/unchanged', {})
  const same = await get('/api/sources/synthetic/scenarios/unchanged/diff?symbol=ALPHA')
  assert.equal(same.status, 200)
  assert.equal(same.json.from, null)
  assert.equal(same.json.instants.length, books().filter((book) => book.crosscode === 'ALPHA').length)
  assert.ok(same.json.instants.every((row) => row.changes.same && row.base === row.scenario))
  const whole = await stream('/api/sources/synthetic/scenarios/unchanged/books')
  assert.deepEqual(whole.books.map((book) => book.stableHash), hashes(books()))
  assert.equal(whole.end.from, null)

  const at = T0 + 3n * MS + 500_000n
  await send('POST', '/api/scenarios/changed/events', order(at, 'ALPHA-X-1', '82.25'))
  const rerun = await stream('/api/sources/synthetic/scenarios/changed/books?symbol=ALPHA')
  assert.equal(rerun.end.from, String(at))
  assert.ok(rerun.books.length > 0 && rerun.books.every((book) => BigInt(book.currunix) >= at))
  assert.ok(rerun.books.every((book) => book.bid.limits.some((limit) => limit.price === '82.25')))
  const everything = await stream('/api/sources/synthetic/scenarios/changed/books?from=0')
  assert.equal(everything.books.length, books().length + 1)

  const diff = await get('/api/sources/synthetic/scenarios/changed/diff?symbol=ALPHA')
  assert.equal(diff.json.from, String(at))
  const changed = diff.json.instants.filter((row) => !row.changes.same)
  assert.ok(changed.length > 0)
  assert.ok(changed.every((row) => BigInt(row.at) >= at))
  // At its own instant the scenario has a book the base has not; after it,
  // each ALPHA book differs by the one limit the event added.
  const added = changed.find((row) => row.at === String(at))
  assert.equal(added.base, null)
  assert.notEqual(added.scenario, null)
  const later = changed.filter((row) => BigInt(row.at) > at)
  assert.ok(later.length > 0)
  for (const row of later) {
    assert.deepEqual(row.changes.bid.limits.added.map((limit) => limit.key), ['82.25'])
    assert.deepEqual(row.changes.bid.limits.removed, [])
    assert.deepEqual(row.changes.bid.live.added.map((entry) => entry.crosscode), ['ALPHA-X-1'])
  }
  assert.equal((await get('/api/sources/synthetic/scenarios/missing/diff?symbol=ALPHA')).status, 404)
  assert.equal((await get('/api/sources/synthetic/scenarios/changed/diff')).status, 400)
})

test('a symbol only the scenario introduces streams and diffs, every instant added', async () => {
  const at = T0 + 3n * MS
  const gamma = { kind: 'order_event', currunix: String(at), facts: { crosscode: 'GAMMA-O-1', ticker: 'GAMMA', side: 'BUY', price: '10', quantity: '5' } }
  assert.equal((await send('POST', '/api/scenarios/gamma/events', gamma)).status, 201)
  const served = await stream('/api/sources/synthetic/scenarios/gamma/books?symbol=GAMMA')
  assert.deepEqual(served.books.map((book) => [book.crosscode, book.currunix]), [['GAMMA', String(at)]])
  const diff = await get('/api/sources/synthetic/scenarios/gamma/diff?symbol=GAMMA')
  assert.equal(diff.status, 200, diff.text)
  assert.equal(diff.json.symbol, 'GAMMA')
  assert.deepEqual(diff.json.instants.map((row) => [row.at, row.base, row.scenario]), [[String(at), null, served.books[0].stableHash]])
  assert.ok(diff.json.instants.every((row) => !row.changes.same))
  // A symbol in neither walk is named with every symbol either walk has.
  const unknown = await get('/api/sources/synthetic/scenarios/gamma/diff?symbol=DELTA')
  assert.equal(unknown.status, 404)
  assert.equal(unknown.json.error, 'no symbol "DELTA" in the base or the scenario walk; their symbols are ALPHA, BETA, GAMMA')
  const unnamed = await get('/api/sources/synthetic/scenarios/gamma/diff')
  assert.equal(unnamed.json.error, 'symbol: expected one of ALPHA, BETA, GAMMA')
})

test('static files: the page, the components, their types', async () => {
  const expected = [
    ['/web/app/', 'app/index.html', 'text/html; charset=utf-8'],
    ['/web/app/index.html', 'app/index.html', 'text/html; charset=utf-8'],
    ['/web/app/app.js', 'app/app.js', 'text/javascript; charset=utf-8'],
    ['/web/app/app.css', 'app/app.css', 'text/css; charset=utf-8'],
    ['/web/component.js', 'component.js', 'text/javascript; charset=utf-8'],
    ['/web/theme.css', 'theme.css', 'text/css; charset=utf-8'],
    ['/web/data.json', 'data.json', 'application/json; charset=utf-8'],
    ['/web/blob.bin', 'blob.bin', 'application/octet-stream'],
  ]
  for (const [route, file, type] of expected) {
    const answer = await get(route)
    assert.equal(answer.status, 200, route)
    assert.equal(answer.headers.get('content-type'), type, route)
    assert.equal(answer.headers.get('x-content-type-options'), 'nosniff', route)
    assert.equal(answer.text, FILES[file], route)
  }
  assert.equal((await get('/web/missing.js')).status, 404)
  assert.equal((await get('/web/')).status, 404)
  assert.equal((await get('/app/app.js')).status, 404)
  assert.equal((await get('/elsewhere.js')).status, 404)
  const head = await get('/web/theme.css', { method: 'HEAD' })
  assert.equal(head.status, 200)
  assert.equal(head.headers.get('content-length'), String(FILES['theme.css'].length))
  assert.equal((await send('POST', '/web/theme.css', {})).status, 405)
})

test('/, /app, /app/ and /web/app are redirected to /web/app/, the folder the page is served from', async () => {
  for (const [rawPath, location] of [
    ['/', '/web/app/'],
    ['/?debug=1', '/web/app/?debug=1'],
    ['/app', '/web/app/'],
    ['/app?symbol=ALPHA', '/web/app/?symbol=ALPHA'],
    ['/app/', '/web/app/'],
    ['/web/app', '/web/app/'],
  ]) {
    const answer = await raw('GET', rawPath)
    assert.equal(answer.status, 308, rawPath)
    assert.equal(answer.headers.location, location, rawPath)
  }
  assert.equal((await raw('HEAD', '/app')).status, 308)
  for (const route of ['/', '/app']) {
    const followed = await get(route)
    assert.equal(followed.status, 200, route)
    assert.equal(followed.text, FILES['app/index.html'], route)
  }
  assert.equal((await send('POST', '/app', {})).status, 405)
})

test('static files: a path leaving the served folder is refused', async () => {
  const secret = `${path.basename(WEB)}-secret.txt`
  for (const rawPath of [
    `/web/../${secret}`,
    `/web/..%2F${secret}`,
    `/web/%2e%2e/${secret}`,
    `/web/..%5C${secret}`,
    `/web/app/../../${secret}`,
    '/web/%00theme.css',
  ]) {
    const answer = await raw('GET', rawPath)
    assert.equal(answer.status, 403, rawPath)
    assert.ok(!answer.text.includes('secret"') && answer.text !== 'secret', rawPath)
  }
})

test('the default web folder is the package\'s own components; a Map names its sources', async () => {
  assert.equal(WEB_DIR, path.join(__dirname, '..', '..', 'web'))
  const own = createReplayServer({ sources: new Map([['mine', loadSource('synthetic')]]) })
  const url = await own.listen(0)
  try {
    const listed = await (await fetch(new URL('/api/sources', url))).json()
    assert.deepEqual(listed.sources.map((source) => source.id), ['mine'])
    assert.equal((await fetch(new URL('/api/sources/mine/book?symbol=ALPHA', url))).status, 200)
    const answer = await fetch(new URL('/web/diff.js', url))
    assert.equal(answer.status, 200)
    assert.equal(answer.headers.get('content-type'), 'text/javascript; charset=utf-8')
    assert.equal(await answer.text(), fs.readFileSync(path.join(WEB_DIR, 'diff.js'), 'utf8'))
  } finally {
    await own.close()
  }
})

test('close removes the state folder the service made itself, never one it was given', async () => {
  const own = createReplayServer({ sources: [loadSource('synthetic')], webDir: WEB })
  assert.equal(path.dirname(own.stateDir), os.tmpdir())
  assert.match(path.basename(own.stateDir), /^yggdryl-replay-[0-9a-f-]{36}$/)
  const url = await own.listen(0)
  assert.equal((await fetch(new URL('/api/scenarios/kept', url), { method: 'PUT', body: '{}' })).status, 200)
  assert.ok(fs.existsSync(path.join(own.stateDir, 'kept.json')))
  await own.close()
  assert.equal(fs.existsSync(own.stateDir), false)

  const given = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-replay-given-'))
  try {
    const theirs = createReplayServer({ sources: [loadSource('synthetic')], stateDir: given, webDir: WEB })
    assert.equal(theirs.stateDir, given)
    const theirsUrl = await theirs.listen(0)
    assert.equal((await fetch(new URL('/api/scenarios/kept', theirsUrl), { method: 'PUT', body: '{}' })).status, 200)
    await theirs.close()
    assert.ok(fs.existsSync(path.join(given, 'kept.json')))
  } finally {
    fs.rmSync(given, { recursive: true, force: true })
  }
})

test('a stream whose client left before it began ends at once, holding no walk', async () => {
  let writes = 0
  const left = Object.assign(new EventEmitter(), {
    destroyed: true,
    writeHead() {},
    write() {
      writes += 1
      return false
    },
    end() {},
  })
  /** `promise`, or a refusal once `ms` pass: a wait on a client that left never ends by itself. */
  function within(promise, ms) {
    let timer
    const late = new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('waited on a client that left')), ms)
    })
    return Promise.race([promise, late]).finally(() => clearTimeout(timer))
  }
  await within(drained(left), 1000)
  await within(streamBooks(left, books()), 5000)
  assert.equal(writes, 0)
})

test('routes: unknown is 404, a wrong method 405 naming the allowed', async () => {
  const unknown = await get('/api/nothing')
  assert.equal(unknown.status, 404)
  assert.equal(unknown.json.error, 'no route GET /api/nothing')
  const wrong = await send('POST', '/api/sources', {})
  assert.equal(wrong.status, 405)
  assert.equal(wrong.headers.get('allow'), 'GET')
  const scenario = await send('PATCH', '/api/scenarios/x', {})
  assert.equal(scenario.headers.get('allow'), 'GET, PUT, DELETE')
})
