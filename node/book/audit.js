// The point summary and the audit tables of one bucket.
//
// `formatInstant`, `formatDecimal`, `sortRows` and `downloadLink` are pure
// and run under Node; `renderSummary` and `renderEvents` build DOM under a
// node they are handed and own everything below it. Every value the service
// answers is text - decimals as their exact text, instants as ISO-8601 in the
// zone asked, uuids canonical - and lands in the page through `textContent`,
// never markup.

import { AUDIT_SUFFIXES, auditUrl } from './api.js'

/** The columns of an audit table, in order: the key in a row, its heading, how it renders. */
export const EVENT_COLUMNS = Object.freeze([
  { key: 'currunix', label: 'Instant', kind: 'instant' },
  { key: 'role', label: 'Role', kind: 'text' },
  { key: 'marketdatakind', label: 'Kind', kind: 'text' },
  { key: 'side', label: 'Side', kind: 'text' },
  { key: 'price', label: 'Price', kind: 'decimal' },
  { key: 'quantity', label: 'Quantity', kind: 'decimal' },
  { key: 'state', label: 'State', kind: 'text' },
  { key: 'crosscode', label: 'Cross code', kind: 'text' },
  { key: 'curruuid', label: 'UUID', kind: 'uuid' },
  { key: 'prevuuid', label: 'Previous', kind: 'uuid' },
].map((column) => Object.freeze(column)))

/** The media type each audit suffix is served as. */
export const AUDIT_TYPES = Object.freeze({ '.csv': 'text/csv', '.csv.gz': 'application/gzip', '.csv.zst': 'application/zstd' })

const AUDIT_LABELS = Object.freeze({ '.csv': 'CSV', '.csv.gz': 'CSV · gzip', '.csv.zst': 'CSV · zstd' })

const NANOS_PER_MILLI = 1_000_000n

const partFormatters = new Map()

function partFormatter(zone) {
  let cached = partFormatters.get(zone)
  if (cached === undefined) {
    const options = {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hourCycle: 'h23',
    }
    try {
      cached = new Intl.DateTimeFormat('en-US', { timeZone: zone, ...options })
    } catch {
      cached = new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', ...options })
    }
    partFormatters.set(zone, cached)
  }
  return cached
}

/**
 * An instant as `millis` since the epoch and the nanoseconds past that
 * millisecond, from a `bigint` or digit string of nanoseconds, a number or a
 * `Date` of milliseconds, or ISO-8601 text; `null` when it is none of them.
 */
export function instantParts(value) {
  if (typeof value === 'bigint') {
    const millis = value / NANOS_PER_MILLI
    const rest = value - millis * NANOS_PER_MILLI
    return rest < 0n ? { millis: Number(millis) - 1, nanos: Number(rest + NANOS_PER_MILLI) } : { millis: Number(millis), nanos: Number(rest) }
  }
  if (typeof value === 'number') return Number.isFinite(value) ? { millis: value, nanos: 0 } : null
  if (value instanceof Date) return Number.isFinite(value.getTime()) ? { millis: value.getTime(), nanos: 0 } : null
  if (typeof value !== 'string') return null
  const text = value.trim()
  if (/^-?\d+$/.test(text)) return instantParts(BigInt(text))
  const millis = Date.parse(text)
  if (!Number.isFinite(millis)) return null
  const fraction = /[T ]\d{2}:\d{2}:\d{2}\.(\d+)/.exec(text)?.[1] ?? ''
  const nanos = fraction.length > 3 ? Number(fraction.slice(3).padEnd(6, '0').slice(0, 6)) : 0
  return { millis, nanos }
}

/**
 * An instant as `YYYY-MM-DD HH:MM:SS.fff` in an IANA zone: `fraction` digits
 * after the second (3 by default, up to 9 when the value carries them, 0 for
 * none), `separator` between the date and the time. An unreadable value is
 * the empty string; an unknown zone reads as UTC.
 */
