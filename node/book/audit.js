// The point summary and the audit tables of one bucket.
//
// The instant reader (`instantParts`, `instantNanos`, `instantText`),
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
 * ISO-8601 date-time text: the date, `T` or a space, the time to the minute
 * or the second with up to nine fraction digits, then an optional `Z` or
 * offset and any RFC 9557 bracketed suffixes (`[Europe/Zurich]`), which is how
 * the service writes an instant outside UTC.
 */
const ISO_DATETIME = /^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2})(?::(\d{2})(?:[.,](\d{1,9}))?)?(?:([Zz])|([+-])(\d{2}):?(\d{2}))?((?:\[[^\]]*\])*)$/

/** The wall clock `millis` shows in `zone`, to the second, as the epoch milliseconds of that wall clock read in UTC. */
function wallOf(millis, zone) {
  const fields = {}
  for (const part of partFormatter(zone).formatToParts(new Date(millis))) fields[part.type] = part.value
  const date = new Date(0)
  date.setUTCFullYear(Number(fields.year), Number(fields.month) - 1, Number(fields.day))
  date.setUTCHours(fields.hour === '24' ? 0 : Number(fields.hour), Number(fields.minute), Number(fields.second), 0)
  return date.getTime()
}

/**
 * The instant a whole-second wall clock (epoch milliseconds of it read in
 * UTC) names in `zone`: the first of the two where saving time repeats it,
 * and the one at the offset before the jump where saving time skips it -
 * a skipped 02:30 is 03:30.
 */
function wallInstant(wall, zone) {
  const before = wallOf(wall - 86_400_000, zone) - (wall - 86_400_000)
  const after = wallOf(wall + 86_400_000, zone) - (wall + 86_400_000)
  const standing = [before, after].map((offset) => wall - offset).filter((instant) => wallOf(instant, zone) === wall)
  return standing.length > 0 ? Math.min(...standing) : wall - before
}

/** The zone a bracketed suffix names (`[Europe/Zurich]`, `[!UTC]`), `null` for none; a `key=value` annotation names none. */
function bracketedZone(suffixes) {
  for (const [, inside] of suffixes.matchAll(/\[([^\]]*)\]/g)) {
    const name = inside.replace(/^!/, '')
    if (name !== '' && !name.includes('=')) return name
  }
  return null
}

function isoParts(text, zone) {
  const match = ISO_DATETIME.exec(text)
  if (match === null) return null
  const [, year, month, day, hour, minute, second = '0', fraction = '', utc, sign, offsetHours, offsetMinutes, suffixes] = match
  const date = new Date(0)
  date.setUTCFullYear(Number(year), Number(month) - 1, Number(day))
  date.setUTCHours(Number(hour), Number(minute), Number(second), 0)
  const stated = [date.getUTCFullYear(), date.getUTCMonth() + 1, date.getUTCDate(), date.getUTCHours(), date.getUTCMinutes(), date.getUTCSeconds()]
  if (stated.join() !== [year, month, day, hour, minute, second].map(Number).join()) return null
  let millis = date.getTime()
  if (sign !== undefined) {
    if (Number(offsetHours) > 23 || Number(offsetMinutes) > 59) return null
    const offset = (Number(offsetHours) * 60 + Number(offsetMinutes)) * 60_000
    millis += sign === '-' ? offset : -offset
  } else if (utc === undefined) {
    millis = wallInstant(millis, bracketedZone(suffixes) ?? zone)
  }
  const digits = fraction.padEnd(9, '0')
  return { millis: millis + Number(digits.slice(0, 3)), nanos: Number(digits.slice(3)) }
}

/**
 * An instant as `millis` since the epoch and the nanoseconds past that
 * millisecond - the one reader of every instant the display holds: a
 * `bigint` or digit string of nanoseconds, a number or a `Date` of
 * milliseconds, or ISO-8601 text as the service writes it - `Z` or an
 * offset, up to nine fraction digits, a bracketed zone after it
 * (`2026-08-14T14:46:00.000000000+02:00[Europe/Zurich]`). Text stating no
 * offset is a wall clock - to the minute or the second, as a
 * `datetime-local` input states it - read in the zone its brackets name,
 * else in `zone`. `null` when the value is none of these.
 */
export function instantParts(value, zone = 'UTC') {
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
  return isoParts(text, zone)
}

/** An instant as nanoseconds since the epoch, read as `instantParts` reads it; `null` when it does not read. */
export function instantNanos(value, zone = 'UTC') {
  const parts = instantParts(value, zone)
  return parts === null ? null : BigInt(Math.floor(parts.millis)) * NANOS_PER_MILLI + BigInt(parts.nanos)
}

/**
 * An instant as the one text the display sends and keeps in its hash:
 * ISO-8601 in UTC with `Z`, the fraction written only as far as it is not
 * zero (`2026-08-14T12:46:59.999999999Z`, `2026-08-14T12:46:00Z`); the
 * empty string when the value does not read.
 */
export function instantText(value, zone = 'UTC') {
  const nanos = instantNanos(value, zone)
  if (nanos === null) return ''
  const text = formatInstant(nanos, 'UTC', { fraction: 9, separator: 'T' })
  return `${text.replace(/\.?0+$/, '')}Z`
}

/**
 * An instant as `YYYY-MM-DD HH:MM:SS.fff` in an IANA zone: `fraction` digits
 * after the second (3 by default, up to 9 when the value carries them, 0 for
 * none), `separator` between the date and the time. The value is read by
 * `instantParts`, a wall clock in `zone`. An unreadable value is the empty
 * string; an unknown zone reads as UTC.
 */
