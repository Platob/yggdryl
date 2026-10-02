// The grid's scroll state, drawn scrollbars and pointer input. Positions are
// anchored to a row and a column (`topRow` + `rowOffset`, `leftColumn` +
// `columnOffset`), so a change above the view never moves it, and nothing
// asks the browser for an element 21 million pixels tall. A drag of the
// fill handle ends in a `fill` event ({ source, target, clear, copy }), a
// double click on it in `filldouble`; ribbon.js sends the edit. A press is
// announced first (`press`, cancelable when it lands on the sheet): an
// editor writing a formula cancels it to point at cells instead, and the
// drag then reports `point` events ({ phase, hit, extend, add }) rather than
// moving the selection. `setReferences` draws a formula's references.

import { Geometry, Viewport, MAX_ROWS, MAX_COLUMNS, TILE_ROWS, TILE_COLUMNS, normalize, fillTarget, rangeName, sameRange } from './geometry.js'
import { Painter } from './render.js'

const MIN_THUMB = 24
// How near the fill handle's centre, in CSS pixels, a press takes it.
const HANDLE = 5

export class Grid extends EventTarget {
  constructor ({ host, canvas, vertical, horizontal, tiles, styles, selection }) {
    super()
    this.host = host
    this.canvas = canvas
    this.bars = { vertical, horizontal }
    this.tiles = tiles
    this.styles = styles
    this.selection = selection
    this.painter = new Painter(canvas, styles)
    this.painter.readPalette(host)
    this.sheet = null
    this.layout = null
    this.geometry = null
    this.viewport = null
    this.scroll = { topRow: 0, rowOffset: 0, leftColumn: 0, columnOffset: 0 }
    this.zoom = 1
    this.gridlines = true
    this.message = null
    this.size = { width: 0, height: 0, dpr: 1 }
    this.scheduled = 0
    this.drag = null
    this.extent = { rows: 1, columns: 1 }
    this.thumbs = { rows: null, columns: null }
    this.frames = 0
    // Edits sent and not yet shown: { id, sheet, ranges, answered }.
    this.pending = []
    // The references of a formula being edited: { range, color, dashed }.
    this.references = []
    this.marks = 0
    host.dataset.edits = '0'

    new ResizeObserver(() => this.resize()).observe(host)
    matchMedia('(forced-colors: active)').addEventListener('change', () => {
      this.painter.readPalette(host)
      this.invalidate()
    })
    tiles.addEventListener('tile', (event) => {
      if (event.detail.sheet === this.sheet) this.invalidate()
    })
    styles.addEventListener('change', () => this.invalidate())
    selection.addEventListener('change', () => this.invalidate())
    canvas.addEventListener('wheel', (event) => this.wheel(event), { passive: false })
    canvas.addEventListener('pointerdown', (event) => this.pointerDown(event))
    canvas.addEventListener('pointermove', (event) => this.pointerMove(event))
    canvas.addEventListener('pointerup', (event) => this.pointerUp(event))
    canvas.addEventListener('pointercancel', (event) => this.pointerUp(event))
    canvas.addEventListener('dblclick', (event) => this.doubleClick(event))
    canvas.addEventListener('contextmenu', (event) => this.contextMenu(event))
    // Escape drops a fill-handle drag before anything else hears the key.
    window.addEventListener('keydown', (event) => {
      if (event.key !== 'Escape' || !this.drag || this.drag.kind !== 'fill') return
      event.preventDefault()
      event.stopPropagation()
      this.endFill(null)
    }, true)
    this.bar(vertical, 'rows')
    this.bar(horizontal, 'columns')
    this.resize()
  }

  // A sheet's layout, with the scroll and selection it had when last shown.
  show (sheet, layout, saved) {
    this.sheet = sheet
    this.layout = layout
    this.message = null
    this.geometry = new Geometry(layout, this.zoom)
    const { frozen } = this.geometry
    this.scroll = saved ? { ...saved.scroll } : { topRow: frozen.rows, rowOffset: 0, leftColumn: frozen.columns, columnOffset: 0 }
    this.clampScroll()
    this.selection.bind(this.geometry, saved && saved.selection)
    this.invalidate()
  }

