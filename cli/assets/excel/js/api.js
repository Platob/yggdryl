// The one door to the service: every request carries `X-Yggdryl-Token`, every
// error is an RFC 9457 problem shown as a toast, and edits carry the revision
// they were made against. It also keeps the sheet gate: a request naming a
// sheet (`sheets/{key}/…`) goes out only while that sheet is in the last
// workbook document and no edit is removing it, so a removed sheet is never
// asked for (no 404) and an answer arriving after it went is ignored.

const TOKEN_HEADER = 'X-Yggdryl-Token'
const TOKEN_KEY = 'yggdryl-token'
// How long a removal waits for its sheet's requests on their way before it
// cancels them.
const CLOSE_LIMIT = 5000

// The sheet a path names, or null.
const sheetOfPath = (path) => {
  const match = /^sheets\/(\d+)\//.exec(path)
  return match ? Number(match[1]) : null
}

// What a request refused by the gate throws: a cancellation, which every
// caller already takes quietly.
const closed = (key) => new DOMException(`Sheet ${key} is being removed.`, 'AbortError')

// The token arrives in the URL fragment, moves to session storage, and leaves
// the address bar so it never reaches history, a bookmark or a Referer.
export const takeToken = () => {
  const match = /(?:^#|&)token=([^&]*)/.exec(location.hash)
  if (match) {
    const token = decodeURIComponent(match[1])
    try {
      sessionStorage.setItem(TOKEN_KEY, token)
    } catch {}
    history.replaceState(history.state, '', location.pathname + location.search)
    return token
  }
  try {
    return sessionStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

export class ApiError extends Error {
  constructor (problem) {
    super(problem.detail || problem.title || 'Request failed')
    this.status = problem.status
    this.kind = problem.kind || null
    this.location = problem.location || null
    this.revision = problem.revision ?? null
    this.problem = problem
  }
}

let toastHost = null

export const toast = (message, tone = 'error') => {
  if (!toastHost) {
    toastHost = document.getElementById('toasts')
    if (!toastHost) return
  }
  const item = document.createElement('div')
  item.className = 'toast toast-' + tone
  item.setAttribute('role', tone === 'error' ? 'alert' : 'status')
  const text = document.createElement('span')
  text.textContent = message
  const close = document.createElement('button')
  close.type = 'button'
  close.className = 'toast-close'
  close.setAttribute('aria-label', 'Dismiss')
  close.textContent = '×'
  close.addEventListener('click', () => item.remove())
  item.append(text, close)
  toastHost.append(item)
  setTimeout(() => item.remove(), tone === 'error' ? 8000 : 4000)
}

const problemOf = async (response) => {
  const type = response.headers.get('Content-Type') || ''
  let problem = null
  try {
    if (type.includes('json')) problem = await response.json()
    else {
      const text = await response.text()
      problem = { detail: text.slice(0, 500) }
    }
  } catch {}
  return {
    status: response.status,
    title: response.statusText,
    ...(problem && typeof problem === 'object' ? problem : {})
  }
}

export class Api extends EventTarget {
  constructor (base, token) {
    super()
    this.base = base
    this.token = token
    this.revision = 0
    this.generation = 0
    // The service process this tab talks to (addendum 8): its copy marker
    // names it, so a paste can tell this service's cells from another's.
    this.instance = null
    // Edits go one at a time, so each carries the revision its predecessor
    // answered rather than one the service has already moved past.
    this.edits = Promise.resolve()
    // The sheet gate: the keys of the last workbook document (null before
    // one), the keys an edit is removing (key -> count) and each sheet's
    // requests on their way (key -> Set of AbortController).
    this.sheets = null
    this.closing = new Map()
    this.flights = new Map()
  }

  // The sheets the last workbook document lists.
  setSheets (sheets) {
    this.sheets = new Set((sheets || []).map((sheet) => sheet.key))
  }

  // Whether a request may name sheet `key`.
  isOpen (key) {
    return !this.closing.has(key) && (this.sheets === null || this.sheets.has(key))
  }

  // Before an edit removing `keys` is sent: nothing new is asked of them,
  // and what is on its way lands first - or, past `limit` ms, is cancelled.
  async closeSheets (keys, limit = CLOSE_LIMIT) {
    for (const key of keys) this.closing.set(key, (this.closing.get(key) || 0) + 1)
    const busy = () => keys.some((key) => (this.flights.get(key) || { size: 0 }).size > 0)
    if (busy()) {
      await new Promise((resolve) => {
        const done = () => {
          clearTimeout(timer)
          this.removeEventListener('landed', check)
          resolve()
        }
        const check = () => {
          if (!busy()) done()
        }
        const timer = setTimeout(done, limit)
        this.addEventListener('landed', check)
      })
    }
    for (const key of keys) for (const flight of this.flights.get(key) || []) flight.abort()
  }

  // The edit has answered (the workbook document then says whether the
  // sheets are gone) or was refused (they stay).
  openSheets (keys) {
    for (const key of keys) {
      const count = (this.closing.get(key) || 0) - 1
      if (count > 0) this.closing.set(key, count)
      else this.closing.delete(key)
    }
  }

  url (path) {
    return new URL(path, this.base)
  }

  headers (extra) {
    const headers = new Headers(extra)
    headers.set(TOKEN_HEADER, this.token || 'none')
    return headers
  }

  // One request. `quiet` leaves the toast to the caller; a 304 answers
  // `{ status: 304 }` so the caller keeps what it holds. One naming a sheet
  // the gate has shut is not sent, and one answered after its sheet was
  // shut is dropped: both reject as a cancellation (`AbortError`).
  async request (method, path, options = {}) {
    const key = sheetOfPath(path)
    if (key === null) return this.exchange(method, path, options)
    if (!this.isOpen(key)) throw closed(key)
    // A request naming a sheet is held in `flights` until its answer is
    // read, so a removal can wait for it or cancel it.
    const flight = new AbortController()
    const { signal } = options
    if (signal) {
      if (signal.aborted) flight.abort()
      else signal.addEventListener('abort', () => flight.abort(), { once: true })
    }
    let held = this.flights.get(key)
    if (!held) this.flights.set(key, (held = new Set()))
    held.add(flight)
    try {
      const answer = await this.exchange(method, path, { ...options, signal: flight.signal })
      if (!this.isOpen(key)) throw closed(key)
      return answer
    } finally {
      held.delete(flight)
      if (held.size === 0 && this.flights.get(key) === held) this.flights.delete(key)
      this.dispatchEvent(new CustomEvent('landed', { detail: key }))
    }
  }

  // The fetch itself, and its problem answer as an `ApiError`.
  async exchange (method, path, { body, etag, raw = false, quiet = false, signal } = {}) {
    const headers = this.headers()
    const init = { method, headers, cache: 'no-store', signal }
    if (body !== undefined) {
      headers.set('Content-Type', 'application/json')
      init.body = JSON.stringify(body)
    }
    if (etag) headers.set('If-None-Match', etag)
    let response
    try {
      response = await fetch(this.url(path), init)
    } catch (error) {
      if (error.name === 'AbortError') throw error
      const failure = new ApiError({ status: 0, title: 'Offline', detail: 'The workbook service did not answer' })
      if (!quiet) toast(failure.message)
      throw failure
    }
    if (response.status === 304) return { status: 304, etag: response.headers.get('ETag') || etag }
    if (!response.ok) {
      const failure = new ApiError(await problemOf(response))
      if (failure.revision !== null) this.dispatchEvent(new CustomEvent('revision', { detail: failure.revision }))
      if (failure.kind === 'stale_base') this.dispatchEvent(new CustomEvent('stale', { detail: failure }))
      if (!quiet) toast(failure.message)
      throw failure
    }
    const etagOut = response.headers.get('ETag')
    if (raw) return { status: response.status, etag: etagOut, response }
    const data = response.status === 204 ? null : await response.json()
    return { status: response.status, etag: etagOut, data }
  }

  async get (path, options) {
    return (await this.request('GET', path, options)).data
  }

  async post (path, body, options) {
    return (await this.request('POST', path, { ...options, body })).data
  }

  // A text answer (`export`'s TSV and HTML).
  async text (path, options) {
    const { response } = await this.request('GET', path, { ...options, raw: true })
    return response.text()
  }

  // Read a workbook document and remember the revision it states.
  async workbook () {
    const workbook = await this.get('workbook')
    this.adopt(workbook)
    return workbook
  }

  // A workbook document or an edit's answer. A new generation, or another
  // service instance, starts over; within one, a revision never goes back.
  adopt (state) {
    if (!state) return
    if (typeof state.instance === 'string' && state.instance !== this.instance) {
      this.instance = state.instance
      this.generation = typeof state.generation === 'number' ? state.generation : 0
      if (typeof state.revision === 'number') this.revision = state.revision
      return
    }
    if (typeof state.generation === 'number' && state.generation !== this.generation) {
      this.generation = state.generation
      if (typeof state.revision === 'number') this.revision = state.revision
      return
    }
    if (typeof state.revision === 'number') this.revision = Math.max(this.revision, state.revision)
  }

  // Posts `{ base, ...body }` once the edits before it have answered. The
  // answer, or the refusal, carries the `base` it was made against; the
  // caller decides what to show.
  send (path, body) {
    const run = async () => {
      const base = this.revision
      let answer
      try {
        answer = await this.post(path, { base, ...body }, { quiet: true })
      } catch (error) {
        if (error instanceof ApiError) error.base = base
        throw error
      }
      this.adopt(answer)
      answer.base = base
      this.dispatchEvent(new CustomEvent('applied', { detail: answer }))
      return answer
    }
    const answer = this.edits.then(run, run)
    this.edits = answer.catch(() => {})
    return answer
  }

  // One §8.4 edit against the revision this tab last saw (§8.6).
  edit (edit) {
    return this.send('edits', { edit })
  }

  // `undo` or `redo`, answered as an edit; 409 `nothing_to_undo` when empty.
  history (kind) {
    return this.send(kind, {})
  }

  // `calculate` (F9; `full` with Ctrl+Alt+F9), answered as an edit.
  calculate (full = false) {
    return this.send('calculate', { full })
  }

  // A download is fetched as a blob, so the token travels as a header.
  async download (path, fallback = 'workbook.xlsx') {
    const { response } = await this.request('GET', path, { raw: true })
    const blob = await response.blob()
    const disposition = response.headers.get('Content-Disposition') || ''
    const star = /filename\*=UTF-8''([^;]+)/i.exec(disposition)
    const plain = /filename="([^"]+)"/i.exec(disposition)
    const name = star ? decodeURIComponent(star[1]) : plain ? plain[1] : fallback
    const link = document.createElement('a')
    link.href = URL.createObjectURL(blob)
    link.download = name
    document.body.append(link)
    link.click()
    link.remove()
    setTimeout(() => URL.revokeObjectURL(link.href), 10000)
    return name
  }
}
