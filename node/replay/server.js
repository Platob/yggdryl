'use strict'

// The replay service: Node's `http` over the native package. Every answer is
// the native package's - the walk's books, a view's rows, a constructor's
// refusal - rendered by `json.js`; the service routes, holds the walks it
// served, keeps the scenarios as files, streams books as server-sent events
// and serves the page. Instants cross as the decimal text of nanoseconds,
// decimals as their exact text, and a refusal as the native message, verbatim.

const { randomUUID } = require('node:crypto')
const fs = require('node:fs')
const http = require('node:http')
const os = require('node:os')
const path = require('node:path')

const { enums, graph } = require('../binding.js')

const { DATED_KINDS, INSTANT_COLUMNS, KINDS, MARKETDATA_COLUMNS, booksJson, bookJson, leafFromJson, refusalText, rowsOf, toJson } =
  require('./json.js')
const { checkName, openScenarios } = require('./scenarios.js')
const { bookAt, booksBetween, indexBooks, rerun, walk } = require('./walk.js')
const web = require('./web.js')

/** The component library and the page, beside this package. */
const WEB_DIR = path.join(__dirname, '..', 'web')
/** The largest request body read: a scenario of a few hundred events, each ~250 KiB of native text. */
const BODY_LIMIT = 64 * 1024 * 1024
/** Books rendered through one Arrow stream while an event stream is written. */
const SSE_CHUNK = 256
/** Walks held per source, one per grid setting asked for; the oldest is dropped past it. */
const WALKS_PER_SOURCE = 4

const CONTENT_TYPES = Object.freeze({
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.map': 'application/json; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
})

/** Where the page is served: its folder under `/web/`. */
const PAGE = '/web/app/'

/** The paths redirected to the page, as their segments joined: `/`, `/app`, `/app/`, `/web/app`. */
const PAGE_ALIASES = new Set(['', 'app', 'app/', 'web/app'])

/** A refusal with its HTTP status. */
class HttpError extends Error {
  constructor(status, message, headers = {}) {
    super(message)
    this.status = status
    this.headers = headers
  }
}

// ---- request intake ------------------------------------------------------

/** The path's segments, each percent-decoded once. */
function segmentsOf(rawPath) {
  return rawPath
    .split('/')
    .slice(1)
    .map((part) => {
      try {
        return decodeURIComponent(part)
      } catch {
        throw new HttpError(400, `malformed path segment ${JSON.stringify(part)}`)
      }
    })
}

/** An instant parameter: decimal nanoseconds, or `undefined` when absent. */
function instantParam(params, name) {
  const value = params.get(name)
  if (value === null || value === '') return undefined
  if (!/^-?\d+$/.test(value)) {
    throw new HttpError(400, `${name}: expected an instant as the decimal text of nanoseconds, got ${JSON.stringify(value)}`)
  }
  return BigInt(value)
}

/** A grid width in whole milliseconds, `fallback` when absent. */
function millisParam(params, name, fallback) {
  const value = params.get(name)
  if (value === null || value === '') return fallback
  const millis = Number(value)
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(millis)) {
    throw new HttpError(400, `${name}: expected a non-negative whole number of milliseconds, got ${JSON.stringify(value)}`)
  }
  return millis
}

/** A flag: `true`/`1`/present-and-empty or `false`/`0`, `fallback` when absent. */
function flagParam(params, name, fallback) {
  const value = params.get(name)
  if (value === null) return fallback
  if (value === '' || value === 'true' || value === '1') return true
  if (value === 'false' || value === '0') return false
  throw new HttpError(400, `${name}: expected true or false, got ${JSON.stringify(value)}`)
}

/** The request body as text, bounded by `BODY_LIMIT`. */
async function bodyText(req) {
  const chunks = []
  let length = 0
  for await (const chunk of req) {
    length += chunk.length
    if (length > BODY_LIMIT) throw new HttpError(413, `request body exceeds ${BODY_LIMIT} bytes`)
    chunks.push(chunk)
  }
  return Buffer.concat(chunks).toString('utf8')
}

/**
 * The request body as JSON; an instant - `currunix` or any `marketdata`
 * instant column - stays the text it was written as, so a nanosecond count
 * past 2^53 arrives exact.
 */
async function bodyJson(req) {
  const text = await bodyText(req)
  if (!text.trim()) return {}
  try {
    return JSON.parse(text, function revive(key, value, context) {
      if (typeof value !== 'number' || !INSTANT_COLUMNS.has(key)) return value
      if (context && typeof context.source === 'string') return context.source
      if (Number.isSafeInteger(value)) return String(value)
      throw new HttpError(400, `${key}: expected the decimal text of nanoseconds, got a number past 2^53`)
    })
  } catch (error) {
    if (error instanceof HttpError) throw error
    throw new HttpError(400, `the request body is not JSON: ${error.message}`)
  }
}