  // The same sheet's layout after an edit changed it: the scroll anchor and
  // the selection stay where they were.
  setLayout (layout) {
    this.layout = layout
    this.geometry = new Geometry(layout, this.zoom)
    this.selection.relayout(this.geometry)
    this.clampScroll()
    this.invalidate()
  }

  // The ranges an edited formula names, each in its colour (render.js
  // draws them over the cells); `data-references` spells them for a test:
  // `B3:B5#0~` is B3:B5 in the first colour, dashed.
  setReferences (references) {
    this.references = references
    const spelled = references.map((item) => rangeName(item.range) + '#' + item.color + (item.dashed ? '~' : '')).join(' ')
    if (spelled) this.host.dataset.references = spelled
    else delete this.host.dataset.references
    this.invalidate()
  }

  showMessage (sheet, text) {
    this.sheet = sheet
    this.message = text
    this.geometry = null
    this.invalidate()
  }

  saved () {
    return { scroll: { ...this.scroll }, selection: this.selection.snapshot() }
  }

  setZoom (zoom) {
    if (zoom === this.zoom) return
    this.zoom = zoom
    if (this.layout) this.geometry = new Geometry(this.layout, zoom)
    if (this.geometry) this.selection.geometry = this.geometry
    this.painter.widths.clear()
    this.clampScroll()
    this.invalidate()
  }

  setGridlines (shown) {
    this.gridlines = shown
    this.invalidate()
  }

  resize () {
    const rect = this.host.getBoundingClientRect()
    const dpr = window.devicePixelRatio || 1
    const width = Math.max(0, Math.floor(rect.width - this.bars.vertical.offsetWidth))
    const height = Math.max(0, Math.floor(rect.height))
    this.size = { width, height, dpr }
    this.canvas.width = Math.max(1, Math.round(width * dpr))
    this.canvas.height = Math.max(1, Math.round(height * dpr))
    this.canvas.style.width = width + 'px'
    this.canvas.style.height = height + 'px'
    this.invalidate()
  }

  // Pending edits (§10: no optimistic values). A mark is hatched from the
  // moment its edit is sent until the service has answered and the tiles
  // under it are current again; `data-edits` counts the marks.
  addPending (sheet, ranges) {
    const id = ++this.marks
    this.pending.push({ id, sheet, ranges, answered: false })
    this.host.dataset.edits = String(this.pending.length)
    this.invalidate()
    return id
  }

  answered (id) {
    const mark = this.pending.find((item) => item.id === id)
    if (mark) mark.answered = true
    this.invalidate()
  }

  dropPending (id) {
    this.pending = this.pending.filter((item) => item.id !== id)
    this.host.dataset.edits = String(this.pending.length)
    this.invalidate()
  }

  // Marks whose tiles have settled go; one on a sheet not shown goes as soon
  // as it is answered, its tiles being revalidated when it is shown again.
  settlePending () {
    if (this.pending.length === 0) return
    this.pending = this.pending.filter((mark) =>
      !(mark.answered && (mark.sheet !== this.sheet || this.tiles.settled(mark.sheet, mark.ranges))))
    this.host.dataset.edits = String(this.pending.length)
  }

  invalidate () {
    if (this.scheduled) return
    // `data-frame` marks a frame owed, so a check can wait for the canvas.
    this.host.dataset.frame = 'owed'
    this.scheduled = requestAnimationFrame(() => {
      this.scheduled = 0
      this.frame()
    })
  }

  // Row numbers widen the row header as they gain digits: the width is
  // settled on the largest row number the frame shows.
  headerFor (digits) {
    const zoom = this.zoom
    const font = this.painter.headerFont(zoom)
    const width = Math.max(Math.round(26 * zoom), Math.ceil(this.painter.measure(font, '0'.repeat(digits))) + Math.round(12 * zoom))
    return { width, height: Math.round(20 * zoom) }
  }

  build () {
    if (!this.geometry) return null
    const guess = String(this.scroll.topRow + 1).length
    let viewport = new Viewport(this.geometry, this.scroll, this.size.width, this.size.height, this.headerFor(guess))
    const digits = String(viewport.scrolled.r1 + 1).length
    if (digits !== guess) {
      viewport = new Viewport(this.geometry, this.scroll, this.size.width, this.size.height, this.headerFor(digits))
    }
    this.viewport = viewport
    return viewport
  }

