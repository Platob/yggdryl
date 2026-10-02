// The selection: an active cell, the ranges selected (several with
// Ctrl+click), and the far corner the current range extends to. Merged cells
// are whole: a range touching one grows to hold it, and moving steps over it.

import { MAX_ROWS, MAX_COLUMNS, normalize, sameRange, contains, rangeName, refName } from './geometry.js'

export class Selection extends EventTarget {
  constructor () {
    super()
    this.geometry = null
    this.active = { row: 0, column: 0 }
    this.focus = { row: 0, column: 0 }
    this.ranges = [{ r0: 0, c0: 0, r1: 0, c1: 0 }]
    this.current = 0
    this.tabColumn = null
  }

  bind (geometry, saved) {
    this.geometry = geometry
    if (saved) {
      this.active = { ...saved.active }
      this.focus = { ...saved.focus }
      this.ranges = saved.ranges.map((range) => ({ ...range }))
      this.current = saved.current
      this.changed()
    } else {
      // A sheet opens on the first cell past its frozen panes, as Ctrl+Home.
      const row = geometry.rows.firstVisible(geometry.frozen.rows, 1)
      const column = geometry.columns.firstVisible(geometry.frozen.columns, 1)
      this.select(Math.max(0, row), Math.max(0, column))
    }
    this.tabColumn = null
  }

  // The layout changed under the selection (a merge, a hidden line): each
  // range grows to hold the merges it now crosses, and an active cell
  // inside a merge moves to its anchor.
  relayout (geometry) {
    this.geometry = geometry
    this.ranges = this.ranges.map((range) => this.merged(range))
    this.active = this.anchorOf(this.active.row, this.active.column)
    this.changed()
  }

  snapshot () {
    return {
      active: { ...this.active },
      focus: { ...this.focus },
      ranges: this.ranges.map((range) => ({ ...range })),
      current: this.current
    }
  }

  changed () {
    this.dispatchEvent(new CustomEvent('change'))
  }

  merged (range) {
    return this.geometry ? this.geometry.merges.expand(range) : range
  }

  mergeAt (row, column) {
    return this.geometry ? this.geometry.merges.at(row, column) : null
  }

  // The cell that holds (row, column): a merge's top-left, or itself.
  anchorOf (row, column) {
    const merge = this.mergeAt(row, column)
    return merge ? { row: merge.r0, column: merge.c0 } : { row, column }
  }

  get range () {
    return this.ranges[this.current]
  }

  isSingle () {
    const range = this.range
    const merge = this.mergeAt(this.active.row, this.active.column)
    return this.ranges.length === 1 && (sameRange(range, { r0: this.active.row, c0: this.active.column, r1: this.active.row, c1: this.active.column }) || (merge && sameRange(range, merge)))
  }

  select (row, column) {
    const at = this.anchorOf(row, column)
    this.active = at
    this.focus = { ...at }
    this.ranges = [this.merged({ r0: at.row, c0: at.column, r1: at.row, c1: at.column })]
    this.current = 0
    this.tabColumn = null
    this.changed()
  }

  // Ctrl+click: a further range, whose cell becomes active.
  add (row, column) {
    const at = this.anchorOf(row, column)
    this.active = at
    this.focus = { ...at }
    this.ranges.push(this.merged({ r0: at.row, c0: at.column, r1: at.row, c1: at.column }))
    this.current = this.ranges.length - 1
    this.tabColumn = null
    this.changed()
  }

  // Shift+click, a drag, Shift+arrows: the current range spans the active cell
  // to (row, column).
  extendTo (row, column) {
    this.focus = { row, column }
    this.ranges[this.current] = this.merged(normalize(this.active.row, this.active.column, row, column))
    this.tabColumn = null
    this.changed()
  }

  selectRange (range, active = { row: range.r0, column: range.c0 }, add = false) {
    const whole = this.merged(range)
    if (add) {
      this.ranges.push(whole)
      this.current = this.ranges.length - 1
    } else {
      this.ranges = [whole]
      this.current = 0
    }
    this.active = { ...active }
    this.focus = active.row === whole.r0 && active.column === whole.c0
      ? { row: whole.r1, column: whole.c1 }
      : { row: whole.r0, column: whole.c0 }
    this.tabColumn = null
    this.changed()
  }

  // Whole columns (a column header, Ctrl+Space). A header click makes the
  // column's top shown cell active (`first`); inside the selection the active
  // cell stays.
  selectColumns (c0, c1, { add = false, extend = false, first = null } = {}) {
    if (extend) {
      this.focus = { row: MAX_ROWS - 1, column: c1 }
      this.ranges[this.current] = this.merged(normalize(0, this.active.column, MAX_ROWS - 1, c1))
      this.tabColumn = null
      this.changed()
      return
    }
    const range = normalize(0, c0, MAX_ROWS - 1, c1)
    const keep = !add && first === null && this.inRange(range, this.active)
    const row = keep ? this.active.row : first ?? this.topVisibleRow()
    this.selectRange(range, { row, column: keep ? this.active.column : c0 }, add)
    this.focus = { row: MAX_ROWS - 1, column: c1 }
  }

