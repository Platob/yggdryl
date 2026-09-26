// The replay service's wire and nothing else: one method per route, each
// answering the served JSON untouched - instants, decimals and 64-bit
// integers stay the text they crossed as - and a refusal as `{ refusal }`,
// the service's own text verbatim, with its `status`. The book streams are
// server-sent events: `onBook` sees each book in walk order and the promise
// answers the `end` event's `{ count, from }`; the source is closed on `end`,
// so the browser never reconnects and replays it. This object is the app's
// whole contract with the wire: a recorded manifest answering the same
// methods stands in for it (the docs page's read-only replay does).

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

  /** One request: the served JSON, `{}` for an empty answer, `{ refusal, status }` for a refused one. */
  async function request(method, path, { query, body } = {}) {
    const init = { method, headers: { accept: 'application/json' } }
    if (body !== undefined) {
      init.headers['content-type'] = 'application/json'
      init.body = JSON.stringify(body)
    }
    const response = await fetch(url(path, query), init)
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
  const scenario = (name) => `/api/scenarios/${segment(name)}`

  return Object.freeze({
    /** `{ snapshotMillis, global, views, sources: [{ id, kind, name, operations, symbols, refusals }] }`. */
    sources: () => request('GET', '/api/sources'),
    /**
     * `{ views }`: the view names alone. `sources()` answers them in the same
     * listing, which is where the app reads them; this method keeps the
     * surface a recorded `api` (the docs page's) answers whole.
     */
    views: async () => ({ views: (await request('GET', '/api/sources')).views }),
    /** `{ columns: [{ name, dtype, nullable }], kinds }`: what an inserted event may state. */
    field: () => request('GET', '/api/field'),
    /** The walk's books: `query` = `{ symbol, global, snapshotMillis, from, to }`. */
    books: (id, query, handlers) => stream(url(`${source(id)}/books`, query), handlers),
    /** The book standing at `query.at`. */
    book: (id, query) => request('GET', `${source(id)}/book`, { query }),
    /** `{ columns, rows }`: the lifecycle view of one chain. */
    lifecycle: (id, crosscode) => request('GET', `${source(id)}/lifecycle`, { query: { crosscode } }),
    /** `{ columns, rows }`: a named view with its lifts, over the walk `query` names. */
    view: (id, { view, lifts = [], ...query }) => request('GET', `${source(id)}/view`, { query: { ...query, view, lift: lifts } }),
    /** `{ scenarios: [{ name, events }] }`. */
    scenarios: () => request('GET', '/api/scenarios'),
    /** Store a scenario whole: `{ name, events }` as the service answers it. */
    saveScenario: (name, body) => request('PUT', scenario(name), { body }),
    deleteScenario: (name) => request('DELETE', scenario(name)),
    /** Insert `{ kind, currunix, facts }`: the stored event, or the native refusal. */
    insert: (name, event) => request('POST', `${scenario(name)}/events`, { body: event }),
    removeEvent: (name, curruuid) => request('DELETE', `${scenario(name)}/events/${segment(curruuid)}`),
    /** The scenario's books, from `query.from`, else from the earliest instant it affects. */
    scenarioBooks: (id, name, query, handlers) => stream(url(`${source(id)}/scenarios/${segment(name)}/books`, query), handlers),
    /** `{ symbol, from, instants: [{ at, base, scenario, changes }] }`. */
    diff: (id, name, query) => request('GET', `${source(id)}/scenarios/${segment(name)}/diff`, { query }),
  })
}