  frame () {
    delete this.host.dataset.frame
    const viewport = this.build()
    const cell = this.sheet !== null && this.geometry ? this.tiles.view(this.sheet) : () => undefined
    if (viewport) this.request(viewport)
    else this.host.dataset.pending = '0'
    this.settlePending()
    const pending = []
    for (const mark of this.pending) if (mark.sheet === this.sheet) pending.push(...mark.ranges)
    const fill = this.drag && this.drag.kind === 'fill' ? this.drag : null
    this.painter.paint({
      viewport: viewport || { width: this.size.width, height: this.size.height, quadrants: [] },
      geometry: this.geometry,
      cell,
      selection: this.selection,
      zoom: this.zoom,
      dpr: this.size.dpr,
      gridlines: this.gridlines,
      pending,
      fill: fill && { source: fill.source, target: fill.target, clear: fill.clear },
      references: this.references,
      message: this.message || (viewport ? null : ' ')
    })
    this.frames++
    this.canvas.dataset.frames = String(this.frames)
    this.canvas.dataset.paintMs = this.painter.last.ms.toFixed(2)
    this.canvas.dataset.cells = String(this.painter.last.cells)
    // A thumb shows only when there is a sheet to scroll.
    this.bars.vertical.firstElementChild.hidden = !viewport
    this.bars.horizontal.firstElementChild.hidden = !viewport
    if (viewport) this.updateBars(viewport)
    this.dispatchEvent(new CustomEvent('frame'))
  }

  // The tiles a frame shows: every quadrant's block and the anchors of the
  // merges it crosses; then the ring one tile around the scrolled block.
  shownTiles (viewport) {
    const keys = new Set()
    const shown = []
    const add = (tr, tc) => {
      const key = tr * 1024 + tc
      if (tr < 0 || tc < 0 || tr >= MAX_ROWS / TILE_ROWS || tc >= MAX_COLUMNS / TILE_COLUMNS || keys.has(key)) return
      keys.add(key)
      shown.push([tr, tc])
    }
    for (const q of viewport.quadrants) {
      if (q.w <= 0 || q.h <= 0) continue
      for (let tr = Math.floor(q.r0 / TILE_ROWS); tr <= Math.floor(q.r1 / TILE_ROWS); tr++) {
        for (let tc = Math.floor(q.c0 / TILE_COLUMNS); tc <= Math.floor(q.c1 / TILE_COLUMNS); tc++) add(tr, tc)
      }
      for (const merge of this.geometry.merges.within(q)) add(Math.floor(merge.r0 / TILE_ROWS), Math.floor(merge.c0 / TILE_COLUMNS))
    }
    return { shown, keys }
  }

  request (viewport) {
    const { shown, keys } = this.shownTiles(viewport)
    // `data-pending` counts the shown tiles not held yet.
    const s = viewport.scrolled
    const ring = []
    const tr0 = Math.floor(s.r0 / TILE_ROWS) - 1
    const tr1 = Math.floor(s.r1 / TILE_ROWS) + 1
    const tc0 = Math.floor(s.c0 / TILE_COLUMNS) - 1
    const tc1 = Math.floor(s.c1 / TILE_COLUMNS) + 1
    for (let tr = tr0; tr <= tr1; tr++) {
      for (let tc = tc0; tc <= tc1; tc++) {
        if (tr < 0 || tc < 0 || tr >= MAX_ROWS / TILE_ROWS || tc >= MAX_COLUMNS / TILE_COLUMNS) continue
        if (!keys.has(tr * 1024 + tc)) ring.push([tr, tc])
      }
    }
    this.tiles.want(this.sheet, shown, ring)
    this.host.dataset.pending = String(this.tiles.pending(this.sheet, shown))
  }

  // Scrolling ----------------------------------------------------------------

