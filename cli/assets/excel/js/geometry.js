// Sheet geometry: sizes, positions and hit testing over 1,048,576 rows by
// 16,384 columns. Nothing here is proportional to the sheet: an axis holds the
// default size and the runs that differ from it, and every position <-> index
// question is a binary search over those runs.

export const MAX_ROWS = 1048576
export const MAX_COLUMNS = 16384
export const TILE_ROWS = 64
export const TILE_COLUMNS = 32

// ECMA-376 §18.3.1.13: a width is character units of the maximum digit width,
// padding included; the layout states that digit width. Every width the
// service states (`defaults.columnWidth`, column runs) is the file's
// `<col width>` in those units.
export const columnWidthPx = (width, digit) =>
  Math.trunc(((256 * width + Math.trunc(128 / digit)) / 256) * digit)

// The inverse, as a width the file would store for a pixel size.
export const columnWidthFromPx = (px, digit) => Math.trunc((px / digit) * 256) / 256

// Excel's default column when a sheet states none: 64 px at digit width 7.
export const DEFAULT_COLUMN_WIDTH = 9.140625

// Row heights are points; the grid is laid out at 96 CSS pixels per inch.
export const rowHeightPx = (points) => Math.round((points * 96) / 72)
export const pointsFromPx = (px) => (px * 72) / 96

export const columnName = (column) => {
  let name = ''
  let n = column + 1
  while (n > 0) {
    const rest = (n - 1) % 26
    name = String.fromCharCode(65 + rest) + name
    n = Math.trunc((n - 1) / 26)
  }
  return name
}

export const refName = (row, column) => columnName(column) + (row + 1)

const REF = /^\$?([A-Za-z]{1,3})\$?([0-9]{1,7})$/
const COLUMN = /^\$?([A-Za-z]{1,3})$/
const ROW = /^\$?([0-9]{1,7})$/

const columnIndex = (letters) => {
  let n = 0
  for (const ch of letters.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64)
  return n - 1
}

export const parseRef = (text) => {
  const match = REF.exec(text.trim())
  if (!match) return null
  const column = columnIndex(match[1])
  const row = Number(match[2]) - 1
  if (column < 0 || column >= MAX_COLUMNS || row < 0 || row >= MAX_ROWS) return null
  return { row, column }
}

// `A1`, `A1:D5`, `A:C` and `2:4`; the answer is normalized and inclusive.
export const parseRange = (text) => {
  if (!text) return null
  const [first, second = first] = text.split(':')
  const a = parseRef(first)
  const b = parseRef(second)
  if (a && b) return normalize(a.row, a.column, b.row, b.column)
  const ca = COLUMN.exec(first.trim())
  const cb = COLUMN.exec(second.trim())
  if (ca && cb) return normalize(0, columnIndex(ca[1]), MAX_ROWS - 1, columnIndex(cb[1]))
  const ra = ROW.exec(first.trim())
  const rb = ROW.exec(second.trim())
  if (ra && rb) return normalize(Number(ra[1]) - 1, 0, Number(rb[1]) - 1, MAX_COLUMNS - 1)
  return null
}

export const normalize = (r0, c0, r1, c1) => ({
  r0: Math.min(r0, r1),
  c0: Math.min(c0, c1),
  r1: Math.max(r0, r1),
  c1: Math.max(c0, c1)
})

export const rangeName = (range) =>
  range.r0 === range.r1 && range.c0 === range.c1
    ? refName(range.r0, range.c0)
    : refName(range.r0, range.c0) + ':' + refName(range.r1, range.c1)

export const sameRange = (a, b) =>
  a.r0 === b.r0 && a.c0 === b.c0 && a.r1 === b.r1 && a.c1 === b.c1

export const intersects = (a, b) =>
  a.r0 <= b.r1 && b.r0 <= a.r1 && a.c0 <= b.c1 && b.c0 <= a.c1

export const contains = (range, row, column) =>
  row >= range.r0 && row <= range.r1 && column >= range.c0 && column <= range.c1

// What a drag of the fill handle from `source` to (row, column) asks for:
// `target`, the source grown down, up, right or left to that cell along
// the axis the pointer left it farther on (§4.3: one direction); or, with
// the pointer back inside the source, the part kept, and `clear`, the rows
// or columns left behind (Excel clears them). `clear` is null otherwise.
export const fillTarget = (source, row, column) => {
  const down = Math.max(0, row - source.r1)
  const up = Math.max(0, source.r0 - row)
  const right = Math.max(0, column - source.c1)
  const left = Math.max(0, source.c0 - column)
  const vertical = Math.max(down, up)
  const across = Math.max(right, left)
  if (vertical || across) {
    if (vertical >= across) return { target: down ? { ...source, r1: row } : { ...source, r0: row }, clear: null }
    return { target: right ? { ...source, c1: column } : { ...source, c0: column }, clear: null }
  }
  const rowsBack = source.r1 - row
  const columnsBack = source.c1 - column
  if (rowsBack === 0 && columnsBack === 0) return { target: source, clear: null }
  if (rowsBack >= columnsBack) return { target: { ...source, r1: row }, clear: { ...source, r0: row + 1 } }
  return { target: { ...source, c1: column }, clear: { ...source, c0: column + 1 } }
}