export function formatInstant(value, zone = 'UTC', { fraction = 3, separator = ' ' } = {}) {
  const parts = instantParts(value)
  if (parts === null) return ''
  const date = new Date(parts.millis)
  const fields = {}
  for (const part of partFormatter(zone).formatToParts(date)) fields[part.type] = part.value
  const hour = fields.hour === '24' ? '00' : fields.hour
  let text = `${fields.year}-${fields.month}-${fields.day}${separator}${hour}:${fields.minute}:${fields.second}`
  const digits = Math.max(0, Math.min(9, Math.trunc(fraction)))
  if (digits > 0) {
    const millis = ((parts.millis % 1000) + 1000) % 1000
    const all = `${String(millis).padStart(3, '0')}${String(parts.nanos).padStart(6, '0')}`
    text += `.${all.slice(0, digits)}`
  }
  return text
}

/**
 * A decimal's text for display: trailing zeros of the fraction dropped, the
 * fraction cut to `fraction` digits, `-0` read as `0`; `null` and
 * `undefined` are the empty string, and text that is not a plain decimal is
 * left as it is.
 */
export function formatDecimal(value, { fraction = 8 } = {}) {
  if (value === null || value === undefined) return ''
  const text = typeof value === 'number' ? String(value) : String(value).trim()
  const match = /^([+-]?)(\d+)(?:\.(\d+))?$/.exec(text)
  if (match === null) return text
  const sign = match[1] === '-' ? '-' : ''
  const whole = match[2].replace(/^0+(?=\d)/, '')
  let part = (match[3] ?? '').slice(0, Math.max(0, fraction)).replace(/0+$/, '')
  const out = part.length === 0 ? whole : `${whole}.${part}`
  return sign && !/^0(\.0*)?$/.test(out) ? `${sign}${out}` : out
}

function isNumeric(text) {
  return typeof text === 'string' && /^[+-]?\d+(\.\d+)?$/.test(text)
}

function isEmpty(value) {
  return value === null || value === undefined || value === ''
}

/** Compare two cells: absent last, decimals by value, everything else as text. */
export function compareCells(left, right) {
  if (isEmpty(left) && isEmpty(right)) return 0
  if (isEmpty(left)) return 1
  if (isEmpty(right)) return -1
  if (typeof left === 'number' && typeof right === 'number') return left - right
  if (typeof left === 'bigint' && typeof right === 'bigint') return left < right ? -1 : left > right ? 1 : 0
  if (isNumeric(left) && isNumeric(right)) {
    const a = Number(left)
    const b = Number(right)
    if (a !== b) return a < b ? -1 : 1
  }
  return String(left).localeCompare(String(right), 'en', { numeric: true })
}

/** A sorted copy of `rows` by one column, `asc` or `desc`, stable, absent values last either way. */
export function sortRows(rows, column, direction = 'asc') {
  const sign = direction === 'desc' ? -1 : 1
  return rows
    .map((row, index) => ({ row, index }))
    .sort((a, b) => {
      const left = a.row?.[column]
      const right = b.row?.[column]
      if (isEmpty(left) !== isEmpty(right)) return isEmpty(left) ? 1 : -1
      const order = compareCells(left, right)
      return order === 0 ? a.index - b.index : order * sign
    })
    .map(({ row }) => row)
}

function safeName(text) {
  return String(text ?? '').replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '')
}

/**
 * The audit download of a range as `{ href, filename, label, type }`, the
 * file named `audit-<ticker>-<from>-<to><suffix>`.
 */
export function downloadLink(base, query, suffix = '.csv') {
  if (!AUDIT_SUFFIXES.includes(suffix)) {
    throw new RangeError(`audit suffix must be one of ${AUDIT_SUFFIXES.join(', ')}, got ${JSON.stringify(suffix)}`)
  }
  const { ticker, from, to } = query ?? {}
  return {
    href: auditUrl(base, query, suffix),
    filename: `audit-${safeName(ticker)}-${safeName(from)}-${safeName(to)}${suffix}`,
    label: AUDIT_LABELS[suffix],
    type: AUDIT_TYPES[suffix],
  }
}