  clampScroll () {
    if (!this.geometry) return
    const { rows, columns, frozen } = this.geometry
    this.setScrollY(rows.start(this.scroll.topRow) + this.scroll.rowOffset, false)
    this.setScrollX(columns.start(this.scroll.leftColumn) + this.scroll.columnOffset, false)
    if (this.scroll.topRow < frozen.rows) this.scroll = { ...this.scroll, topRow: frozen.rows, rowOffset: 0 }
    if (this.scroll.leftColumn < frozen.columns) this.scroll = { ...this.scroll, leftColumn: frozen.columns, columnOffset: 0 }
  }

  setScrollY (position, repaint = true) {
    const { rows, frozen } = this.geometry
    const min = rows.start(frozen.rows)
    const max = Math.max(min, rows.start(rows.lastVisible()))
    const p = Math.min(Math.max(position, min), max)
    const topRow = Math.max(frozen.rows, rows.indexAt(p))
    this.scroll.topRow = topRow
    this.scroll.rowOffset = Math.max(0, p - rows.start(topRow))
    if (repaint) this.invalidate()
  }

  setScrollX (position, repaint = true) {
    const { columns, frozen } = this.geometry
    const min = columns.start(frozen.columns)
    const max = Math.max(min, columns.start(columns.lastVisible()))
    const p = Math.min(Math.max(position, min), max)
    const leftColumn = Math.max(frozen.columns, columns.indexAt(p))
    this.scroll.leftColumn = leftColumn
    this.scroll.columnOffset = Math.max(0, p - columns.start(leftColumn))
    if (repaint) this.invalidate()
  }

  scrollBy (dx, dy) {
    if (!this.geometry) return
    const { rows, columns } = this.geometry
    if (dy) this.setScrollY(rows.start(this.scroll.topRow) + this.scroll.rowOffset + dy)
    if (dx) this.setScrollX(columns.start(this.scroll.leftColumn) + this.scroll.columnOffset + dx)
  }

  scrollTo (topRow, leftColumn) {
    if (!this.geometry) return
    const { rows, columns } = this.geometry
    if (topRow !== null && topRow !== undefined) this.setScrollY(rows.start(topRow))
    if (leftColumn !== null && leftColumn !== undefined) this.setScrollX(columns.start(leftColumn))
  }

  // Scroll the least that shows (row, column) whole; frozen lines always are.
  ensureVisible (row, column) {
    if (!this.geometry) return
    // The scroll may have moved since the last frame; measure it afresh.
    const viewport = this.build()
    if (!viewport) return
    const { rows, columns, frozen } = this.geometry
    const merge = this.geometry.merges.at(row, column)
    const r0 = merge ? merge.r0 : row
    const r1 = merge ? merge.r1 : row
    const c0 = merge ? merge.c0 : column
    const c1 = merge ? merge.c1 : column
    if (r1 >= frozen.rows) {
      const top = rows.start(Math.max(r0, frozen.rows))
      const bottom = rows.start(r1 + 1)
      const pane = Math.max(1, viewport.height - viewport.bodyY)
      const y = viewport.scrollY
      if (top < y) this.setScrollY(top)
      else if (bottom > y + pane) {
        const want = Math.min(top, bottom - pane)
        let first = rows.indexAt(want)
        if (rows.start(first) < want) first = rows.firstVisible(first + 1, 1)
        this.setScrollY(rows.start(Math.min(Math.max(first, frozen.rows), Math.max(r0, frozen.rows))))
      }
    }
    if (c1 >= frozen.columns) {
      // A vertical move can widen the row header, narrowing the pane.
      const across = this.build()
      const left = columns.start(Math.max(c0, frozen.columns))
      const right = columns.start(c1 + 1)
      const pane = Math.max(1, across.width - across.bodyX)
      const x = across.scrollX
      if (left < x) this.setScrollX(left)
      else if (right > x + pane) {
        const want = Math.min(left, right - pane)
        let first = columns.indexAt(want)
        if (columns.start(first) < want) first = columns.firstVisible(first + 1, 1)
        this.setScrollX(columns.start(Math.min(Math.max(first, frozen.columns), Math.max(c0, frozen.columns))))
      }
    }
    this.build()
  }

  // PgUp/PgDn: the view and the active cell move one pane's worth of rows.
  pageRows (direction) {
    const viewport = this.viewport || this.build()
    return viewport ? viewport.pageRows() * direction : direction
  }