// ---- responses -------------------------------------------------------------

function sendJson(res, status, body, headers = {}) {
  const text = toJson(body)
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': Buffer.byteLength(text),
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
    ...headers,
  })
  res.end(text)
}

function sendEmpty(res, status) {
  res.writeHead(status, { 'Cache-Control': 'no-store' })
  res.end()
}

/** Wait until `res` drains or closes; at once when its client has already left. */
function drained(res) {
  if (res.destroyed) return Promise.resolve()
  return new Promise((resolve) => {
    const done = () => {
      res.off('drain', done)
      res.off('close', done)
      resolve()
    }
    res.on('drain', done)
    res.on('close', done)
  })
}

/**
 * Stream `books` as server-sent events - `event: book` per book in walk
 * order, then `event: end` with the count and `end`'s facts - rendering them
 * a chunk at a time and stopping when the client leaves.
 */
async function streamBooks(res, books, end = {}) {
  res.writeHead(200, {
    'Content-Type': 'text/event-stream; charset=utf-8',
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
  })
  // A client may have left before this listener exists: its close has fired.
  let closed = res.destroyed
  res.on('close', () => {
    closed = true
  })
  let count = 0
  for (let at = 0; at < books.length && !closed; at += SSE_CHUNK) {
    for (const json of booksJson(books.slice(at, at + SSE_CHUNK))) {
      if (closed) break
      if (!res.write(`event: book\ndata: ${toJson(json)}\n\n`)) await drained(res)
      count += 1
    }
  }
  if (!closed) res.end(`event: end\ndata: ${toJson({ count, ...end })}\n\n`)
}

// ---- the service -----------------------------------------------------------

/** Loaded sources by id: a `Map` keyed by id, or any iterable of loaded sources. */
function sourcesOf(sources) {
  if (sources instanceof Map) return new Map(sources)
  const byId = new Map()
  for (const source of sources ?? []) {
    if (byId.has(source.id)) throw new Error(`two sources share the id ${source.id}`)
    byId.set(source.id, source)
  }
  return byId
}

/**
 * The replay service over loaded sources (`loadSource` answers each):
 * `listen(port = 0, host = '127.0.0.1')` answers the URL it serves at,
 * `close()` ends every open stream and stops. `stateDir` holds the scenarios
 * (when omitted, a fresh temporary folder of the service's own, created on
 * the first save and removed by `close`), `webDir` the component library and
 * the page, `snapshotMillis` and `global` the walk a request that names
 * neither reads.
 */