  selectRows (r0, r1, { add = false, extend = false, first = null } = {}) {
    if (extend) {
      this.focus = { row: r1, column: MAX_COLUMNS - 1 }
      this.ranges[this.current] = this.merged(normalize(this.active.row, 0, r1, MAX_COLUMNS - 1))
      this.tabColumn = null
      this.changed()
      return
    }
    const range = normalize(r0, 0, r1, MAX_COLUMNS - 1)
    const keep = !add && first === null && this.inRange(range, this.active)
    const column = keep ? this.active.column : first ?? Math.max(0, this.geometry.columns.firstVisible(0, 1))
    this.selectRange(range, { row: keep ? this.active.row : r0, column }, add)
    this.focus = { row: r1, column: MAX_COLUMNS - 1 }
  }

  selectAll () {
    const active = { ...this.active }
    this.ranges = [{ r0: 0, c0: 0, r1: MAX_ROWS - 1, c1: MAX_COLUMNS - 1 }]
    this.current = 0
    this.active = active
    this.focus = { row: MAX_ROWS - 1, column: MAX_COLUMNS - 1 }
    this.tabColumn = null
    this.changed()
  }

  topVisibleRow () {
    return Math.max(0, this.geometry.rows.firstVisible(0, 1))
  }

  inRange (range, cell) {
    return contains(range, cell.row, cell.column)
  }

  isSelected (row, column) {
    for (const range of this.ranges) if (contains(range, row, column)) return true
    return false
  }

  // The next shown cell from (row, column), stepping over hidden lines and
  // over the merge (row, column) is in.
  step (row, column, dRow, dColumn) {
    const { rows, columns } = this.geometry
    const merge = this.mergeAt(row, column)
    let r = row
    let c = column
    if (dRow > 0) r = merge ? merge.r1 : row
    if (dRow < 0) r = merge ? merge.r0 : row
    if (dColumn > 0) c = merge ? merge.c1 : column
    if (dColumn < 0) c = merge ? merge.c0 : column
    if (dRow !== 0) {
      const next = rows.firstVisible(r + dRow, dRow)
      if (next < 0) return { row, column }
      r = next
    }
    if (dColumn !== 0) {
      const next = columns.firstVisible(c + dColumn, dColumn)
      if (next < 0) return { row, column }
      c = next
    }
    return { row: r, column: c }
  }

  // An arrow key: one cell, the selection collapsing onto it.
  move (dRow, dColumn) {
    const next = this.step(this.active.row, this.active.column, dRow, dColumn)
    this.select(next.row, next.column)
  }

  // Shift+arrow: the far edge moves one shown cell, past any merge it grows by.
  extendBy (dRow, dColumn) {
    const before = { ...this.range }
    let focus = { ...this.focus }
    for (let guard = 0; guard < 64; guard++) {
      const next = this.step(focus.row, focus.column, dRow, dColumn)
      if (next.row === focus.row && next.column === focus.column) break
      focus = next
      const range = this.merged(normalize(this.active.row, this.active.column, focus.row, focus.column))
      if (!sameRange(range, before)) break
    }
    this.extendTo(focus.row, focus.column)
  }

  // Tab and Enter: inside a multi-cell selection the active cell walks it,
  // wrapping, then on to the next range; otherwise it moves one cell, and
  // Enter after Tab returns to the column the tabbing began in.
  walk (dRow, dColumn) {
    if (this.isSingle()) {
      if (dColumn !== 0) {
        if (this.tabColumn === null) this.tabColumn = this.active.column
        const column = this.tabColumn
        const next = this.step(this.active.row, this.active.column, 0, dColumn)
        this.select(next.row, next.column)
        this.tabColumn = column
        return
      }
      const column = this.tabColumn
      const next = this.step(this.active.row, this.active.column, dRow, 0)
      if (column !== null && dRow > 0) this.select(next.row, column)
      else this.select(next.row, next.column)
      return
    }
    const cells = this.walkOrder(dRow !== 0)
    let { row, column } = this.active
    for (let guard = 0; guard < 4096; guard++) {
      ;({ row, column } = cells(row, column, dRow + dColumn > 0 ? 1 : -1))
      const merge = this.mergeAt(row, column)
      if (merge && (merge.r0 !== row || merge.c0 !== column)) continue
      if (this.geometry.rows.isHidden(row) || this.geometry.columns.isHidden(column)) continue
      break
    }
    this.active = { row, column }
    this.changed()
  }

  // The next cell of the selection in row-major (Tab) or column-major (Enter)
  // order, moving between ranges at either end.
  walkOrder (byColumn) {
    return (row, column, direction) => {
      let range = this.range
      let r = row
      let c = column
      if (byColumn) r += direction
      else c += direction
      if (byColumn) {
        if (r > range.r1) { r = range.r0; c += 1 }
        if (r < range.r0) { r = range.r1; c -= 1 }
      } else {
        if (c > range.c1) { c = range.c0; r += 1 }
        if (c < range.c0) { c = range.c1; r -= 1 }
      }
      if (!contains(range, r, c)) {
        this.current = (this.current + direction + this.ranges.length) % this.ranges.length
        range = this.range
        r = direction > 0 ? range.r0 : range.r1
        c = direction > 0 ? range.c0 : range.c1
      }
      return { row: r, column: c }
    }
  }

  // The reference the Name Box shows.
  get activeRef () {
    return refName(this.active.row, this.active.column)
  }

  // The ranges as the service names them, A1 text.
  get refs () {
    return this.ranges.map(rangeName)
  }
}