  pageColumns (direction) {
    const viewport = this.viewport || this.build()
    return viewport ? viewport.pageColumns() * direction : direction
  }

  // Scrollbars ---------------------------------------------------------------

  // The scrollable extent, in lines: the used range, the view and the
  // selection, plus a pane, so the thumb has somewhere to go.
  updateBars (viewport) {
    const { frozen, dimension } = this.geometry
    const pageRows = viewport.pageRows()
    const pageColumns = viewport.pageColumns()
    const focus = this.selection.focus
    const usedRows = Math.max(dimension.r1 + 1, Math.min(focus.row, MAX_ROWS - 1) + 1, this.scroll.topRow + pageRows)
    const usedColumns = Math.max(dimension.c1 + 1, Math.min(focus.column, MAX_COLUMNS - 1) + 1, this.scroll.leftColumn + pageColumns)
    if (!this.drag || this.drag.kind !== 'rows') this.extent.rows = Math.min(MAX_ROWS, usedRows + pageRows)
    if (!this.drag || this.drag.kind !== 'columns') this.extent.columns = Math.min(MAX_COLUMNS, usedColumns + pageColumns)
    this.place(this.bars.vertical, 'rows', this.scroll.topRow - frozen.rows, this.extent.rows - frozen.rows, pageRows)
    this.place(this.bars.horizontal, 'columns', this.scroll.leftColumn - frozen.columns, this.extent.columns - frozen.columns, pageColumns)
  }

  place (bar, kind, at, total, page) {
    const thumb = bar.firstElementChild
    const vertical = kind === 'rows'
    const track = vertical ? bar.clientHeight : bar.clientWidth
    if (track <= 0) return
    const size = Math.min(track, Math.max(MIN_THUMB, Math.round((track * page) / Math.max(page, total))))
    const room = Math.max(1, total - page)
    const offset = Math.round((track - size) * Math.min(1, Math.max(0, at / room)))
    if (vertical) {
      thumb.style.height = size + 'px'
      thumb.style.transform = `translateY(${offset}px)`
    } else {
      thumb.style.width = size + 'px'
      thumb.style.transform = `translateX(${offset}px)`
    }
    bar.setAttribute('aria-valuenow', String(at))
    bar.setAttribute('aria-valuemax', String(total))
    this.thumbs[kind] = { size, offset, track, room }
  }

  bar (bar, kind) {
    const thumb = bar.firstElementChild
    const vertical = kind === 'rows'
    thumb.addEventListener('pointerdown', (event) => {
      if (event.button !== 0 || !this.geometry) return
      event.preventDefault()
      event.stopPropagation()
      thumb.setPointerCapture(event.pointerId)
      const state = this.thumbs[kind] || { offset: 0, track: 1, size: 1, room: 1 }
      this.drag = { kind, start: vertical ? event.clientY : event.clientX, offset: state.offset }
      thumb.classList.add('dragging')
    })
    thumb.addEventListener('pointermove', (event) => {
      if (!this.drag || this.drag.kind !== kind) return
      const state = this.thumbs[kind]
      const moved = (vertical ? event.clientY : event.clientX) - this.drag.start
      const fraction = Math.min(1, Math.max(0, (this.drag.offset + moved) / Math.max(1, state.track - state.size)))
      const { frozen } = this.geometry
      if (vertical) this.scrollTo(frozen.rows + Math.round(fraction * state.room), null)
      else this.scrollTo(null, frozen.columns + Math.round(fraction * state.room))
    })
    const end = (event) => {
      if (!this.drag || this.drag.kind !== kind) return
      thumb.releasePointerCapture(event.pointerId)
      thumb.classList.remove('dragging')
      this.drag = null
      this.invalidate()
    }
    thumb.addEventListener('pointerup', end)
    thumb.addEventListener('pointercancel', end)
    // A press on the track pages toward the pointer.
    bar.addEventListener('pointerdown', (event) => {
      if (event.target !== bar || !this.geometry) return
      event.preventDefault()
      const state = this.thumbs[kind] || { offset: 0 }
      const rect = bar.getBoundingClientRect()
      const at = vertical ? event.clientY - rect.top : event.clientX - rect.left
      const direction = at < state.offset ? -1 : 1
      if (vertical) this.scrollTo(this.scroll.topRow + this.pageRows(direction), null)
      else this.scrollTo(null, this.scroll.leftColumn + this.pageColumns(direction))
    })
  }