// One axis: `count` indices of `size` pixels, except the sorted,
// non-overlapping runs, each `{ first, last, size, style }` (size in pixels,
// 0 when hidden). `before[j]` is the pixel delta the runs ahead of run j add.
export class Axis {
  constructor (count, size, runs) {
    this.count = count
    this.size = size
    this.runs = runs
    this.before = new Float64Array(runs.length + 1)
    let delta = 0
    for (let j = 0; j < runs.length; j++) {
      this.before[j] = delta
      const run = runs[j]
      delta += (run.last - run.first + 1) * (run.size - size)
    }
    this.before[runs.length] = delta
    this.total = this.start(count)
  }

  // The last run whose first index is at or before `index`, or -1.
  runBefore (index) {
    let lo = 0
    let hi = this.runs.length - 1
    let found = -1
    while (lo <= hi) {
      const mid = (lo + hi) >> 1
      if (this.runs[mid].first <= index) {
        found = mid
        lo = mid + 1
      } else hi = mid - 1
    }
    return found
  }

  runAt (index) {
    const j = this.runBefore(index)
    return j >= 0 && this.runs[j].last >= index ? this.runs[j] : null
  }

  sizeOf (index) {
    const run = this.runAt(index)
    return run ? run.size : this.size
  }

  styleOf (index) {
    const run = this.runAt(index)
    return run ? run.style : null
  }

  isHidden (index) {
    return this.sizeOf(index) === 0
  }

  // The pixel offset where `index` begins; `start(count)` is the total.
  start (index) {
    const j = this.runBefore(index - 1)
    if (j < 0) return index * this.size
    const run = this.runs[j]
    const covered = Math.min(index, run.last + 1) - run.first
    return index * this.size + this.before[j] + covered * (run.size - this.size)
  }

  // The index whose extent holds `position`, skipping hidden indices; clamped.
  indexAt (position) {
    if (position <= 0) return Math.max(0, this.firstVisible(0, 1))
    let lo = 0
    let hi = this.runs.length - 1
    let found = -1
    while (lo <= hi) {
      const mid = (lo + hi) >> 1
      const run = this.runs[mid]
      if (run.first * this.size + this.before[mid] <= position) {
        found = mid
        lo = mid + 1
      } else hi = mid - 1
    }
    let index
    if (found < 0) index = Math.floor(position / this.size)
    else {
      const run = this.runs[found]
      const begin = run.first * this.size + this.before[found]
      const end = begin + (run.last - run.first + 1) * run.size
      if (position < end) index = run.first + Math.floor((position - begin) / run.size)
      else index = run.last + 1 + Math.floor((position - end) / this.size)
    }
    index = Math.min(Math.max(index, 0), this.count - 1)
    // Past the last shown index (a hidden tail), the last shown one.
    return this.isHidden(index) ? Math.max(0, this.firstVisible(index, -1)) : index
  }

  // The last index shown, where scrolling stops.
  lastVisible () {
    return Math.max(0, this.firstVisible(this.count - 1, -1))
  }

  // From `index` (inclusive) in `step` direction, the first shown index, or -1.
  firstVisible (index, step) {
    let at = index
    while (at >= 0 && at < this.count) {
      const run = this.runAt(at)
      if (!run || run.size > 0) return at
      at = step > 0 ? run.last + 1 : run.first - 1
    }
    return -1
  }

  // The shown index `count` shown indices away, stopping at the axis edge.
  advance (index, count) {
    const step = count < 0 ? -1 : 1
    let at = index
    for (let moved = 0; moved < Math.abs(count); moved++) {
      const next = this.firstVisible(at + step, step)
      if (next < 0) break
      at = next
    }
    return at
  }
}

const runsOf = (list, count, defaultSize, toPx) => {
  const runs = []
  for (const [first, last, size, hidden, style] of list || []) {
    if (first > last || first >= count) continue
    runs.push({
      first,
      last: Math.min(last, count - 1),
      size: hidden ? 0 : size === null || size === undefined ? defaultSize : toPx(size),
      style: style === undefined ? null : style
    })
  }
  runs.sort((a, b) => a.first - b.first)
  return runs
}

