// The book display: one page over the book service.
//
// The state is the selection the header states - table, ticker, from, to,
// zone, interval - and the bucket chosen on the chart; every selector change
// refetches the candles (debounced), a selection fetches the last book of the
// bucket and its events on both sides, and the download control points at the
// whole range. The URL hash carries the selection, so a view is a link.
//
// `start(document)` runs when the module loads in a browser; under Node the
// module only exports its pure pieces (`readHash`, `writeHash`,
// `timezoneChoices`, `spanToRange`, `apiBase`).

import { ApiError, fetchBook, fetchCandles, fetchEvents, fetchTables, fetchTickers } from './api.js'
import { downloadLink, renderEvents, renderSummary, formatInstant } from './audit.js'
import { drawCandles } from './chart.js'
import { applyTheme, readTheme, themeLabel, toggleTheme } from './theme.js'

/** The candle intervals the header offers, as the service spells them. */
export const INTERVALS = Object.freeze(['30s', '1m', '5m', '15m', '1h', '1d'])

/** The events asked per side of a bucket; the service bounds the answer and says when it cut it. */
export const EVENT_LIMIT = 5000

/** The keys the hash carries, in the order they are written. */
export const HASH_KEYS = Object.freeze(['table', 'ticker', 'from', 'to', 'tz', 'interval', 'at'])

/** The debounce of a selector change before the candles are refetched, in milliseconds. */
export const REFRESH_DELAY = 150

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

/** The zones a select offers: the browser's, then UTC, then every other supported one in order. */
export function timezoneChoices(supported = [], browserZone = 'UTC') {
  const first = [browserZone, 'UTC'].filter((zone, index, all) => zone && all.indexOf(zone) === index)
  const rest = [...supported].filter((zone) => !first.includes(zone)).sort()
  return [...first, ...rest]
}

/**
 * A ticker's span, two ISO-8601 UTC instants, as the `from` and `to` a
 * `datetime-local` input states in `zone`: `from` at its second and `to` one
 * second past its own, so the exclusive end still covers the last book.
 */