  wheel (event) {
    if (!this.geometry || event.ctrlKey) return
    event.preventDefault()
    const line = this.geometry.rows.size
    const scale = event.deltaMode === 1 ? line : event.deltaMode === 2 ? this.size.height : 1
    let dx = event.deltaX * scale
    let dy = event.deltaY * scale
    if (event.shiftKey && dx === 0) {
      dx = dy
      dy = 0
    }
    this.scrollBy(dx, dy)
  }

  // Pointer selection --------------------------------------------------------

  point (event) {
    const rect = this.canvas.getBoundingClientRect()
    return { x: event.clientX - rect.left, y: event.clientY - rect.top }
  }

  pointerDown (event) {
    // The canvas takes no focus; the mirror grid keeps it. `press` comes
    // first, so an open editor commits before the selection moves - or,
    // pointing at cells for a formula, cancels it and takes the drag.
    event.preventDefault()
    const viewport = this.geometry ? this.viewport || this.build() : null
    const { x, y } = this.point(event)
    const hit = viewport && event.button === 0 && event.pointerType !== 'touch' ? viewport.hit(x, y) : null
    const press = new CustomEvent('press', { cancelable: Boolean(hit), detail: { hit, extend: event.shiftKey } })
    const pointing = !this.dispatchEvent(press)
    this.dispatchEvent(new CustomEvent('focus'))
    if (pointing) {
      this.canvas.setPointerCapture(event.pointerId)
      this.drag = { kind: 'point', hit: hit.kind, x, y, at: hit }
      this.dispatchEvent(new CustomEvent('point', { detail: { phase: 'start', hit, extend: event.shiftKey, add: event.ctrlKey || event.metaKey } }))
      return
    }
    if (event.button !== 0 || !this.geometry) return
    if (event.pointerType === 'touch') {
      this.canvas.setPointerCapture(event.pointerId)
      this.drag = { kind: 'pan', x, y, startX: x, startY: y }
      return
    }
    const add = event.ctrlKey || event.metaKey
    const extend = event.shiftKey
    this.canvas.setPointerCapture(event.pointerId)
    if (hit.kind === 'cell' && !extend && this.onHandle(viewport, x, y)) {
      const source = { ...this.selection.range }
      this.drag = { kind: 'fill', x, y, source, target: source, clear: null, id: event.pointerId }
      this.markFill(add)
      return
    }
    this.drag = { kind: 'select', hit: hit.kind, x, y }
    const selection = this.selection
    if (hit.kind === 'corner') {
      selection.selectAll()
    } else if (hit.kind === 'column') {
      selection.selectColumns(hit.column, hit.column, { add, extend, first: viewport.rowAt(viewport.bodyY) })
      this.drag.from = extend ? selection.active.column : hit.column
    } else if (hit.kind === 'row') {
      selection.selectRows(hit.row, hit.row, { add, extend, first: viewport.columnAt(viewport.bodyX) })
      this.drag.from = extend ? selection.active.row : hit.row
    } else if (extend) {
      selection.extendTo(hit.row, hit.column)
    } else if (add) {
      selection.add(hit.row, hit.column)
    } else {
      selection.select(hit.row, hit.column)
    }
    this.dispatchEvent(new CustomEvent('dragging', { detail: true }))
  }

  pointerMove (event) {
    if (!this.geometry) return
    const viewport = this.viewport || this.build()
    const { x, y } = this.point(event)
    if (this.drag && this.drag.kind === 'pan') {
      this.scrollBy(this.drag.x - x, this.drag.y - y)
      this.drag.x = x
      this.drag.y = y
      return
    }
    if (this.drag && this.drag.kind === 'fill') {
      this.drag.x = x
      this.drag.y = y
      this.markFill(event.ctrlKey || event.metaKey)
      this.dragTo(viewport)
      this.autoScroll()
      return
    }
    if (this.drag && this.drag.kind === 'point') {
      this.drag.x = x
      this.drag.y = y
      this.dragTo(viewport)
      this.autoScroll()
      return
    }
    if (!this.drag || this.drag.kind !== 'select') {
      const hit = viewport.hit(x, y)
      this.canvas.dataset.cursor = hit.kind === 'cell' && this.onHandle(viewport, x, y) ? 'handle' : hit.kind
      return
    }
    this.drag.x = x
    this.drag.y = y
    this.dragTo(viewport)
    this.autoScroll()
  }

