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

const { bookJson } = require('../../replay/json.js')
const { createReplayServer, drained, streamBooks, WEB_DIR } = require('../../replay/server.js')
const { loadSource } = require('../../replay/sources.js')
const { books, T0 } = require('../../replay/synthetic.js')

const MS = 1_000_000n
const WEB = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-replay-web-'))
const SECRET = path.join(path.dirname(WEB), `${path.basename(WEB)}-secret.txt`)
const FILES = {
  'app/index.html': '<!doctype html><title>replay</title>',
  'app/app.js': 'export const app = 1\n',
  'app/app.css': 'body { margin: 0 }\n',
  'theme.css': ':root { --ygg-ui: 1 }\n',
  'book-timeline.js': 'export class BookTimeline {}\n',
  'data.json': '{"a":1}',
  'blob.bin': 'raw',
}
for (const [name, text] of Object.entries(FILES)) {
  fs.mkdirSync(path.dirname(path.join(WEB, name)), { recursive: true })
  fs.writeFileSync(path.join(WEB, name), text)
}
// A file beside the served folder, which no request may reach.
fs.writeFileSync(SECRET, 'secret')

let service
let base

before(async () => {
  service = createReplayServer({ sources: [loadSource('synthetic')], webDir: WEB })
  base = await service.listen(0)
})

after(async () => {
  await service.close()
  for (const made of [WEB, SECRET]) fs.rmSync(made, { recursive: true, force: true })
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

test('GET /api/sources: the sources, their symbols and GLOBAL once', async () => {
  const { status, json } = await get('/api/sources')
  assert.equal(status, 200)
  assert.deepEqual(json, {
    snapshotMillis: 0,
    global: false,
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

test('static files: the page, the components, their types', async () => {
  const expected = [
    ['/web/app/', 'app/index.html', 'text/html; charset=utf-8'],
    ['/web/app/index.html', 'app/index.html', 'text/html; charset=utf-8'],
    ['/web/app/app.js', 'app/app.js', 'text/javascript; charset=utf-8'],
    ['/web/app/app.css', 'app/app.css', 'text/css; charset=utf-8'],
    ['/web/book-timeline.js', 'book-timeline.js', 'text/javascript; charset=utf-8'],
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
    assert.equal((await fetch(new URL('/api/sources/mine/books?symbol=ALPHA', url))).status, 200)
    const answer = await fetch(new URL('/web/book-timeline.js', url))
    assert.equal(answer.status, 200)
    assert.equal(answer.headers.get('content-type'), 'text/javascript; charset=utf-8')
    assert.equal(await answer.text(), fs.readFileSync(path.join(WEB_DIR, 'book-timeline.js'), 'utf8'))
  } finally {
    await own.close()
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
  // What the replay no longer answers is no route at all.
  for (const gone of ['/api/field', '/api/scenarios', '/api/sources/synthetic/book', '/api/sources/synthetic/view']) {
    assert.equal((await get(gone)).status, 404, gone)
  }
})
