'use strict'

// The replay service: Node's `http` over the native package. Every answer is
// the native package's - the sources, the walk's books, a refusal - rendered
// by `json.js`; the service routes, holds the walks it served, streams books
// as server-sent events and serves the page. Instants cross as the decimal
// text of nanoseconds, decimals as their exact text, and a refusal as the
// native message, verbatim.

const fs = require('node:fs')
const http = require('node:http')
const path = require('node:path')

const { graph } = require('../binding.js')

const { booksJson, refusalText, toJson } = require('./json.js')
const { booksBetween, indexBooks, walk } = require('./walk.js')

/** The component library and the page, beside this package. */
const WEB_DIR = path.join(__dirname, '..', 'web')
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
 * order, then `event: end` with the count - rendering them a chunk at a
 * time and stopping when the client leaves.
 */
async function streamBooks(res, books) {
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
  if (!closed) res.end(`event: end\ndata: ${toJson({ count })}\n\n`)
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
 * `close()` ends every open stream and stops. `webDir` holds the component
 * and the page, `snapshotMillis` and `global` the walk a request that names
 * neither reads.
 */
function createReplayServer(options = {}) {
  const { sources, webDir = WEB_DIR, snapshotMillis: defaultMillis = 0, global: defaultGlobal = false } = options
  const states = new Map([...sourcesOf(sources)].map(([id, source]) => [id, { id, source, walks: new Map() }]))

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

  /** The symbol a request names, checked against the walk it reads; `GLOBAL` for a consolidated walk by default. */
  function symbolParam(params, index, walkOpts) {
    const known = [...index.bySymbol.keys()]
    let symbol = params.get('symbol') ?? undefined
    if (symbol === undefined && walkOpts.global) symbol = graph.GLOBAL_SYMBOL
    if (symbol !== undefined && !known.includes(symbol)) {
      throw new HttpError(404, `no symbol ${JSON.stringify(symbol)} in this walk; its symbols are ${known.join(', ')}`)
    }
    return symbol
  }

  const routes = [
    [
      'GET',
      ['api', 'sources'],
      () => ({
        snapshotMillis: defaultMillis,
        global: defaultGlobal,
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
        const symbol = symbolParam(params, index, walkOpts)
        await streamBooks(res, booksBetween(index, symbol, instantParam(params, 'from'), instantParam(params, 'to')))
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
    const answer = await handler({ res, params, ...named })
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
    /** Stop serving, ending every open connection and stream. */
    close() {
      return new Promise((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()))
        server.closeAllConnections()
      })
    },
  }
}

module.exports = { createReplayServer, drained, streamBooks, CONTENT_TYPES, WEB_DIR }
