// The grid's keymap (§10) for everything that moves or selects without an
// edit. Ctrl+arrows and Ctrl+A ask the service (`edge`), so the data rule is
// the server's; the keys resolve in the order they were pressed.

import { parseRef, parseRange, refName, MAX_ROWS, MAX_COLUMNS } from './geometry.js'

const ARROWS = {
  ArrowUp: [-1, 0, 'up'],
  ArrowDown: [1, 0, 'down'],
  ArrowLeft: [0, -1, 'left'],
  ArrowRight: [0, 1, 'right']
}

export class Keyboard {
  constructor ({ target, grid, selection, api, app }) {
    this.grid = grid
    this.selection = selection
    this.api = api
    this.app = app
    this.chain = Promise.resolve()
    this.handlers = []
    target.addEventListener('keydown', (event) => this.keydown(event))
  }

  // Later phases put their keys (editing, clipboard, formatting) ahead of
  // navigation; a handler answering true has taken the key.
  use (handler) {
    this.handlers.push(handler)
  }

  // Keys resolve in order, even when one waits on the service.
  queue (action) {
    this.chain = this.chain.then(action).catch(() => {})
    return this.chain
  }

  keydown (event) {
    for (const handler of this.handlers) if (handler(event)) return
    if (this.navigate(event)) event.preventDefault()
  }

  sheetKey () {
    return this.app.sheetKey()
  }

  reveal (cell) {
    this.grid.ensureVisible(cell.row, cell.column)
  }

  navigate (event) {
    if (!this.grid.geometry) {
      if ((event.ctrlKey || event.metaKey) && (event.key === 'PageUp' || event.key === 'PageDown')) {
        this.app.switchSheet(event.key === 'PageUp' ? -1 : 1)
        return true
      }
      return false
    }
    const ctrl = event.ctrlKey || event.metaKey
    const { shiftKey: shift, altKey: alt, key } = event
    const selection = this.selection
    const { geometry } = this.grid

    if (key in ARROWS && !alt) {
      const [dRow, dColumn, direction] = ARROWS[key]
      if (ctrl) {
        this.queue(() => this.edge(direction, shift))
        return true
      }
      this.queue(() => {
        if (shift) {
          selection.extendBy(dRow, dColumn)
          this.reveal(selection.focus)
        } else {
          selection.move(dRow, dColumn)
          this.reveal(selection.active)
        }
      })
      return true
    }

    switch (key) {
      case 'Home': {
        this.queue(() => {
          const column = Math.max(0, geometry.columns.firstVisible(geometry.frozen.columns, 1))
          const row = ctrl ? Math.max(0, geometry.rows.firstVisible(geometry.frozen.rows, 1)) : shift ? selection.focus.row : selection.active.row
          if (shift) selection.extendTo(row, column)
          else selection.select(row, column)
          this.grid.scrollTo(ctrl ? geometry.frozen.rows : null, geometry.frozen.columns)
          this.reveal(shift ? selection.focus : selection.active)
        })
        return true
      }
      case 'End': {
        if (!ctrl) return false
        this.queue(() => {
          const { r1, c1 } = geometry.dimension
          if (shift) selection.extendTo(r1, c1)
          else selection.select(r1, c1)
          this.reveal({ row: r1, column: c1 })
        })
        return true
      }
      case 'PageUp':
      case 'PageDown': {
        const direction = key === 'PageUp' ? -1 : 1
        if (ctrl) {
          this.app.switchSheet(direction)
          return true
        }
        this.queue(() => this.page(direction, alt, shift))
        return true
      }
      case 'Tab': {
        if (ctrl || alt) return false
        this.queue(() => {
          selection.walk(0, shift ? -1 : 1)
          this.reveal(selection.active)
        })
        return true
      }
      case 'Enter': {
        if (ctrl || alt) return false
        this.queue(() => {
          selection.walk(shift ? -1 : 1, 0)
          this.reveal(selection.active)
        })
        return true
      }
      case 'a':
      case 'A': {
        if (!ctrl || alt) return false
        this.queue(() => this.selectAll())
        return true
      }
      case ' ': {
        if (ctrl && !shift) {
          const range = selection.range
          this.queue(() => selection.selectColumns(range.c0, range.c1, {}))
          return true
        }
        if (shift && !ctrl) {
          const range = selection.range
          this.queue(() => selection.selectRows(range.r0, range.r1, {}))
          return true
        }
        if (shift && ctrl) {
          this.queue(() => selection.selectAll())
          return true
        }
        return false
      }
    }
    return false
  }

  // Ctrl+arrow: the service answers where the data edge is; with Shift the
  // range grows from its far corner.
  async edge (direction, extend) {
    const selection = this.selection
    const from = extend ? selection.focus : selection.active
    const ref = this.refOf(from)
    const sheet = this.sheetKey()
    const answer = await this.api.get(`sheets/${sheet}/edge?from=${ref}&direction=${direction}`)
    if (sheet !== this.sheetKey() || !answer || !answer.ref) return
    const to = parseRef(answer.ref)
    if (!to) return
    if (extend) {
      selection.extendTo(to.row, to.column)
      this.reveal(selection.focus)
    } else {
      selection.select(to.row, to.column)
      this.reveal(selection.active)
    }
  }

  refOf (cell) {
    return refName(Math.min(cell.row, MAX_ROWS - 1), Math.min(cell.column, MAX_COLUMNS - 1))
  }

  // PgUp/PgDn move the view and the active cell one pane; Alt pages across.
  page (direction, across, extend) {
    const { grid, selection } = this
    const { rows, columns } = grid.geometry
    const from = extend ? selection.focus : selection.active
    if (across) {
      const count = grid.pageColumns(1)
      const column = columns.advance(from.column, count * direction)
      grid.scrollTo(null, columns.advance(grid.scroll.leftColumn, count * direction))
      if (extend) selection.extendTo(from.row, column)
      else selection.select(from.row, column)
    } else {
      const count = grid.pageRows(1)
      const row = rows.advance(from.row, count * direction)
      grid.scrollTo(rows.advance(grid.scroll.topRow, count * direction), null)
      if (extend) selection.extendTo(row, from.column)
      else selection.select(row, from.column)
    }
    this.reveal(extend ? selection.focus : selection.active)
  }

  // Ctrl+A: the current region, then the whole sheet.
  async selectAll () {
    const selection = this.selection
    const sheet = this.sheetKey()
    const answer = await this.api.get(`sheets/${sheet}/edge?from=${this.refOf(selection.active)}&direction=region`)
    if (sheet !== this.sheetKey()) return
    const found = answer && answer.range ? parseRange(answer.range) : null
    const region = found && selection.merged(found)
    const current = selection.range
    if (!region || selection.ranges.length > 1 ||
      (region.r0 === current.r0 && region.c0 === current.c0 && region.r1 === current.r1 && region.c1 === current.c1) ||
      (region.r0 === region.r1 && region.c0 === region.c1)) {
      selection.selectAll()
      return
    }
    selection.selectRange(region, { ...selection.active })
  }
}
