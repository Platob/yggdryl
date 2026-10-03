// The book display: one page over the book service.
//
// The state is the selection the header states - table, book, from, to,
// zone, interval - and the bucket chosen on the chart; every selector change
// refetches the candles (debounced), a selection fetches the last book of the
// bucket and its events on both sides, and the download control points at the
// whole range. A book is held as the key the service lists it under - the
// instrument's ISIN, else the ticker - which every query sends as its
// `ticker` parameter, and labelled by its ticker where it states one. `from`,
// `to` and the chosen bucket are held as instants
// (nanoseconds, `bigint`) and sent as UTC text, so a zone change moves the
// wall clocks the inputs show and never the range asked. The URL hash carries
// the selection, so a view is a link, and a link pasted into the page is
// followed. Each kind of read - tables, tickers, candles, a bucket - has one
// request in flight: a newer one aborts it, and an answer that is no longer
// the latest is dropped.
//
// `start(document)` runs when the module loads in a browser; under Node the
// module only exports its pure pieces (`readHash`, `writeHash`,
// `timezoneChoices`, `apiBase`, `debounce`).

import { ApiError, fetchBook, fetchCandles, fetchEvents, fetchTables, fetchTickers, fetchTimezones } from './api.js'
import { downloadLink, formatInstant, instantNanos, instantText, renderEvents, renderSummary } from './audit.js'
import { drawCandles } from './chart.js'
import { applyTheme, readTheme, themeLabel, toggleTheme } from './theme.js'

/** The candle intervals the header offers, as the service spells them. */
export const INTERVALS = Object.freeze(['30s', '1m', '5m', '15m', '1h', '1d'])

/** The events asked per side of a bucket; the service bounds the answer and says when it cut it. */
export const EVENT_LIMIT = 5000

/**
 * The keys the hash carries, in the order they are written: `ticker` the
 * book key, as the service's parameter of that name reads it.
 */
export const HASH_KEYS = Object.freeze(['table', 'ticker', 'from', 'to', 'tz', 'interval', 'at'])

/** The debounce of a selector change before the candles are refetched, in milliseconds. */
export const REFRESH_DELAY = 150

/** How a `datetime-local` input spells an instant: the wall clock in the zone, to the second. */
const WALL_CLOCK = Object.freeze({ fraction: 0, separator: 'T' })

