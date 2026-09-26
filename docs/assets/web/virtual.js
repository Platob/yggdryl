// The rows of a long list with only the visible window in the DOM: a spacer
// carries the full height, the window's rows are translated into place, and
// each row is keyed so an update patches the same element in place. It also
// owns the active row a grid's keyboard moves - `aria-activedescendant`, the
// arrows, Page keys, Home and End - and that row is an item, held by its key:
// an update that inserts above it moves it with its item. What a row shows is
// its owner's business.

import { element } from './component.js'

let serial = 0

export class VirtualRows {
  /**
   * `grid` is the `role="grid"` element the rows go in (after its header);
   * `key(item, index)` names a row, `create()` builds one, `patch(row, item,
   * index)` fills it.
   */
  constructor(grid, { rows, rowHeight, headerRows = 1, overscan = 6, key, create, patch }) {
    this.grid = grid
    this.options = { rows, rowHeight, headerRows, overscan }
    this.key = key
    this.create = create
    this.patch = patch
    this.id = `ygg-ui-rows-${++serial}`
    this.rowSerial = 0
    this.items = []
    this.byKey = new Map()
    this._active = -1
    // The active item's key and its ordinal among the items under that key.
    this.activeKey = null
    this.viewport = element('div', 'ygg-ui__virtual-viewport')
    this.viewport.setAttribute('role', 'rowgroup')
    this.viewport.style.height = `${rows * rowHeight}px`
    this.spacer = element('div', 'ygg-ui__virtual-spacer')
    this.spacer.setAttribute('role', 'presentation')
    this.layer = element('div', 'ygg-ui__virtual-layer')
    this.layer.setAttribute('role', 'presentation')
    this.spacer.append(this.layer)
    this.viewport.append(this.spacer)
    grid.append(this.viewport)
    grid.style.setProperty('--ygg-ui-row', `${rowHeight}px`)
    grid.setAttribute('aria-rowcount', String(headerRows))
  }

  /** The index of the active item, or -1; setting it makes that item the one followed. */
  get active() {
    return this._active
  }

  set active(index) {
    this._active = index
    this.activeKey = index >= 0 && index < this.items.length ? this.identify(index) : null
  }

  /** Item `index`'s key and how many items before it share that key. */
  identify(index) {
    const key = String(this.key(this.items[index], index))
    let ordinal = 0
    for (let at = 0; at < index; at += 1) if (String(this.key(this.items[at], at)) === key) ordinal += 1
    return { key, ordinal }
  }

  /** Where the item `identify` named sits among `items`, or -1. */
  locate({ key, ordinal }) {
    let seen = 0
    for (let at = 0; at < this.items.length; at += 1) {
      if (String(this.key(this.items[at], at)) === key && seen++ === ordinal) return at
    }
    return -1
  }

  /**
   * Take the items to show, in the order they are to be shown. The active
   * item stays active where it moved to; when it is gone, the row that took
   * its place is.
   */
  setItems(items) {
    this.items = items
    this.spacer.style.height = `${items.length * this.options.rowHeight}px`
    this.grid.setAttribute('aria-rowcount', String(items.length + this.options.headerRows))
    if (this.activeKey === null) return
    const moved = this.locate(this.activeKey)
    this.active = moved >= 0 ? moved : Math.min(this._active, items.length - 1)
  }

  /** Lay out the window of rows the viewport shows. */
  render() {
    const { rows, rowHeight, headerRows, overscan } = this.options
    const count = this.items.length
    const first = Math.max(0, Math.min(Math.floor(this.viewport.scrollTop / rowHeight) - overscan, count - rows - overscan))
    const last = Math.min(count, first + rows + 2 * overscan)
    this.layer.style.transform = `translateY(${first * rowHeight}px)`
    const next = new Map()
    const nodes = []
    for (let index = first; index < last; index += 1) {
      const item = this.items[index]
      let key = String(this.key(item, index))
      // Two items under one key stay two rows.
      while (next.has(key)) key += '#'
      let row = this.byKey.get(key)
      if (!row) {
        row = this.create()
        row.setAttribute('role', 'row')
        row.id = `${this.id}-${++this.rowSerial}`
      }
      next.set(key, row)
      row.dataset.index = String(index)
      row.setAttribute('aria-rowindex', String(index + headerRows + 1))
      row.setAttribute('aria-selected', String(index === this.active))
      row.classList.toggle('ygg-ui__active', index === this.active)
      this.patch(row, item, index)
      nodes.push(row)
    }
    this.byKey = next
    const children = this.layer.children
    if (children.length !== nodes.length || nodes.some((node, at) => children[at] !== node)) this.layer.replaceChildren(...nodes)
    const active = this.rowAt(this.active)
    if (active) this.grid.setAttribute('aria-activedescendant', active.id)
    else this.grid.removeAttribute('aria-activedescendant')
  }

  /** The rendered row of item `index`, or null when it is out of the window. */
  rowAt(index) {
    if (index < 0) return null
    for (const row of this.byKey.values()) if (Number(row.dataset.index) === index) return row
    return null
  }

  /** The item index a node inside a row belongs to, or -1. */
  indexOf(node) {
    const start = node?.nodeType === Node.ELEMENT_NODE ? node : node?.parentElement
    const row = start?.closest('[role="row"]')
    return row && this.layer.contains(row) ? Number(row.dataset.index) : -1
  }

  /** Move the active row for a navigation key; answers whether the key was one (and consumes it). */
  navigate(event) {
    const count = this.items.length
    const page = this.options.rows
    const steps = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page }
    let next
    if (event.key in steps) next = this.active < 0 ? 0 : this.active + steps[event.key]
    else if (event.key === 'Home') next = 0
    else if (event.key === 'End') next = count - 1
    else return false
    event.preventDefault()
    event.stopPropagation()
    if (!count) return true
    this.active = Math.min(count - 1, Math.max(0, next))
    this.reveal(this.active)
    return true
  }

  /** Scroll so item `index` is inside the view. */
  reveal(index) {
    const { rows, rowHeight } = this.options
    const top = index * rowHeight
    const view = this.viewport
    if (top < view.scrollTop) view.scrollTop = top
    else if (top + rowHeight > view.scrollTop + rows * rowHeight) view.scrollTop = top + rowHeight - rows * rowHeight
  }
}
