// The replay service's wire and nothing else: one method per route, each
// answering the served JSON untouched - instants, decimals and 64-bit
// integers stay the text they crossed as - and a refusal as `{ refusal }`,
// the service's own text verbatim, with its `status`. The book stream is
// server-sent events: `onBook` sees each book in walk order and the promise
// answers the `end` event's `{ count }`; the source is closed on `end`, so
// the browser never reconnects and replays it. This object is the page's
// whole contract with the wire: a recorded manifest answering the same two
// methods stands in for it (the docs page's replay does).

const segment = encodeURIComponent

/** The client of the service at `baseUrl`; every route is absolute under it. */
export function createApi(baseUrl = globalThis.location?.origin) {
  if (!baseUrl) throw new TypeError('expected the base URL of the replay service')

  /** `path` under the service with `query`: absent values are left out, a list repeats its name. */
  function url(path, query = {}) {
    const target = new URL(path, baseUrl)
    for (const [name, value] of Object.entries(query)) {
      if (value === undefined || value === null) continue
      for (const item of Array.isArray(value) ? value : [value]) target.searchParams.append(name, String(item))
    }
    return target
  }

  /** One request: the served JSON, `{ refusal, status }` for a refused one. */
  async function request(method, path, { query } = {}) {
    const response = await fetch(url(path, query), { method, headers: { accept: 'application/json' } })
    const text = await response.text()
    let json
    try {
      json = text ? JSON.parse(text) : {}
    } catch {
      json = undefined
    }
    if (!response.ok) return { refusal: typeof json?.error === 'string' ? json.error : text || `${response.status} ${response.statusText}`, status: response.status }
    if (json === undefined) throw new TypeError(`${method} ${path}: the service answered no JSON`)
    return json
  }

  /**
   * A book stream: `onBook(book)` per `event: book`, answering the `end`
   * event's data. A stream the service refused answers `{ refusal }` - read
   * once more as a plain request, since an event source hides the status -
   * and one cut short answers how many books came first.
   */
  function stream(target, { onBook, signal } = {}) {
    return new Promise((resolve, reject) => {
      if (signal?.aborted) {
        reject(signal.reason)
        return
      }
      const source = new EventSource(target)
      let received = 0
      let settled = false
      const finish = (settle, value) => {
        if (settled) return
        settled = true
        source.close()
        signal?.removeEventListener('abort', abort)
        settle(value)
      }
      const abort = () => finish(reject, signal.reason)
      signal?.addEventListener('abort', abort, { once: true })
      source.addEventListener('book', (event) => {
        if (settled) return
        received += 1
        try {
          onBook?.(JSON.parse(event.data))
        } catch (error) {
          finish(reject, error)
        }
      })
      source.addEventListener('end', (event) => finish(resolve, JSON.parse(event.data)))
      source.addEventListener('error', () => {
        if (settled) return
        // Closed at once: an event source left open reconnects and replays.
        source.close()
        if (received) {
          finish(resolve, { refusal: `the book stream closed after ${received} books, before its end`, status: 0 })
          return
        }
        fetch(target, { signal })
          .then(async (response) => {
            if (response.ok) {
              await response.body?.cancel()
              return { refusal: 'the book stream failed before its first book', status: 0 }
            }
            const text = await response.text()
            let error
            try {
              error = JSON.parse(text).error
            } catch {
              error = undefined
            }
            return { refusal: typeof error === 'string' ? error : text || `${response.status} ${response.statusText}`, status: response.status }
          })
          .then((answer) => finish(resolve, answer), (error) => finish(reject, error))
      })
    })
  }

  const source = (id) => `/api/sources/${segment(id)}`

  return Object.freeze({
    /** `{ snapshotMillis, global, sources: [{ id, kind, name, operations, symbols, refusals }] }`. */
    sources: () => request('GET', '/api/sources'),
    /** The walk's books: `query` = `{ symbol, global, snapshotMillis, from, to }`. */
    books: (id, query, handlers) => stream(url(`${source(id)}/books`, query), handlers),
  })
}