/** The selection a URL hash spells, only the keys it names. */
export function readHash(hash = '') {
  const params = new URLSearchParams(String(hash).replace(/^#/, ''))
  const state = {}
  for (const key of HASH_KEYS) {
    const value = params.get(key)
    if (value !== null && value !== '') state[key] = value
  }
  return state
}

/** The hash of a selection: `#table=..&ticker=..`, the empty string when nothing is set. */
export function writeHash(state = {}) {
  const params = new URLSearchParams()
  for (const key of HASH_KEYS) {
    const value = state[key]
    if (value !== undefined && value !== null && value !== '') params.set(key, String(value))
  }
  const text = params.toString()
  return text.length === 0 ? '' : `#${text}`
}

/**
 * The zones the Timezone select offers: exactly the zones the service reads
 * (`api/timezones`), the browser's first where it is one of them, then UTC,
 * then the rest by name.
 */
export function timezoneChoices(served = [], browserZone = 'UTC') {
  const first = [browserZone, 'UTC'].filter((zone, index, all) => served.includes(zone) && all.indexOf(zone) === index)
  const rest = served.filter((zone) => !first.includes(zone)).sort()
  return [...first, ...rest]
}

/** The API base of a page: its URL up to the folder the page is served from, with no query or hash. */
export function apiBase(baseURI) {
  const url = new URL(String(baseURI))
  url.search = ''
  url.hash = ''
  const segments = url.pathname.split('/')
  const last = segments[segments.length - 1]
  if (last !== '' && last.includes('.')) segments[segments.length - 1] = ''
  else if (last !== '') segments.push('')
  url.pathname = segments.join('/')
  return url.toString()
}

/** `fn` called once `wait` milliseconds after its last call. */
export function debounce(fn, wait, setTimer = globalThis.setTimeout, clearTimer = globalThis.clearTimeout) {
  let handle = null
  return (...args) => {
    if (handle !== null) clearTimer(handle)
    handle = setTimer(() => {
      handle = null
      fn(...args)
    }, wait)
  }
}

function fill(select, entries, chosen) {
  select.replaceChildren()
  for (const { value, label, title } of entries) {
    const option = select.ownerDocument.createElement('option')
    option.value = value
    option.textContent = label
    if (title) option.title = title
    select.append(option)
  }
  const values = entries.map((entry) => entry.value)
  select.value = values.includes(chosen) ? chosen : (values[0] ?? '')
  select.disabled = entries.length === 0
  return select.value
}

function describe(error) {
  if (error instanceof ApiError) return error.status ? `${error.status}: ${error.message}` : error.message
  return error?.message ?? String(error)
}

/**
 * One kind of read at a time: `begin()` aborts the read before and answers
 * the controller of the new one, `cancel()` aborts it, `current(control)`
 * says whether `control` is still the latest - an answer that is not is
 * dropped, whenever it lands.
 */
function lane() {
  let latest = null
  return {
    begin() {
      latest?.abort()
      latest = new AbortController()
      return latest
    },
    cancel() {
      latest?.abort()
      latest = null
    },
    current: (control) => latest === control && !control.signal.aborted,
  }
}

/** Wire the page. Exported so a host page can start it on a document of its own. */
export function start(document) {
  const window = document.defaultView
  const byId = (id) => document.getElementById(id)
  const elements = {
    skip: byId('skip'),
    table: byId('table'),
    ticker: byId('ticker'),
    from: byId('from'),
    to: byId('to'),
    tz: byId('tz'),
    interval: byId('interval'),
    theme: byId('theme'),
    chart: byId('chart'),
    tooltip: byId('tooltip'),
    readout: byId('readout'),
    status: byId('status'),
    title: byId('chart-title'),
    summary: byId('summary'),
    bid: byId('bid-events'),
    ask: byId('ask-events'),
    downloads: byId('downloads'),
  }

  // The fetch of the window the page is started in.
  const fetch = typeof window.fetch === 'function' ? window.fetch.bind(window) : undefined

  let browserZone = 'UTC'
  try {
    browserZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    browserZone = 'UTC'
  }

  const state = {
    base: apiBase(document.baseURI),
    zones: [],
    table: '',
    ticker: '',
    from: null,
    to: null,
    tz: 'UTC',
    interval: '1m',
    at: null,
    candles: [],
    tickers: [],
  }
  const reads = { tables: lane(), tickers: lane(), candles: lane(), bucket: lane() }
  let chart = null

  const status = (text, kind = '') => {
    elements.status.textContent = text
    elements.status.dataset.kind = kind
    elements.status.hidden = text === ''
  }

  const wallClock = (instant) => formatInstant(instant, state.tz, WALL_CLOCK)

  // The selected book's name: the ticker its books state, else its key.
  const bookName = () => state.tickers.find((entry) => entry.key === state.ticker)?.ticker ?? state.ticker

  const query = () => ({ table: state.table, ticker: state.ticker, from: instantText(state.from), to: instantText(state.to), tz: state.tz })

  const currentHash = () => writeHash({ ...query(), interval: state.interval, at: instantText(state.at) })

  const syncHash = () => {
    const hash = currentHash()
    if (window.location.hash !== hash) window.history.replaceState(null, '', `${window.location.pathname}${window.location.search}${hash}`)
  }

  // The inputs show the range's instants as wall clocks in the zone; a zone change shows them again.
  const showRange = () => {
    elements.from.value = wallClock(state.from)
    elements.to.value = wallClock(state.to)
  }

  const downloads = () => {
    elements.downloads.replaceChildren()
    const ready = state.table && state.ticker && state.from !== null && state.to !== null
    for (const suffix of ['.csv', '.csv.gz', '.csv.zst']) {
      const link = downloadLink(state.base, query(), suffix)
      const anchor = document.createElement('a')
      anchor.className = 'download'
      anchor.textContent = link.label
      anchor.type = link.type
      if (ready) {
        anchor.href = link.href
        // The route's Content-Disposition names the file.
        anchor.setAttribute('download', '')
      } else {
        anchor.setAttribute('aria-disabled', 'true')
        anchor.tabIndex = -1
      }
      elements.downloads.append(anchor)
    }
  }

  const draw = () => {
    chart = drawCandles(elements.chart, state.candles, {
      zone: state.tz,
      selected: state.at,
      tooltip: elements.tooltip,
      readout: elements.readout,
      emptyText: state.ticker ? 'No candles in this range' : 'No ticker selected',
      onSelect: (candle) => select(candle),
    })
    const range = state.candles.length > 0 ? `${state.candles.length} buckets` : 'no buckets'
    elements.chart.setAttribute(
      'aria-label',
      state.ticker ? `${bookName()} bid and ask candles, ${state.interval} in ${state.tz}, ${range}` : 'Bid and ask candles',
    )
    elements.title.textContent = state.ticker ? `${bookName()} · ${state.interval} · ${state.tz}` : 'Bid and ask'
  }

  // The empty panels, which the selection's absence renders; `clearSelection` also drops the selection and its read.
  const showNoSelection = () => {
    renderSummary(elements.summary, null)
    renderEvents(elements.bid, null, 'bid', { zone: state.tz })
    renderEvents(elements.ask, null, 'ask', { zone: state.tz })
  }

  const clearSelection = () => {
    reads.bucket.cancel()
    state.at = null
    showNoSelection()
  }

  const select = async (candle) => {
    const start = instantNanos(candle?.start)
    const end = instantNanos(candle?.end)
    if (start === null || end === null) return
    state.at = start
    syncHash()
    draw()
    const control = reads.bucket.begin()
    const options = { fetch, signal: control.signal }
    const base = { table: state.table, ticker: state.ticker, tz: state.tz }
    const span = { ...base, from: instantText(start), to: instantText(end) }
    try {
      const [book, bid, ask] = await Promise.all([
        // The bucket is [start, end): its last book stands strictly before its end, and the route answers at or before `at`.
        fetchBook(state.base, { ...base, at: instantText(end - 1n) }, options).catch((error) => {
          if (error instanceof ApiError && error.status === 404) return null
          throw error
        }),
        fetchEvents(state.base, { ...span, side: 'bid', limit: EVENT_LIMIT }, options),
        fetchEvents(state.base, { ...span, side: 'ask', limit: EVENT_LIMIT }, options),
      ])
      if (!reads.bucket.current(control)) return
      renderSummary(elements.summary, book, { zone: state.tz, emptyText: 'No book stands before the end of this bucket.' })
      renderEvents(elements.bid, bid.rows ?? [], 'bid', { zone: state.tz, truncated: bid.truncated === true })
      renderEvents(elements.ask, ask.rows ?? [], 'ask', { zone: state.tz, truncated: ask.truncated === true })
    } catch (error) {
      if (!reads.bucket.current(control)) return
      status(`The bucket could not be read - ${describe(error)}`, 'error')
    }
  }

  const refresh = async () => {
    syncHash()
    downloads()
    reads.bucket.cancel()
    const control = reads.candles.begin()
    if (!state.table || !state.ticker || state.from === null || state.to === null) {
      state.candles = []
      draw()
      clearSelection()
      status(state.table ? 'Choose a ticker and a range' : 'No table is served', 'empty')
      return
    }
    status('Loading candles…', 'loading')
    elements.chart.classList.add('is-loading')
    try {
      const answer = await fetchCandles(state.base, { ...query(), interval: state.interval }, { fetch, signal: control.signal })
      if (!reads.candles.current(control)) return
      state.candles = Array.isArray(answer?.candles) ? answer.candles : []
      // What the keyboard last read was a bucket of the view before.
      elements.readout.textContent = ''
      const chosen = state.at === null ? null : (state.candles.find((candle) => instantNanos(candle.start) === state.at) ?? null)
      if (chosen === null) clearSelection()
      draw()
      status(state.candles.length === 0 ? `No books for ${bookName()} between ${wallClock(state.from)} and ${wallClock(state.to)} (${state.tz})` : '', 'empty')
      if (chosen !== null) await select(chosen)
    } catch (error) {
      if (!reads.candles.current(control)) return
      state.candles = []
      draw()
      clearSelection()
      status(`Candles could not be read - ${describe(error)}`, 'error')
    } finally {
      if (reads.candles.current(control)) elements.chart.classList.remove('is-loading')
    }
  }
  // A selector change drops the candles read in flight at once - its answer is of a view that is gone - and reads again once the changes settle.
  const refreshSoon = debounce(refresh, REFRESH_DELAY, window.setTimeout.bind(window), window.clearTimeout.bind(window))
  const scheduled = () => {
    reads.candles.cancel()
    refreshSoon()
  }

  const loadTickers = async () => {
    const control = reads.tickers.begin()
    state.tickers = []
    fill(elements.ticker, [], '')
    if (!state.table) return refresh()
    let tickers
    try {
      tickers = await fetchTickers(state.base, state.table, { fetch, signal: control.signal })
    } catch (error) {
      if (!reads.tickers.current(control)) return
      status(`Tickers could not be read - ${describe(error)}`, 'error')
      state.candles = []
      draw()
      return
    }
    if (!reads.tickers.current(control)) return
    state.tickers = Array.isArray(tickers) ? tickers : []
    state.ticker = fill(
      elements.ticker,
      state.tickers.map((entry) => ({
        value: entry.key,
        label: `${entry.ticker ?? entry.key} · ${entry.books} book${entry.books === 1 ? '' : 's'}`,
        title: entry.crosscode,
      })),
      keyOf(state.ticker),
    )
    if (state.from === null || state.to === null) spanOf(state.ticker)
    return refresh()
  }

  // The book a link names, as the service reads its `ticker` parameter: a
  // listed key, else the key of the one book whose ticker it is.
  const keyOf = (name) => {
    if (state.tickers.some((entry) => entry.key === name)) return name
    const named = state.tickers.filter((entry) => entry.ticker === name)
    return named.length === 1 ? named[0].key : name
  }

  // The range of a book: its span as the service states it, `from` its first book's second and `to` the second after its last.
  const spanOf = (key) => {
    const chosen = state.tickers.find((entry) => entry.key === key)
    if (!chosen) return
    state.from = instantNanos(chosen.from)
    state.to = instantNanos(chosen.to)
    showRange()
  }

  const loadTables = async () => {
    const control = reads.tables.begin()
    status('Loading tables…', 'loading')
    let tables
    try {
      tables = await fetchTables(state.base, { fetch, signal: control.signal })
    } catch (error) {
      if (!reads.tables.current(control)) return
      status(`Tables could not be read - ${describe(error)}`, 'error')
      draw()
      return
    }
    if (!reads.tables.current(control)) return
    state.table = fill(
      elements.table,
      (Array.isArray(tables) ? tables : []).map((entry) => ({ value: entry.name, label: entry.name, title: entry.url })),
      state.table,
    )
    return loadTickers()
  }

  /**
   * Take the view a hash names: the interval, the zone - one the service
   * reads, else the browser's where it is one, else UTC - the table, the
   * book - its key, or a ticker one listed book states - and the range and
   * the bucket as instants, a wall clock in a hand-written link read in the
   * link's zone.
   */
  const applyHash = (hash) => {
    state.interval = fill(elements.interval, INTERVALS.map((value) => ({ value, label: value })), INTERVALS.includes(hash.interval) ? hash.interval : '1m')
    const fallback = state.zones.includes(browserZone) ? browserZone : 'UTC'
    const zones = timezoneChoices(state.zones, browserZone)
    state.tz = fill(elements.tz, zones.map((value) => ({ value, label: value })), state.zones.includes(hash.tz) ? hash.tz : fallback)
    const zone = hash.tz ?? state.tz
    state.table = hash.table ?? ''
    state.ticker = hash.ticker ?? ''
    state.from = instantNanos(hash.from, zone)
    state.to = instantNanos(hash.to, zone)
    state.at = instantNanos(hash.at, zone)
    showRange()
  }

  // The zones the service reads; where it cannot say, UTC and the zone the view asks for.
  const loadZones = async (wanted) => {
    let zones
    try {
      zones = await fetchTimezones(state.base, { fetch })
    } catch {
      zones = null
    }
    const served = Array.isArray(zones) ? zones.filter((zone) => typeof zone === 'string' && zone !== '') : []
    state.zones = served.length > 0 ? served : [...new Set(['UTC', wanted])]
  }

  // The theme: stamped now, cycled by the button, redrawn when the system changes.
  const themeButton = elements.theme
  let themeName = readTheme()
  applyTheme(themeName)
  const themeText = () => {
    themeButton.textContent = themeLabel(themeName)
    themeButton.dataset.theme = themeName
    themeButton.setAttribute('aria-label', `${themeLabel(themeName)} - press to change`)
  }
  themeText()
  themeButton.addEventListener('click', () => {
    themeName = toggleTheme(themeName)
    themeText()
    draw()
  })
  try {
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => draw())
  } catch {
    // No media queries: the theme is whatever was stamped.
  }

  // The skip link moves the focus to the chart and leaves the hash - the view - alone.
  elements.skip.addEventListener('click', (event) => {
    event.preventDefault()
    elements.chart.focus()
  })

  // The header selectors.
  elements.table.addEventListener('change', () => {
    state.table = elements.table.value
    state.ticker = ''
    state.from = state.to = null
    reads.candles.cancel()
    clearSelection()
    loadTickers()
  })
  elements.ticker.addEventListener('change', () => {
    state.ticker = elements.ticker.value
    clearSelection()
    spanOf(state.ticker)
    scheduled()
  })
  for (const key of ['from', 'to']) {
    elements[key].addEventListener('change', () => {
      // A wall clock in the zone, to the minute or the second as the input states it.
      state[key] = instantNanos(elements[key].value, state.tz)
      clearSelection()
      scheduled()
    })
  }
  elements.tz.addEventListener('change', () => {
    state.tz = elements.tz.value
    showRange()
    scheduled()
  })
  elements.interval.addEventListener('change', () => {
    state.interval = elements.interval.value
    clearSelection()
    scheduled()
  })

  // A view link pasted into the page is followed; a fragment naming no view (`#chart`) is not one.
  window.addEventListener('hashchange', () => {
    const hash = readHash(window.location.hash)
    if (Object.keys(hash).length === 0) {
      syncHash()
      return
    }
    if (window.location.hash === currentHash()) return
    reads.candles.cancel()
    reads.bucket.cancel()
    applyHash(hash)
    showNoSelection()
    loadTables()
  })

  // The chart follows its box.
  if (typeof window.ResizeObserver === 'function') {
    new window.ResizeObserver(() => {
      if (chart) draw()
    }).observe(elements.chart.parentElement)
  } else {
    window.addEventListener('resize', () => draw())
  }

  // The empty panels, leaving the linked bucket in the state for the candles to find.
  const linked = readHash(window.location.hash)
  showNoSelection()
  downloads()
  draw()
  status('Loading…', 'loading')
  loadZones(linked.tz ?? browserZone).then(() => {
    applyHash(linked)
    downloads()
    draw()
    return loadTables()
  })
}

if (globalThis.document?.getElementById?.('chart')) start(globalThis.document)