export function formatInstant(value, zone = 'UTC', { fraction = 3, separator = ' ' } = {}) {
  const parts = instantParts(value, zone)
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

/**
 * The audit download of a range as `{ href, label, type }`. The route names
 * the file - its `Content-Disposition` states
 * `audit-<ticker>-<from>-<to><suffix>` with the instants in UTC - so the link
 * carries no name of its own.
 */
export function downloadLink(base, query, suffix = '.csv') {
  if (!AUDIT_SUFFIXES.includes(suffix)) {
    throw new RangeError(`audit suffix must be one of ${AUDIT_SUFFIXES.join(', ')}, got ${JSON.stringify(suffix)}`)
  }
  return { href: auditUrl(base, query, suffix), label: AUDIT_LABELS[suffix], type: AUDIT_TYPES[suffix] }
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

function limitsTable(document, title, limits, side, emptyText) {
  const holder = make(document, 'div', `limits limits-${side}`)
  holder.append(make(document, 'h3', 'limits-title', title))
  const rows = Array.isArray(limits) ? limits.slice(0, 5) : []
  if (rows.length === 0) {
    holder.append(make(document, 'p', 'empty', emptyText))
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
 * the alive, delta and events counts and the top five limits of each side -
 * under `node`; `null` renders the empty state, saying `emptyText`. A book
 * the service could not rebuild (`complete` false) is a delta book: its
 * touch stands, and it is flagged, with no entry and no limit counted.
 */
export function renderSummary(node, book, { zone = 'UTC', emptyText = 'Select a bucket - click a candle, or focus the chart and press Enter - to read its last book.' } = {}) {
  const document = node.ownerDocument
  node.replaceChildren()
  if (!book) {
    node.append(make(document, 'p', 'empty', emptyText))
    return
  }
  const head = make(document, 'div', 'summary-head')
  head.append(make(document, 'span', 'summary-name', [book.ticker ?? book.isincode, book.crosscode].filter(Boolean).join(' · ')))
  head.append(make(document, 'span', 'summary-when', formatInstant(book.currunix, zone, { fraction: 9 })))
  const flags = make(document, 'span', 'summary-flags')
  if (book.iscrossed) flags.append(make(document, 'span', 'chip chip-warn', 'Crossed'))
  else if (book.islocked) flags.append(make(document, 'span', 'chip chip-warn', 'Locked'))
  else if (isEmpty(book.bestbid) || isEmpty(book.bestask)) flags.append(make(document, 'span', 'chip', 'One-sided'))
  else flags.append(make(document, 'span', 'chip chip-ok', 'Two-sided'))
  const complete = book.complete !== false
  if (!complete) flags.append(make(document, 'span', 'chip chip-warn', 'Delta book'))
  head.append(flags)
  node.append(head)

  const stats = make(document, 'div', 'stats')
  stats.append(stat(document, 'Best bid', formatDecimal(book.bestbid), isEmpty(book.bidqty) ? null : `qty ${formatDecimal(book.bidqty)}`, 'bid'))
  stats.append(stat(document, 'Best ask', formatDecimal(book.bestask), isEmpty(book.askqty) ? null : `qty ${formatDecimal(book.askqty)}`, 'ask'))
  stats.append(stat(document, 'Spread', formatDecimal(book.spread), null, 'spread'))
  stats.append(stat(document, 'Mid', formatDecimal(book.midpoint), null, 'mid'))
  stats.append(stat(document, 'Imbalance', formatDecimal(book.imbalance, { fraction: 4 }), 'bid depth less ask, over both'))
  stats.append(stat(document, 'Alive', complete ? String(book.alive ?? 0) : '', complete ? 'entries' : 'not rebuilt'))
  stats.append(stat(document, 'Delta', String(book.delta ?? 0), 'since the previous book'))
  stats.append(stat(document, 'Events', String(book.events ?? 0), 'recorded at this instant'))
  node.append(stats)

  const sides = make(document, 'div', 'limit-sides')
  const noLimits = complete ? 'No limits on this side.' : 'Not rebuilt: this is a delta book.'
  sides.append(limitsTable(document, 'Top bid limits', book.bidlimits, 'bid', noLimits))
  sides.append(limitsTable(document, 'Top ask limits', book.asklimits, 'ask', noLimits))
  node.append(sides)
}

function sideLabel(side) {
  return side === 'ask' ? 'Ask' : side === 'bid' ? 'Bid' : 'All'
}

/**
 * Render the audit rows of one side as a sortable table under `node`: a
 * heading with the count, one column per `EVENT_COLUMNS`, a click on a
 * heading sorting by it (ascending, then descending) and leaving the focus
 * on that heading in the table rendered again. `rows` `null` is the
 * unselected state and `[]` a bucket with no event on that side; `truncated`
 * says the service cut the list at its bound. The sort is kept on the node
 * across renders unless `sort` states one; `focus` names the column whose
 * heading takes the focus once rendered. Answers `{ sort, count }`.
 */
export function renderEvents(node, rows, side, options = {}) {
  const { zone = 'UTC', columns = EVENT_COLUMNS, truncated = false, onSort, focus = null } = options
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
  let focused = null
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
      renderEvents(node, rows, side, { ...options, sort: next, focus: column.key })
      onSort?.(next)
    })
    if (column.key === focus) focused = button
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
  // The heading clicked was replaced with the table: its twin takes the focus, not <body>.
  focused?.focus()
  return { sort, count }
}