function createReplayServer(options = {}) {
  const { sources, stateDir, webDir = WEB_DIR, snapshotMillis: defaultMillis = 0, global: defaultGlobal = false } = options
  const states = new Map([...sourcesOf(sources)].map(([id, source]) => [id, { id, source, walks: new Map() }]))
  // The folder the service made itself, which it alone removes; a named one is the caller's.
  const ownState = stateDir === undefined ? path.join(os.tmpdir(), `yggdryl-replay-${randomUUID()}`) : null
  const stateFolder = stateDir ?? ownState
  let store

  function scenarios() {
    store ??= openScenarios(stateFolder)
    return store
  }

  function stateOf(id) {
    const state = states.get(id)
    if (state === undefined) {
      throw new HttpError(404, `no source ${JSON.stringify(id)}; the sources are ${[...states.keys()].join(', ')}`)
    }
    return state
  }

  /** The walk options a request names, the service's where it names none. */
  function walkOptions(params) {
    return {
      snapshotMillis: millisParam(params, 'snapshotMillis', defaultMillis),
      global: flagParam(params, 'global', defaultGlobal),
    }
  }

  /** The source's books under `walkOptions`, walked once and held. */
  function walkOf(state, walkOpts) {
    const key = `${walkOpts.snapshotMillis}:${walkOpts.global}`
    let held = state.walks.get(key)
    if (held === undefined) {
      const books = walk(state.source.operations, walkOpts)
      held = { books, index: indexBooks(books) }
      state.walks.set(key, held)
      if (state.walks.size > WALKS_PER_SOURCE) state.walks.delete(state.walks.keys().next().value)
    }
    return held
  }

  /**
   * The symbol a request names, checked against the walks it reads - one, or
   * the base and the scenario's - and known when either has it; the one
   * `GLOBAL` of a consolidated walk by default.
   */
  function symbolParam(params, indexes, walkOpts, required) {
    const known = [...new Set(indexes.flatMap((index) => [...index.bySymbol.keys()]))]
    let symbol = params.get('symbol') ?? undefined
    if (symbol === undefined && walkOpts.global) symbol = graph.GLOBAL_SYMBOL
    if (symbol === undefined) {
      if (!required) return undefined
      throw new HttpError(400, `symbol: expected one of ${known.join(', ')}`)
    }
    if (!known.includes(symbol)) {
      const walks = indexes.length > 1 ? 'the base or the scenario walk; their' : 'this walk; its'
      throw new HttpError(404, `no symbol ${JSON.stringify(symbol)} in ${walks} symbols are ${known.join(', ')}`)
    }
    return symbol
  }

  /**
   * What `read(store)` answers of the scenario `name`: a name that names no
   * file is the request's fault (400), a file that cannot be read back the
   * service's (500, its message).
   */
  async function stored(name, read) {
    const store = await scenarios()
    checkName(name)
    try {
      return read(store)
    } catch (error) {
      throw new HttpError(500, refusalText(error))
    }
  }

  /** `{ scenario, operations }`: the stored scenario and its events' native leaves, each decoded once. */
  async function loadScenario(name) {
    const loaded = await stored(name, (store) => store.load(name))
    if (loaded === null) throw new HttpError(404, `no scenario ${JSON.stringify(name)}`)
    return loaded
  }

  const routes = [
    // The insert form's vocabulary: every column, and the kinds an inserted event may name - the dated
    // ones, because admission is the native walk over the leaf alone and it refuses an undated leaf.
    ['GET', ['api', 'field'], () => ({ columns: MARKETDATA_COLUMNS, kinds: DATED_KINDS })],
    [
      'GET',
      ['api', 'sources'],
      () => ({
        snapshotMillis: defaultMillis,
        global: defaultGlobal,
        // The view names the view route reads: the package's own listing, served so no page keeps a copy.
        views: enums.marketViews,
        sources: [...states.values()].map(({ id, source }) => ({
          id,
          kind: source.kind,
          name: source.name,
          operations: source.operations.length,
          symbols: [...source.symbols.filter((symbol) => symbol !== graph.GLOBAL_SYMBOL), graph.GLOBAL_SYMBOL],
          refusals: source.refusals ?? [],
        })),
      }),
    ],
    [
      'GET',
      ['api', 'sources', ':source', 'books'],
      async ({ res, params, source }) => {
        const walkOpts = walkOptions(params)
        const { index } = walkOf(stateOf(source), walkOpts)
        const symbol = symbolParam(params, [index], walkOpts, false)
        await streamBooks(res, booksBetween(index, symbol, instantParam(params, 'from'), instantParam(params, 'to')))
      },
    ],
    [
      'GET',
      ['api', 'sources', ':source', 'book'],
      ({ params, source }) => {
        const walkOpts = walkOptions(params)
        const { index } = walkOf(stateOf(source), walkOpts)
        const symbol = symbolParam(params, [index], walkOpts, true)
        const at = instantParam(params, 'at')
        const book = bookAt(index, symbol, at)
        if (book === null) throw new HttpError(404, `no book of ${symbol} stands at ${at}`)
        return bookJson(book)
      },
    ],
    [
      'GET',
      ['api', 'sources', ':source', 'lifecycle'],
      ({ params, source }) => {
        const { operations } = stateOf(source).source
        const reader = graph.MarketData.arrowReader(operations)
        return rowsOf(graph.MarketData.applyView('lifecycle', reader, [], params.get('crosscode')))
      },
    ],
    [
      'GET',
      ['api', 'sources', ':source', 'view'],
      ({ params, source }) => {
        // The replay's whole stream: the operations, then the books the
        // walk the request names folded them into; each view keeps its kinds.
        const state = stateOf(source)
        const { books } = walkOf(state, walkOptions(params))
        const reader = graph.MarketData.arrowReader([...state.source.operations, ...books])
        const view = params.get('view') ?? ''
        return rowsOf(graph.MarketData.applyView(view, reader, params.getAll('lift'), params.get('crosscode')))
      },
    ],
    [
      'GET',
      ['api', 'sources', ':source', 'scenarios', ':name', 'books'],
      async ({ res, params, source, name }) => {
        const state = stateOf(source)
        const walkOpts = walkOptions(params)
        const { scenario, operations } = await loadScenario(name)
        const { books, from } = await rerun(state.source.operations, scenario, walkOpts, operations)
        const index = indexBooks(books)
        const symbol = symbolParam(params, [index], walkOpts, false)
        const start = instantParam(params, 'from') ?? from ?? undefined
        const range = booksBetween(index, symbol, start, instantParam(params, 'to'))
        await streamBooks(res, range, { from: from === null ? null : String(from) })
      },
    ],
    [
      'GET',
      ['api', 'sources', ':source', 'scenarios', ':name', 'diff'],
      async ({ params, source, name }) => {
        const state = stateOf(source)
        const walkOpts = walkOptions(params)
        const { scenario, operations } = await loadScenario(name)
        const base = walkOf(state, walkOpts).index
        // The re-run first: a symbol only the scenario introduces diffs
        // against an empty base, every instant of it added.
        const { books, from } = await rerun(state.source.operations, scenario, walkOpts, operations)
        const other = indexBooks(books)
        const symbol = symbolParam(params, [base, other], walkOpts, true)
        const bounds = [instantParam(params, 'from'), instantParam(params, 'to')]
        const { diffStreams } = await web()
        const diff = diffStreams(
          booksJson(booksBetween(base, symbol, ...bounds)),
          booksJson(booksBetween(other, symbol, ...bounds)),
        )
        return {
          symbol,
          from: from === null ? null : String(from),
          instants: diff,
        }
      },
    ],
    [
      'GET',
      ['api', 'scenarios'],
      async () => {
        // Every scenario whole, as its file holds it, in one pass over the folder.
        const store = await scenarios()
        const all = store.list().map((name) => {
          try {
            return store.read(name)
          } catch (error) {
            throw new HttpError(500, `scenario ${JSON.stringify(name)}: ${refusalText(error)}`)
          }
        })
        return { scenarios: all.filter((scenario) => scenario !== null) }
      },
    ],
    ['GET', ['api', 'scenarios', ':name'], async ({ name }) => (await loadScenario(name)).scenario],
    [
      'PUT',
      ['api', 'scenarios', ':name'],
      async ({ req, name }) => {
        const body = await bodyJson(req)
        if (body.name !== undefined && body.name !== name) {
          throw new HttpError(400, `the body names the scenario ${JSON.stringify(body.name)}, the path ${JSON.stringify(name)}`)
        }
        if (body.events !== undefined && !Array.isArray(body.events)) throw new HttpError(400, 'events: expected a list')
        // The store rebuilds, admits and re-renders each event, as a POST does.
        return (await scenarios()).save({ name, events: body.events ?? [] })
      },
    ],
    [
      'DELETE',
      ['api', 'scenarios', ':name'],
      async ({ res, name }) => {
        if (!(await scenarios()).remove(name)) throw new HttpError(404, `no scenario ${JSON.stringify(name)}`)
        sendEmpty(res, 204)
      },
    ],
    [
      'POST',
      ['api', 'scenarios', ':name', 'events'],
      async ({ req, res, name }) => {
        const leaf = leafFromJson(await bodyJson(req))
        const { createScenario } = await web()
        const store = await scenarios()
        // The events already held are kept as stored; only the new one is rendered.
        const held = (await stored(name, () => store.read(name))) ?? createScenario(name)
        sendJson(res, 201, store.insert(held, leaf))
      },
    ],
    [
      'DELETE',
      ['api', 'scenarios', ':name', 'events', ':curruuid'],
      async ({ res, name, curruuid }) => {
        const { removeEvent } = await web()
        const { scenario, operations } = await loadScenario(name)
        const kept = removeEvent(scenario, curruuid)
        if (kept === scenario) throw new HttpError(404, `no event ${curruuid} in scenario ${JSON.stringify(name)}`)
        // The leaves the load decoded are handed on, the removed one left out.
        const leaves = operations.filter((_, at) => scenario.events[at].curruuid !== curruuid)
        ;(await scenarios()).save(kept, leaves)
        sendEmpty(res, 204)
      },
    ],
  ]

  /** A route's named segments when `pattern` matches `segments`, else `null`. */
  function bind(pattern, segments) {
    if (pattern.length !== segments.length) return null
    const named = {}
    for (let at = 0; at < pattern.length; at += 1) {
      if (pattern[at].startsWith(':')) named[pattern[at].slice(1)] = segments[at]
      else if (pattern[at] !== segments[at]) return null
    }
    return named
  }

  /** The handlers a path names, by method, with their named segments. */
  function match(segments) {
    const found = new Map()
    for (const [method, pattern, handler] of routes) {
      const named = bind(pattern, segments)
      if (named !== null) found.set(method, [handler, named])
    }
    return found
  }

  async function api(req, res, segments, params) {
    const found = match(segments.at(-1) === '' ? segments.slice(0, -1) : segments)
    if (!found.size) throw new HttpError(404, `no route ${req.method} ${req.url.split('?')[0]}`)
    const held = found.get(req.method)
    if (!held) {
      const allowed = [...found.keys()].join(', ')
      throw new HttpError(405, `${req.method} is not allowed here; allowed: ${allowed}`, { Allow: allowed })
    }
    const [handler, named] = held
    const answer = await handler({ req, res, params, ...named })
    if (!res.headersSent && answer !== undefined) sendJson(res, 200, answer)
  }

  /**
   * A file under `webDir`, served at `/web/*`: the components, and the page
   * at `/web/app/` - a path ending in `/` answers that folder's `index.html`.
   * The page names its files relative to itself, so it is read only from its
   * own folder: `/`, `/app`, `/app/` and `/web/app` answer 308 to
   * `/web/app/`, the query kept.
   */
  async function serveStatic(req, res, segments) {
    if (req.method !== 'GET' && req.method !== 'HEAD') {
      throw new HttpError(405, `${req.method} is not allowed here; allowed: GET, HEAD`, { Allow: 'GET, HEAD' })
    }
    if (PAGE_ALIASES.has(segments.join('/'))) {
      const split = req.url.indexOf('?')
      res.writeHead(308, { Location: `${PAGE}${split === -1 ? '' : req.url.slice(split)}`, 'Cache-Control': 'no-store' })
      res.end()
      return
    }
    if (segments[0] !== 'web') throw new HttpError(404, `no file at ${req.url.split('?')[0]}`)
    const root = webDir
    let rest = segments.slice(1)
    if (rest.at(-1) === '') rest = [...rest.slice(0, -1), 'index.html']
    for (const part of rest) {
      if (part === '.' || part === '..' || /[\\/\0:]/.test(part)) {
        throw new HttpError(403, `the path ${JSON.stringify(req.url.split('?')[0])} leaves the served folder`)
      }
    }
    const target = path.join(root, ...rest)
    const relative = path.relative(root, target)
    if (relative.startsWith('..') || path.isAbsolute(relative)) {
      throw new HttpError(403, `the path ${JSON.stringify(req.url.split('?')[0])} leaves the served folder`)
    }
    let stat
    try {
      stat = await fs.promises.stat(target)
    } catch {
      stat = null
    }
    if (rest.includes('') || stat === null || !stat.isFile()) throw new HttpError(404, `no file at ${req.url.split('?')[0]}`)
    res.writeHead(200, {
      'Content-Type': CONTENT_TYPES[path.extname(target).toLowerCase()] ?? 'application/octet-stream',
      'Content-Length': stat.size,
      'Cache-Control': 'no-cache',
      'X-Content-Type-Options': 'nosniff',
    })
    if (req.method === 'HEAD') {
      res.end()
      return
    }
    await new Promise((resolve, reject) => {
      fs.createReadStream(target).on('error', reject).on('end', resolve).pipe(res)
    })
  }

  async function handle(req, res) {
    try {
      const split = req.url.indexOf('?')
      const rawPath = split === -1 ? req.url : req.url.slice(0, split)
      const params = new URLSearchParams(split === -1 ? '' : req.url.slice(split + 1))
      const segments = segmentsOf(rawPath)
      if (segments[0] === 'api') await api(req, res, segments, params)
      else await serveStatic(req, res, segments)
    } catch (error) {
      if (res.headersSent) {
        res.destroy(error)
        return
      }
      const status = error instanceof HttpError ? error.status : typeof error?.code === 'string' && error.code.startsWith('E') ? 500 : 400
      sendJson(res, status, { error: refusalText(error) }, error instanceof HttpError ? error.headers : {})
    }
  }

  const server = http.createServer((req, res) => {
    handle(req, res)
  })

  return {
    server,
    /** The folder the scenarios are kept in. */
    stateDir: stateFolder,
    /** Serve on `port` (an ephemeral one by default), answering the base URL. */
    listen(port = 0, host = '127.0.0.1') {
      return new Promise((resolve, reject) => {
        server.once('error', reject)
        server.listen(port, host, () => {
          server.off('error', reject)
          const { address, port: bound } = server.address()
          resolve(`http://${address.includes(':') ? `[${address}]` : address}:${bound}/`)
        })
      })
    },
    /** Stop serving, ending every open connection and stream; the state folder the service made is removed. */
    close() {
      return new Promise((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()))
        server.closeAllConnections()
      }).finally(() => {
        if (ownState !== null) fs.rmSync(ownState, { recursive: true, force: true })
      })
    },
  }
}

module.exports = { createReplayServer, drained, streamBooks, CONTENT_TYPES, WEB_DIR }