// Merged ranges bucketed by row tile, so a lookup reads one bucket.
export class Merges {
  constructor (texts) {
    this.list = []
    this.buckets = new Map()
    for (const text of texts || []) {
      const range = parseRange(text)
      if (!range) continue
      this.list.push(range)
      for (let band = range.r0 >> 6; band <= range.r1 >> 6; band++) {
        let bucket = this.buckets.get(band)
        if (!bucket) this.buckets.set(band, (bucket = []))
        bucket.push(range)
      }
    }
  }

  at (row, column) {
    const bucket = this.buckets.get(row >> 6)
    if (!bucket) return null
    for (const range of bucket) if (contains(range, row, column)) return range
    return null
  }

  within (area) {
    const found = new Set()
    for (let band = area.r0 >> 6; band <= area.r1 >> 6; band++) {
      const bucket = this.buckets.get(band)
      if (bucket) for (const range of bucket) if (intersects(range, area)) found.add(range)
    }
    return [...found]
  }

  // The smallest range holding `area` and every merge it touches.
  expand (area) {
    let range = { ...area }
    for (let changed = true; changed;) {
      changed = false
      for (const merge of this.within(range)) {
        const next = normalize(
          Math.min(range.r0, merge.r0), Math.min(range.c0, merge.c0),
          Math.max(range.r1, merge.r1), Math.max(range.c1, merge.c1))
        if (!sameRange(next, range)) {
          range = next
          changed = true
        }
      }
    }
    return range
  }
}

// One sheet's layout at one zoom: both axes, the frozen split, the merges and
// the used range. `version` changes whenever a pixel position may have.
let versions = 0

export class Geometry {
  constructor (layout, zoom = 1) {
    const defaults = layout.defaults || {}
    this.digit = defaults.maxDigitWidth || 7
    this.zoom = zoom
    const column = (width) => Math.max(0, Math.round(columnWidthPx(width, this.digit) * zoom))
    const row = (points) => Math.max(0, Math.round(rowHeightPx(points) * zoom))
    const columnSize = column(defaults.columnWidth ?? DEFAULT_COLUMN_WIDTH) || 1
    const rowSize = row(defaults.rowHeight ?? 15) || 1
    this.columns = new Axis(MAX_COLUMNS, columnSize, runsOf(layout.columns, MAX_COLUMNS, columnSize, column))
    this.rows = new Axis(MAX_ROWS, rowSize, runsOf(layout.rows, MAX_ROWS, rowSize, row))
    const frozen = layout.frozen || { rows: 0, columns: 0 }
    this.frozen = {
      rows: Math.min(frozen.rows || 0, MAX_ROWS - 1),
      columns: Math.min(frozen.columns || 0, MAX_COLUMNS - 1)
    }
    this.merges = new Merges(layout.merges)
    this.dimension = parseRange(layout.dimension) || { r0: 0, c0: 0, r1: 0, c1: 0 }
    this.version = ++versions
  }

  // The style an absent cell shows: its row's, else its column's.
  blankStyle (row, column) {
    return this.rows.styleOf(row) ?? this.columns.styleOf(column)
  }
}

// What one frame shows: the four quadrants a frozen split makes, each with the
// indices it covers and the offsets turning an axis position into a canvas one.
export class Viewport {
  constructor (geometry, scroll, width, height, header) {
    const { rows, columns, frozen } = geometry
    this.geometry = geometry
    this.width = width
    this.height = height
    this.headerWidth = header.width
    this.headerHeight = header.height
    this.frozenWidth = columns.start(frozen.columns)
    this.frozenHeight = rows.start(frozen.rows)
    this.scrollX = columns.start(scroll.leftColumn) + scroll.columnOffset
    this.scrollY = rows.start(scroll.topRow) + scroll.rowOffset
    this.bodyX = header.width + this.frozenWidth
    this.bodyY = header.height + this.frozenHeight
    this.ox = this.bodyX - this.scrollX
    this.oy = this.bodyY - this.scrollY
    const lastColumn = columns.indexAt(this.scrollX + Math.max(0, width - this.bodyX) - 1)
    const lastRow = rows.indexAt(this.scrollY + Math.max(0, height - this.bodyY) - 1)
    this.scrolled = {
      r0: scroll.topRow,
      c0: scroll.leftColumn,
      r1: Math.max(scroll.topRow, lastRow),
      c1: Math.max(scroll.leftColumn, lastColumn)
    }
    const fr = frozen.rows
    const fc = frozen.columns
    const quadrant = (name, r0, r1, c0, c1, x, y, w, h, ox, oy) => ({
      name, r0, r1, c0, c1, x, y, w: Math.max(0, w), h: Math.max(0, h), ox, oy
    })
    const bw = width - this.bodyX
    const bh = height - this.bodyY
    this.quadrants = [quadrant('main', this.scrolled.r0, this.scrolled.r1, this.scrolled.c0, this.scrolled.c1,
      this.bodyX, this.bodyY, bw, bh, this.ox, this.oy)]
    if (fr > 0) {
      this.quadrants.push(quadrant('top', 0, fr - 1, this.scrolled.c0, this.scrolled.c1,
        this.bodyX, header.height, bw, this.frozenHeight, this.ox, header.height))
    }
    if (fc > 0) {
      this.quadrants.push(quadrant('left', this.scrolled.r0, this.scrolled.r1, 0, fc - 1,
        header.width, this.bodyY, this.frozenWidth, bh, header.width, this.oy))
    }
    if (fr > 0 && fc > 0) {
      this.quadrants.push(quadrant('corner', 0, fr - 1, 0, fc - 1,
        header.width, header.height, this.frozenWidth, this.frozenHeight, header.width, header.height))
    }
  }

