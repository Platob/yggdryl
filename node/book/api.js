// The book service's routes, as the display calls them.
//
// Every function is pure over the `fetch` it is handed - the browser's by
// default, a stub in a test - and builds its URL from a plain query object
// through `queryString`, so what the display asks the server is one function
// per route and nothing else. The routes and their JSON are the book service's
// (`rust/src/graph/serve.rs`): a refusal is `{"error": "<text>"}` with 400 for
// a bad parameter, 404 for an unknown table or ticker, 500 otherwise, and
// `ApiError` carries that text and status.

/** The three spellings of the audit download, by the coding each carries. */
export const AUDIT_SUFFIXES = Object.freeze(['.csv', '.csv.gz', '.csv.zst'])

/** An answer the service refused, with its status and the URL it was asked at. */
export class ApiError extends Error {
  constructor(message, { status = 0, url = '' } = {}) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.url = url
  }
}

/**
 * The query string of a plain object, in key order: `?a=1&b=x`, or the empty
 * string when nothing is set. An `undefined` or `null` value is skipped, every
 * other value is spelled as text and percent-encoded.
 */
export function queryString(params = {}) {
  const pairs = []
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) continue
    pairs.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`)
  }
  return pairs.length === 0 ? '' : `?${pairs.join('&')}`
}

/** `base` with exactly one trailing slash, so a relative route resolves under it. */
export function normalizeBase(base) {
  const text = String(base ?? '')
  const bare = text.replace(/[?#].*$/, '')
  return bare.endsWith('/') ? bare : `${bare}/`
}

/** The absolute URL of a route under the API base, with its query. */
export function apiUrl(base, route, params = {}) {
  return new URL(route + queryString(params), normalizeBase(base)).toString()
}

// Every query names its book by the `ticker` parameter, which the service
// reads as a book key - the `key` `fetchTickers` lists, an instrument's ISIN
// or a ticker - else as the ticker of exactly one key's books. The display
// sends the key.

/** The parameters of a candles query, in the route's order. */
export function candleParams({ table, ticker, from, to, tz, interval } = {}) {
  return { table, ticker, from, to, tz, interval }
}

/** The parameters of a book query. */
export function bookParams({ table, ticker, at, tz } = {}) {
  return { table, ticker, at, tz }
}

/** The parameters of an events query; `side` and `limit` are optional. */
export function eventParams({ table, ticker, from, to, tz, side, limit } = {}) {
  return { table, ticker, from, to, tz, side, limit }
}

/** The parameters of an audit download; the whole range, one side or both. */
export function auditParams({ table, ticker, from, to, tz, side } = {}) {
  return { table, ticker, from, to, tz, side }
}

/**
 * Fetch a JSON answer, refusing a non-2xx status with the service's own
 * `error` text when it sent one.
 */
export async function fetchJson(url, { fetch = globalThis.fetch, signal } = {}) {
  if (typeof fetch !== 'function') throw new ApiError('no fetch implementation is available', { url })
  let answer
  try {
    answer = await fetch(url, { signal, headers: { accept: 'application/json' } })
  } catch (cause) {
    if (cause?.name === 'AbortError') throw cause
    throw new ApiError(`the request failed: ${cause?.message ?? cause}`, { url })
  }
  const text = await answer.text()
  let body = null
  try {
    body = text.length === 0 ? null : JSON.parse(text)
  } catch {
    body = null
  }
  if (!answer.ok) {
    const message = typeof body?.error === 'string' ? body.error : `${answer.status} ${answer.statusText ?? ''}`.trim()
    throw new ApiError(message, { status: answer.status, url })
  }
  if (body === null && text.length !== 0) {
    throw new ApiError('the answer is not JSON', { status: answer.status, url })
  }
  return body
}

/** `GET api/tables`: `[{ name, url }]`. */
export function fetchTables(base, options) {
  return fetchJson(apiUrl(base, 'api/tables'), options)
}

/** `GET api/timezones`: `["UTC", ...]`, the zones the service reads - UTC, then every zone it has rules for, by name. */
export function fetchTimezones(base, options) {
  return fetchJson(apiUrl(base, 'api/timezones'), options)
}

/**
 * `GET api/tickers?table=`: `[{ key, ticker, crosscode, from, to, books }]`,
 * one per book key - the instrument's ISIN, else the ticker, else
 * `XX0000000000` - ordered by key, `ticker` the first its books state or
 * `null`.
 */
export function fetchTickers(base, table, options) {
  return fetchJson(apiUrl(base, 'api/tickers', { table }), options)
}

/** `GET api/candles`: the candles of one ticker over a range, in a zone and an interval. */
export function fetchCandles(base, query, options) {
  return fetchJson(apiUrl(base, 'api/candles', candleParams(query)), options)
}

/** `GET api/book`: the last book at or before `at`, or a 404 `ApiError`. */
export function fetchBook(base, query, options) {
  return fetchJson(apiUrl(base, 'api/book', bookParams(query)), options)
}

/** `GET api/events`: `{ rows, truncated }`, the audit rows of a range. */
export function fetchEvents(base, query, options) {
  return fetchJson(apiUrl(base, 'api/events', eventParams(query)), options)
}

/**
 * The URL of the audit download over the whole range, `suffix` one of
 * `AUDIT_SUFFIXES`; an unknown suffix is refused by name.
 */
export function auditUrl(base, query, suffix = '.csv') {
  if (!AUDIT_SUFFIXES.includes(suffix)) {
    throw new RangeError(`audit suffix must be one of ${AUDIT_SUFFIXES.join(', ')}, got ${JSON.stringify(suffix)}`)
  }
  return apiUrl(base, `api/audit${suffix}`, auditParams(query))
}