  dragTo (viewport) {
    const { x, y } = this.drag
    const cx = Math.min(Math.max(x, viewport.headerWidth), viewport.width - 1)
    const cy = Math.min(Math.max(y, viewport.headerHeight), viewport.height - 1)
    const selection = this.selection
    if (this.drag.kind === 'point') {
      // The cell, row or column under the pointer, of the kind pressed.
      const at = this.drag.hit === 'column'
        ? { kind: 'column', column: viewport.columnAt(cx) }
        : this.drag.hit === 'row'
          ? { kind: 'row', row: viewport.rowAt(cy) }
          : this.drag.hit === 'corner' ? { kind: 'corner' } : { kind: 'cell', row: viewport.rowAt(cy), column: viewport.columnAt(cx) }
      const before = this.drag.at
      if (at.kind !== before.kind || at.row !== before.row || at.column !== before.column) {
        this.drag.at = at
        this.dispatchEvent(new CustomEvent('point', { detail: { phase: 'move', hit: at, extend: false } }))
      }
      return
    }
    if (this.drag.kind === 'fill') {
      const { target, clear } = fillTarget(this.drag.source, viewport.rowAt(cy), viewport.columnAt(cx))
      if (!sameRange(target, this.drag.target) || Boolean(clear) !== Boolean(this.drag.clear)) {
        this.drag.target = target
        this.drag.clear = clear
        this.host.dataset.fillTarget = rangeName(target)
        this.invalidate()
      }
      return
    }
    if (this.drag.hit === 'column') {
      const column = viewport.columnAt(cx)
      selection.selectColumns(this.drag.from, column, { extend: true })
    } else if (this.drag.hit === 'row') {
      const row = viewport.rowAt(cy)
      selection.selectRows(this.drag.from, row, { extend: true })
    } else if (this.drag.hit === 'cell') {
      selection.extendTo(viewport.rowAt(cy), viewport.columnAt(cx))
    }
    const range = selection.range
    this.dispatchEvent(new CustomEvent('dragsize', { detail: normalize(range.r0, range.c0, range.r1, range.c1) }))
  }

  // While a drag is held past the pane's edge, the view follows it.
  autoScroll () {
    if (this.drag.timer) return
    const tick = () => {
      if (!this.drag || (this.drag.kind !== 'select' && this.drag.kind !== 'fill' && this.drag.kind !== 'point')) return
      const viewport = this.viewport
      const { x, y } = this.drag
      let dx = 0
      let dy = 0
      if (this.drag.hit !== 'row') {
        if (x > viewport.width) dx = 1
        else if (x < viewport.headerWidth) dx = -1
      }
      if (this.drag.hit !== 'column') {
        if (y > viewport.height) dy = 1
        else if (y < viewport.headerHeight) dy = -1
      }
      if (dx === 0 && dy === 0) {
        this.drag.timer = 0
        return
      }
      if (dy) this.scrollTo(this.geometry.rows.advance(this.scroll.topRow, dy), null)
      if (dx) this.scrollTo(null, this.geometry.columns.advance(this.scroll.leftColumn, dx))
      this.dragTo(this.build())
      this.drag.timer = setTimeout(tick, 40)
    }
    this.drag.timer = setTimeout(tick, 40)
  }

  // A double click on a cell edits it in place; on the fill handle it fills
  // down the current region.
  doubleClick (event) {
    if (!this.geometry || event.button !== 0) return
    const { x, y } = this.point(event)
    const viewport = this.viewport || this.build()
    const hit = viewport.hit(x, y)
    if (hit.kind !== 'cell') return
    if (this.onHandle(viewport, x, y)) this.dispatchEvent(new CustomEvent('filldouble'))
    else this.dispatchEvent(new CustomEvent('activate', { detail: { row: hit.row, column: hit.column } }))
  }