  // The rows and columns the frame shows, frozen ones first.
  visibleRows () {
    const shown = []
    const { rows, frozen } = this.geometry
    for (let r = 0; r < frozen.rows; r++) if (!rows.isHidden(r)) shown.push(r)
    for (let r = this.scrolled.r0; r <= this.scrolled.r1; r++) if (!rows.isHidden(r)) shown.push(r)
    return shown
  }

  visibleColumns () {
    const shown = []
    const { columns, frozen } = this.geometry
    for (let c = 0; c < frozen.columns; c++) if (!columns.isHidden(c)) shown.push(c)
    for (let c = this.scrolled.c0; c <= this.scrolled.c1; c++) if (!columns.isHidden(c)) shown.push(c)
    return shown
  }

  x (column) {
    return this.geometry.columns.start(column) +
      (column < this.geometry.frozen.columns ? this.headerWidth : this.ox)
  }

  y (row) {
    return this.geometry.rows.start(row) +
      (row < this.geometry.frozen.rows ? this.headerHeight : this.oy)
  }

  // Where a range's bottom-right corner is drawn - the fill handle's place -
  // or null while that corner's cell is outside the frame.
  corner (range) {
    const { rows, columns, frozen } = this.geometry
    const row = range.r1 < frozen.rows || (range.r1 >= this.scrolled.r0 && range.r1 <= this.scrolled.r1)
    const column = range.c1 < frozen.columns || (range.c1 >= this.scrolled.c0 && range.c1 <= this.scrolled.c1)
    if (!row || !column) return null
    return { x: this.x(range.c1) + columns.sizeOf(range.c1), y: this.y(range.r1) + rows.sizeOf(range.r1) }
  }

  // A range's rectangle in canvas pixels, from the quadrant of its top-left cell.
  rect (range) {
    const { rows, columns } = this.geometry
    const x = this.x(range.c0)
    const y = this.y(range.r0)
    return {
      x,
      y,
      w: columns.start(range.c1 + 1) - columns.start(range.c0),
      h: rows.start(range.r1 + 1) - rows.start(range.r0)
    }
  }

  columnAt (x) {
    const { columns, frozen } = this.geometry
    if (x < this.bodyX && frozen.columns > 0) {
      return Math.min(columns.indexAt(Math.max(0, x - this.headerWidth)), frozen.columns - 1)
    }
    return Math.max(columns.indexAt(x - this.ox), frozen.columns)
  }

  rowAt (y) {
    const { rows, frozen } = this.geometry
    if (y < this.bodyY && frozen.rows > 0) {
      return Math.min(rows.indexAt(Math.max(0, y - this.headerHeight)), frozen.rows - 1)
    }
    return Math.max(rows.indexAt(y - this.oy), frozen.rows)
  }

  // What a canvas point lands on: the corner, a header, or a cell.
  hit (x, y) {
    if (x < this.headerWidth && y < this.headerHeight) return { kind: 'corner' }
    if (y < this.headerHeight) return { kind: 'column', column: this.columnAt(x) }
    if (x < this.headerWidth) return { kind: 'row', row: this.rowAt(y) }
    return { kind: 'cell', row: this.rowAt(y), column: this.columnAt(x) }
  }

  // Rows fully shown in the scrolling pane, the page PgUp/PgDn moves by.
  pageRows () {
    const { rows } = this.geometry
    let count = 0
    for (let r = this.scrolled.r0; r <= this.scrolled.r1; r++) {
      if (rows.isHidden(r)) continue
      if (this.y(r) + rows.sizeOf(r) > this.height) break
      count++
    }
    return Math.max(1, count)
  }

  pageColumns () {
    const { columns } = this.geometry
    let count = 0
    for (let c = this.scrolled.c0; c <= this.scrolled.c1; c++) {
      if (columns.isHidden(c)) continue
      if (this.x(c) + columns.sizeOf(c) > this.width) break
      count++
    }
    return Math.max(1, count)
  }
}