function make(document, tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

function cellText(column, value, zone) {
  if (value === null || value === undefined) return ''
  switch (column.kind) {
    case 'instant':
      return formatInstant(value, zone)
    case 'decimal':
      return formatDecimal(value)
    default:
      return typeof value === 'object' ? JSON.stringify(value) : String(value)
  }
}

function stat(document, label, value, note, key) {
  const tile = make(document, 'div', 'stat')
  if (key) tile.classList.add(`stat-${key}`)
  tile.append(make(document, 'span', 'stat-label', label))
  tile.append(make(document, 'strong', 'stat-value', value === '' ? '–' : value))
  if (note) tile.append(make(document, 'span', 'stat-note', note))
  return tile
}

function limitsTable(document, title, limits, side) {
  const holder = make(document, 'div', `limits limits-${side}`)
  holder.append(make(document, 'h3', 'limits-title', title))
  const rows = Array.isArray(limits) ? limits.slice(0, 5) : []
  if (rows.length === 0) {
    holder.append(make(document, 'p', 'empty', 'No limits on this side.'))
    return holder
  }
  const table = make(document, 'table', 'grid grid-limits')
  const head = make(document, 'thead')
  const heading = make(document, 'tr')
  for (const label of ['#', 'Price', 'Quantity', 'Entries', 'Tradable']) {
    const cell = make(document, 'th', null, label)
    cell.setAttribute('scope', 'col')
    heading.append(cell)
  }
  head.append(heading)
  const body = make(document, 'tbody')
  rows.forEach((limit, index) => {
    const line = make(document, 'tr')
    line.append(make(document, 'td', 'num', String(index + 1)))
    line.append(make(document, 'td', 'num', limit.price === null || limit.price === undefined ? 'market' : formatDecimal(limit.price)))
    line.append(make(document, 'td', 'num', formatDecimal(limit.quantity)))
    line.append(make(document, 'td', 'num', String(Array.isArray(limit.uuids) ? limit.uuids.length : 0)))
    line.append(make(document, 'td', null, limit.tradable === false ? 'no' : 'yes'))
    body.append(line)
  })
  table.append(head, body)
  holder.append(table)
  return holder
}

/**
 * Render the point summary of a book - the two bests and their quantities,
 * the spread, the mid, the imbalance, whether the book is locked or crossed,
 * the alive, delta and execution counts and the top five limits of each side
 * - under `node`; `null` renders the empty state, saying `emptyText`.
 */
export function renderSummary(node, book, { zone = 'UTC', emptyText = 'Select a bucket - click a candle, or focus the chart and press Enter - to read its last book.' } = {}) {
  const document = node.ownerDocument
  node.replaceChildren()
  if (!book) {
    node.append(make(document, 'p', 'empty', emptyText))
    return
  }
  const head = make(document, 'div', 'summary-head')
  head.append(make(document, 'span', 'summary-name', `${book.ticker ?? ''}${book.crosscode ? ` · ${book.crosscode}` : ''}`))
  head.append(make(document, 'span', 'summary-when', formatInstant(book.currunix, zone, { fraction: 9 })))
  const flags = make(document, 'span', 'summary-flags')
  if (book.iscrossed) flags.append(make(document, 'span', 'chip chip-warn', 'Crossed'))
  else if (book.islocked) flags.append(make(document, 'span', 'chip chip-warn', 'Locked'))
  else if (isEmpty(book.bestbid) || isEmpty(book.bestask)) flags.append(make(document, 'span', 'chip', 'One-sided'))
  else flags.append(make(document, 'span', 'chip chip-ok', 'Two-sided'))
  head.append(flags)
  node.append(head)

  const stats = make(document, 'div', 'stats')
  stats.append(stat(document, 'Best bid', formatDecimal(book.bestbid), isEmpty(book.bidqty) ? null : `qty ${formatDecimal(book.bidqty)}`, 'bid'))
  stats.append(stat(document, 'Best ask', formatDecimal(book.bestask), isEmpty(book.askqty) ? null : `qty ${formatDecimal(book.askqty)}`, 'ask'))
  stats.append(stat(document, 'Spread', formatDecimal(book.spread), null, 'spread'))
  stats.append(stat(document, 'Mid', formatDecimal(book.midpoint), null, 'mid'))
  stats.append(stat(document, 'Imbalance', formatDecimal(book.imbalance, { fraction: 4 }), 'bid depth less ask, over both'))
  stats.append(stat(document, 'Alive', String(book.alive ?? 0), 'entries'))
  stats.append(stat(document, 'Deltas', String(book.deltas ?? 0), 'since the previous book'))
  stats.append(stat(document, 'Executions', String(book.executions ?? 0), 'in this book'))
  node.append(stats)

  const sides = make(document, 'div', 'limit-sides')
  sides.append(limitsTable(document, 'Top bid limits', book.bidlimits, 'bid'))
  sides.append(limitsTable(document, 'Top ask limits', book.asklimits, 'ask'))
  node.append(sides)
}

function sideLabel(side) {
  return side === 'ask' ? 'Ask' : side === 'bid' ? 'Bid' : 'All'
}

/**
 * Render the audit rows of one side as a sortable table under `node`: a
 * heading with the count, one column per `EVENT_COLUMNS`, a click on a
 * heading sorting by it (ascending, then descending). `rows` `null` is the
 * unselected state and `[]` a bucket with no event on that side; `truncated`
 * says the service cut the list at its bound. The sort is kept on the node
 * across renders unless `sort` states one. Answers `{ sort, count }`.
 */
export function renderEvents(node, rows, side, options = {}) {
  const { zone = 'UTC', columns = EVENT_COLUMNS, truncated = false, onSort } = options
  const document = node.ownerDocument
  const stored = node.dataset.sortColumn ? { column: node.dataset.sortColumn, direction: node.dataset.sortDirection === 'desc' ? 'desc' : 'asc' } : null
  const sort = options.sort ?? stored
  if (sort) {
    node.dataset.sortColumn = sort.column
    node.dataset.sortDirection = sort.direction
  }
  node.replaceChildren()
  const list = Array.isArray(rows) ? rows : null
  const count = list?.length ?? 0

  const head = make(document, 'div', 'events-head')
  head.append(make(document, 'h3', `events-title events-${side}`, `${sideLabel(side)} events`))
  const summary = list === null ? 'no bucket selected' : `${count} row${count === 1 ? '' : 's'}${truncated ? ', truncated at the service bound' : ''}`
  head.append(make(document, 'span', 'events-count', summary))
  node.append(head)

  if (list === null) {
    node.append(make(document, 'p', 'empty', `Select a bucket to list its ${sideLabel(side).toLowerCase()} events.`))
    return { sort, count: 0 }
  }
  if (count === 0) {
    node.append(make(document, 'p', 'empty', `No ${sideLabel(side).toLowerCase()} event in this bucket.`))
    return { sort, count: 0 }
  }

  const ordered = sort ? sortRows(list, sort.column, sort.direction) : list
  const scroll = make(document, 'div', 'scroll')
  const table = make(document, 'table', 'grid grid-events')
  const thead = make(document, 'thead')
  const heading = make(document, 'tr')
  for (const column of columns) {
    const cell = make(document, 'th', column.kind === 'decimal' || column.kind === 'instant' ? 'num' : null)
    cell.setAttribute('scope', 'col')
    const active = sort?.column === column.key
    cell.setAttribute('aria-sort', active ? (sort.direction === 'desc' ? 'descending' : 'ascending') : 'none')
    const button = make(document, 'button', 'sort')
    button.type = 'button'
    button.dataset.column = column.key
    button.append(make(document, 'span', null, column.label))
    button.append(make(document, 'span', 'sort-mark', active ? (sort.direction === 'desc' ? '▼' : '▲') : '↕'))
    button.addEventListener('click', () => {
      const next = { column: column.key, direction: active && sort.direction === 'asc' ? 'desc' : 'asc' }
      renderEvents(node, rows, side, { ...options, sort: next })
      onSort?.(next)
    })
    cell.append(button)
    heading.append(cell)
  }
  thead.append(heading)
  const tbody = make(document, 'tbody')
  for (const row of ordered) {
    const line = make(document, 'tr')
    for (const column of columns) {
      const value = row?.[column.key]
      const text = cellText(column, value, zone)
      const cell = make(document, 'td', column.kind === 'decimal' || column.kind === 'instant' ? 'num' : null)
      if (column.kind === 'uuid' && text) {
        const code = make(document, 'code', 'uuid', text)
        code.title = text
        cell.append(code)
      } else {
        cell.textContent = text
      }
      line.append(cell)
    }
    tbody.append(line)
  }
  table.append(thead, tbody)
  scroll.append(table)
  node.append(scroll)
  return { sort, count }
}