  // Whether (x, y) is on the fill handle: one range selected, its corner in
  // the frame, no Format Painter waiting for its target.
  onHandle (viewport, x, y) {
    if (this.selection.ranges.length !== 1 || this.host.dataset.painter) return false
    const corner = viewport.corner(this.selection.range)
    return Boolean(corner) && Math.abs(x - (corner.x - 1)) <= HANDLE && Math.abs(y - (corner.y - 1)) <= HANDLE
  }

  // `data-fill` says what a release fills with: a series, or a copy while
  // Ctrl is held.
  markFill (copy) {
    this.drag.copy = copy
    this.host.dataset.fill = copy ? 'copy' : 'series'
  }

  // The end of a fill-handle drag: `event` a release, null when dropped.
  endFill (event) {
    const drag = this.drag
    if (drag.timer) clearTimeout(drag.timer)
    if (this.canvas.hasPointerCapture(drag.id)) this.canvas.releasePointerCapture(drag.id)
    this.drag = null
    delete this.host.dataset.fill
    delete this.host.dataset.fillTarget
    this.invalidate()
    if (!event || event.type !== 'pointerup' || sameRange(drag.source, drag.target)) return
    const copy = event.ctrlKey || event.metaKey
    this.dispatchEvent(new CustomEvent('fill', { detail: { source: drag.source, target: drag.target, clear: drag.clear, copy } }))
  }

  // A right click keeps a selection it lands in and otherwise selects what
  // it lands on, then asks for the menu of a cell, a row or a column.
  contextMenu (event) {
    event.preventDefault()
    if (!this.geometry) return
    const { x, y } = this.point(event)
    const hit = (this.viewport || this.build()).hit(x, y)
    const selection = this.selection
    const full = (test) => selection.ranges.some(test)
    let kind = 'cell'
    if (hit.kind === 'column') {
      kind = 'column'
      if (!full((range) => range.r0 === 0 && range.r1 === MAX_ROWS - 1 && hit.column >= range.c0 && hit.column <= range.c1)) {
        selection.selectColumns(hit.column, hit.column, {})
      }
    } else if (hit.kind === 'row') {
      kind = 'row'
      if (!full((range) => range.c0 === 0 && range.c1 === MAX_COLUMNS - 1 && hit.row >= range.r0 && hit.row <= range.r1)) {
        selection.selectRows(hit.row, hit.row, {})
      }
    } else if (hit.kind === 'cell' && !selection.isSelected(hit.row, hit.column)) {
      selection.select(hit.row, hit.column)
    }
    this.dispatchEvent(new CustomEvent('context', { detail: { kind, x: event.clientX, y: event.clientY } }))
  }

  pointerUp (event) {
    if (this.drag && this.drag.kind === 'fill') {
      this.endFill(event)
      return
    }
    if (this.drag && this.drag.kind === 'point') {
      if (this.drag.timer) clearTimeout(this.drag.timer)
      if (this.canvas.hasPointerCapture(event.pointerId)) this.canvas.releasePointerCapture(event.pointerId)
      const at = this.drag.at
      this.drag = null
      this.dispatchEvent(new CustomEvent('point', { detail: { phase: 'end', hit: at, extend: false } }))
      return
    }
    if (this.drag && this.drag.kind === 'pan') {
      const { x, y } = this.point(event)
      const tap = Math.abs(x - this.drag.startX) < 6 && Math.abs(y - this.drag.startY) < 6
      this.drag = null
      if (this.canvas.hasPointerCapture(event.pointerId)) this.canvas.releasePointerCapture(event.pointerId)
      if (tap && event.type === 'pointerup') {
        const hit = (this.viewport || this.build()).hit(x, y)
        if (hit.kind === 'cell') this.selection.select(hit.row, hit.column)
      }
      return
    }
    if (!this.drag || this.drag.kind !== 'select') return
    if (this.drag.timer) clearTimeout(this.drag.timer)
    if (this.canvas.hasPointerCapture(event.pointerId)) this.canvas.releasePointerCapture(event.pointerId)
    this.drag = null
    this.dispatchEvent(new CustomEvent('dragging', { detail: false }))
  }
}
