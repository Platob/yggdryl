// A visually hidden role=grid mirroring the cells the canvas shows, with
// `aria-activedescendant` on the active cell. It is the keyboard focus target,
// what a screen reader reads, and what the browser checks assert against.

import { MAX_ROWS, MAX_COLUMNS, columnName, refName, rangeName, contains } from './geometry.js'

const DEBOUNCE = 60

export class Mirror {
  constructor (host, { grid, selection }) {
    this.grid = grid
    this.selection = selection
    this.element = document.createElement('div')
    this.element.className = 'visually-hidden grid-mirror'
    this.element.id = 'grid-mirror'
    this.element.setAttribute('role', 'grid')
    this.element.setAttribute('tabindex', '0')
    this.element.setAttribute('aria-multiselectable', 'true')
    this.element.setAttribute('aria-rowcount', String(MAX_ROWS + 1))
    this.element.setAttribute('aria-colcount', String(MAX_COLUMNS + 1))
    this.element.setAttribute('aria-readonly', 'true')
    this.body = document.createElement('div')
    this.body.setAttribute('role', 'rowgroup')
    this.element.append(this.body)
    this.live = document.createElement('div')
    this.live.className = 'visually-hidden'
    this.live.id = 'grid-live'
    this.live.setAttribute('aria-live', 'polite')
    this.live.setAttribute('aria-atomic', 'true')
    host.append(this.element, this.live)
    this.timer = 0
    this.label = ''
    this.sheet = null
    grid.addEventListener('frame', () => this.schedule())
    selection.addEventListener('change', () => this.activate())
  }

  focus () {
    this.element.focus({ preventScroll: true })
  }

  // `data-stale` marks a mirror behind the canvas, so a check can wait for
  // it; the rebuild waits for the frames to pause.
  schedule () {
    this.element.dataset.stale = 'true'
    clearTimeout(this.timer)
    this.timer = setTimeout(() => {
      this.timer = 0
      this.update()
    }, DEBOUNCE)
  }

  // Whether the sheet takes edits; read-only workbooks say so.
  setReadOnly (readOnly) {
    this.element.setAttribute('aria-readonly', readOnly ? 'true' : 'false')
  }

  setSheet (key, name) {
    this.sheet = key
    this.label = name
    this.element.setAttribute('aria-label', name)
    this.element.dataset.sheet = String(key)
  }

  announce (text) {
    this.live.textContent = ''
    // A fresh text node is what makes a repeated message speak again.
    requestAnimationFrame(() => {
      this.live.textContent = text
    })
  }

  // Point aria-activedescendant at the active cell, rebuilding the mirror
  // when that cell is not in it yet.
  activate () {
    const { active } = this.selection
    const id = 'yg-' + refName(active.row, active.column)
    this.element.dataset.active = refName(active.row, active.column)
    this.element.dataset.selection = this.selection.refs.join(',')
    if (document.getElementById(id) && this.element.contains(document.getElementById(id))) {
      this.element.setAttribute('aria-activedescendant', id)
      this.markSelected()
    } else {
      if (this.timer) clearTimeout(this.timer)
      this.timer = 0
      this.update()
    }
  }

  markSelected () {
    for (const cell of this.body.querySelectorAll('[role="gridcell"]')) {
      const selected = this.selection.isSelected(Number(cell.dataset.row), Number(cell.dataset.column))
      cell.setAttribute('aria-selected', selected ? 'true' : 'false')
    }
  }

  update () {
    const grid = this.grid
    const viewport = grid.viewport
    if (!viewport || grid.message || !grid.geometry) {
      this.body.replaceChildren()
      delete this.element.dataset.stale
      this.element.removeAttribute('aria-activedescendant')
      this.element.dataset.rows = '0'
      return
    }
    const { geometry } = grid
    const { active } = this.selection
    const rows = viewport.visibleRows()
    const columns = viewport.visibleColumns()
    if (!rows.includes(active.row)) rows.push(active.row)
    rows.sort((a, b) => a - b)
    const shownColumns = new Set(columns)
    const cell = grid.tiles.view(grid.sheet)
    const pending = []
    for (const mark of grid.pending) if (mark.sheet === grid.sheet) pending.push(...mark.ranges)
    const fragment = document.createDocumentFragment()

    const header = document.createElement('div')
    header.setAttribute('role', 'row')
    header.setAttribute('aria-rowindex', '1')
    const corner = document.createElement('div')
    corner.setAttribute('role', 'columnheader')
    corner.setAttribute('aria-colindex', '1')
    corner.textContent = this.label
    header.append(corner)
    for (const c of columns) {
      const node = document.createElement('div')
      node.setAttribute('role', 'columnheader')
      node.setAttribute('aria-colindex', String(c + 2))
      node.textContent = columnName(c)
      header.append(node)
    }
    fragment.append(header)

    for (const r of rows) {
      const row = document.createElement('div')
      row.setAttribute('role', 'row')
      row.setAttribute('aria-rowindex', String(r + 2))
      const head = document.createElement('div')
      head.setAttribute('role', 'rowheader')
      head.setAttribute('aria-colindex', '1')
      head.textContent = String(r + 1)
      row.append(head)
      const cells = shownColumns.has(active.column) || r !== active.row ? columns : [...columns, active.column].sort((a, b) => a - b)
      for (const c of cells) {
        const merge = geometry.merges.at(r, c)
        if (merge && (merge.r0 !== r || merge.c0 !== c)) continue
        const ref = refName(r, c)
        const node = document.createElement('div')
        node.setAttribute('role', 'gridcell')
        node.id = 'yg-' + ref
        node.dataset.ref = ref
        node.dataset.row = String(r)
        node.dataset.column = String(c)
        node.setAttribute('aria-colindex', String(c + 2))
        node.setAttribute('aria-selected', this.selection.isSelected(r, c) ? 'true' : 'false')
        if (merge) {
          node.setAttribute('aria-rowspan', String(merge.r1 - merge.r0 + 1))
          node.setAttribute('aria-colspan', String(merge.c1 - merge.c0 + 1))
          node.dataset.merge = rangeName(merge)
        }
        // Where the canvas draws the cell, in CSS pixels from its top-left,
        // so a pointer can be aimed at it.
        const box = viewport.rect(merge || { r0: r, c0: c, r1: r, c1: c })
        node.dataset.x = String(box.x)
        node.dataset.y = String(box.y)
        node.dataset.w = String(box.w)
        node.dataset.h = String(box.h)
        const found = cell(r, c)
        if (found) node.textContent = found.text
        // A cell whose edit is unanswered still reads its old value, busy.
        if (pending.some((range) => contains(range, r, c))) node.setAttribute('aria-busy', 'true')
        row.append(node)
      }
      fragment.append(row)
    }
    this.body.replaceChildren(fragment)
    delete this.element.dataset.stale
    this.element.dataset.rows = String(rows.length)
    this.element.dataset.top = refName(viewport.scrolled.r0, viewport.scrolled.c0)
    const id = 'yg-' + refName(active.row, active.column)
    if (document.getElementById(id)) this.element.setAttribute('aria-activedescendant', id)
    else this.element.removeAttribute('aria-activedescendant')
    this.element.dataset.active = refName(active.row, active.column)
    this.element.dataset.selection = this.selection.refs.join(',')
  }
}