export function spanToRange(from, to, zone = 'UTC') {
  const start = Date.parse(from)
  const end = Date.parse(to)
  if (!Number.isFinite(start) || !Number.isFinite(end)) return { from: '', to: '' }
  const local = (millis) => formatInstant(millis, zone, { fraction: 0, separator: 'T' })
  return { from: local(Math.floor(start / 1000) * 1000), to: local(Math.floor(end / 1000) * 1000 + 1000) }
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

/** Wire the page. Exported so a host page can start it on a document of its own. */
export function start(document) {
  const window = document.defaultView
  const byId = (id) => document.getElementById(id)
  const elements = {
    table: byId('table'),
    ticker: byId('ticker'),
    from: byId('from'),
    to: byId('to'),
    tz: byId('tz'),
    interval: byId('interval'),
    theme: byId('theme'),
    chart: byId('chart'),
    tooltip: byId('tooltip'),
    status: byId('status'),
    title: byId('chart-title'),
    summary: byId('summary'),
    bid: byId('bid-events'),
    ask: byId('ask-events'),
    downloads: byId('downloads'),
  }

  const state = {
    base: apiBase(document.baseURI),
    table: '',
    ticker: '',
    from: '',
    to: '',
    tz: 'UTC',
    interval: '1m',
    at: null,
    candles: [],
    tickers: [],
  }
  let chart = null
  let pending = null
  let selecting = null

  const status = (text, kind = '') => {
    elements.status.textContent = text
    elements.status.dataset.kind = kind
    elements.status.hidden = text === ''
  }

  const query = () => ({ table: state.table, ticker: state.ticker, from: state.from, to: state.to, tz: state.tz })

  const syncHash = () => {
    const hash = writeHash({ ...query(), interval: state.interval, at: state.at ?? undefined })
    if (window.location.hash !== hash) window.history.replaceState(null, '', `${window.location.pathname}${window.location.search}${hash}`)
  }

  const downloads = () => {
    elements.downloads.replaceChildren()
    const ready = state.table && state.ticker && state.from && state.to
    for (const suffix of ['.csv', '.csv.gz', '.csv.zst']) {
      const link = downloadLink(state.base, query(), suffix)
      const anchor = document.createElement('a')
      anchor.className = 'download'
      anchor.textContent = link.label
      anchor.type = link.type
      if (ready) {
        anchor.href = link.href
        anchor.download = link.filename
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
      emptyText: state.ticker ? 'No candles in this range' : 'No ticker selected',
      onSelect: (candle) => select(candle),
    })
    const range = state.candles.length > 0 ? `${state.candles.length} buckets` : 'no buckets'
    elements.chart.setAttribute(
      'aria-label',
      state.ticker ? `${state.ticker} bid and ask candles, ${state.interval} in ${state.tz}, ${range}` : 'Bid and ask candles',
    )
    elements.title.textContent = state.ticker ? `${state.ticker} · ${state.interval} · ${state.tz}` : 'Bid and ask'
  }

  const clearSelection = () => {
    state.at = null
    renderSummary(elements.summary, null)
    renderEvents(elements.bid, null, 'bid', { zone: state.tz })
    renderEvents(elements.ask, null, 'ask', { zone: state.tz })
  }

  const select = async (candle) => {
    if (!candle) return
    state.at = candle.start
    syncHash()
    draw()
    selecting?.abort()
    const control = new AbortController()
    selecting = control
    const options = { signal: control.signal }
    const base = { table: state.table, ticker: state.ticker, tz: state.tz }
    try {
      const [book, bid, ask] = await Promise.all([
        fetchBook(state.base, { ...base, at: candle.end }, options).catch((error) => {
          if (error instanceof ApiError && error.status === 404) return null
          throw error
        }),
        fetchEvents(state.base, { ...base, from: candle.start, to: candle.end, side: 'bid', limit: EVENT_LIMIT }, options),
        fetchEvents(state.base, { ...base, from: candle.start, to: candle.end, side: 'ask', limit: EVENT_LIMIT }, options),
      ])
      if (control.signal.aborted) return
      renderSummary(elements.summary, book, { zone: state.tz, emptyText: 'No book stands at or before the end of this bucket.' })
      renderEvents(elements.bid, bid.rows ?? [], 'bid', { zone: state.tz, truncated: bid.truncated === true })
      renderEvents(elements.ask, ask.rows ?? [], 'ask', { zone: state.tz, truncated: ask.truncated === true })
    } catch (error) {
      if (error?.name === 'AbortError') return
      status(`The bucket could not be read - ${describe(error)}`, 'error')
    }
  }

  const refresh = async () => {
    syncHash()
    downloads()
    pending?.abort()
    if (!state.table || !state.ticker || !state.from || !state.to) {
      state.candles = []
      draw()
      clearSelection()
      status(state.table ? 'Choose a ticker and a range' : 'No table is served', 'empty')
      return
    }
    const control = new AbortController()
    pending = control
    status('Loading candles…', 'loading')
    elements.chart.classList.add('is-loading')
    try {
      const answer = await fetchCandles(state.base, { ...query(), interval: state.interval }, { signal: control.signal })
      if (control.signal.aborted) return
      state.candles = Array.isArray(answer?.candles) ? answer.candles : []
      const chosen = state.candles.find((candle) => candle.start === state.at) ?? null
      if (chosen === null) clearSelection()
      draw()
      status(state.candles.length === 0 ? `No books for ${state.ticker} between ${state.from} and ${state.to} (${state.tz})` : '', 'empty')
      if (chosen !== null) await select(chosen)
    } catch (error) {
      if (error?.name === 'AbortError') return
      state.candles = []
      draw()
      clearSelection()
      status(`Candles could not be read - ${describe(error)}`, 'error')
    } finally {
      if (pending === control) elements.chart.classList.remove('is-loading')
    }
  }
  const scheduled = debounce(refresh, REFRESH_DELAY, window.setTimeout.bind(window), window.clearTimeout.bind(window))

  const loadTickers = async () => {
    state.tickers = []
    fill(elements.ticker, [], '')
    if (!state.table) return refresh()
    try {
      const tickers = await fetchTickers(state.base, state.table)
      state.tickers = Array.isArray(tickers) ? tickers : []
    } catch (error) {
      status(`Tickers could not be read - ${describe(error)}`, 'error')
      state.candles = []
      draw()
      return
    }
    state.ticker = fill(
      elements.ticker,
      state.tickers.map((entry) => ({
        value: entry.ticker,
        label: `${entry.ticker} · ${entry.books} book${entry.books === 1 ? '' : 's'}`,
        title: entry.crosscode,
      })),
      state.ticker,
    )
    const chosen = state.tickers.find((entry) => entry.ticker === state.ticker)
    if (chosen && (!state.from || !state.to)) {
      const range = spanToRange(chosen.from, chosen.to, state.tz)
      state.from = elements.from.value = range.from
      state.to = elements.to.value = range.to
    }
    return refresh()
  }

  const loadTables = async () => {
    let tables = []
    try {
      tables = await fetchTables(state.base)
    } catch (error) {
      status(`Tables could not be read - ${describe(error)}`, 'error')
      draw()
      return
    }
    state.table = fill(
      elements.table,
      (Array.isArray(tables) ? tables : []).map((entry) => ({ value: entry.name, label: entry.name, title: entry.url })),
      state.table,
    )
    return loadTickers()
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

  // The header selectors.
  const hash = readHash(window.location.hash)
  state.interval = INTERVALS.includes(hash.interval) ? hash.interval : '1m'
  fill(elements.interval, INTERVALS.map((value) => ({ value, label: value })), state.interval)
  let supported = []
  try {
    supported = Intl.supportedValuesOf('timeZone')
  } catch {
    supported = []
  }
  let browserZone = 'UTC'
  try {
    browserZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    browserZone = 'UTC'
  }
  const zones = timezoneChoices(supported, browserZone)
  state.tz = fill(elements.tz, zones.map((value) => ({ value, label: value })), hash.tz ?? browserZone)
  state.table = hash.table ?? ''
  state.ticker = hash.ticker ?? ''
  state.from = elements.from.value = hash.from ?? ''
  state.to = elements.to.value = hash.to ?? ''
  state.at = hash.at ?? null

  elements.table.addEventListener('change', () => {
    state.table = elements.table.value
    state.ticker = ''
    state.from = state.to = ''
    state.at = null
    loadTickers()
  })
  elements.ticker.addEventListener('change', () => {
    state.ticker = elements.ticker.value
    state.at = null
    const chosen = state.tickers.find((entry) => entry.ticker === state.ticker)
    if (chosen) {
      const range = spanToRange(chosen.from, chosen.to, state.tz)
      state.from = elements.from.value = range.from
      state.to = elements.to.value = range.to
    }
    scheduled()
  })
  for (const key of ['from', 'to']) {
    elements[key].addEventListener('change', () => {
      state[key] = elements[key].value
      state.at = null
      scheduled()
    })
  }
  elements.tz.addEventListener('change', () => {
    state.tz = elements.tz.value
    scheduled()
  })
  elements.interval.addEventListener('change', () => {
    state.interval = elements.interval.value
    state.at = null
    scheduled()
  })

  // The chart follows its box.
  if (typeof window.ResizeObserver === 'function') {
    new window.ResizeObserver(() => {
      if (chart) draw()
    }).observe(elements.chart.parentElement)
  } else {
    window.addEventListener('resize', () => draw())
  }

  clearSelection()
  downloads()
  draw()
  status('Loading tables…', 'loading')
  loadTables()
}

if (globalThis.document?.getElementById?.('chart')) start(globalThis.document)
