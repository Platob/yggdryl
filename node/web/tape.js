// Time and sales: the served executions, newest first as the service ordered
// them, never re-sorted. The clock shows every digit of the instant - the
// milliseconds `formatClock` gives, then the six below them - so two prints
// one nanosecond apart read as two instants. Only the rows in view are in the
// DOM (`virtual.js`), keyed by `curruuid`.

import { Component, element } from './component.js'
import { formatDecimal } from './decimal.js'
import { formatClock, formatInstant, instantParts } from './instant.js'
import { VirtualRows } from './virtual.js'

/**
 * Display only: which glyph a served side wears. The text shown is always the
 * served side, and nothing is decided by this map. It mirrors the core's
 * `Side::is_bid`/`is_ask` split because no reading of a side's lane is served
 * - the Node binding answers none (`new BookSide(side)` accepts exactly these
 * seven and names no lane) - and goes once a row states its lane. Any other
 * side shows no glyph.
 */
const LANES = { BUY: 'bid', BUYMINUS: 'bid', SELL: 'ask', SELLPLUS: 'ask', SSHORT: 'ask', SSHORTEX: 'ask', SELLUND: 'ask' }

/** `HH:MM:SS.mmm` then the microsecond and nanosecond digits. */
function clockParts(ns) {
  const below = instantParts(ns).nanos % 1_000_000n
  return [formatClock(ns), below.toString().padStart(6, '0')]
}

export class Tape extends Component {
  constructor({ rows = 30, rowHeight = 22 } = {}) {
    super({ rows, rowHeight })
    this.executions = []
    this.cells = new WeakMap()
    this.drawn = undefined
  }

  render() {
    const { rows, rowHeight } = this.props
    const root = element('section', 'ygg-ui__tape')
    root.setAttribute('aria-label', 'Time and sales')
    this.summary = element('p', 'ygg-ui__tape-summary ygg-ui__muted')
    this.summary.setAttribute('aria-live', 'polite')
    this.grid = element('div', 'ygg-ui__tape-grid')
    this.grid.setAttribute('role', 'grid')
    this.grid.setAttribute('aria-label', 'Executions, newest first')
    this.grid.setAttribute('aria-colcount', '5')
    this.grid.tabIndex = 0
    const header = element('div', 'ygg-ui__tape-row ygg-ui__tape-head')
    header.setAttribute('role', 'row')
    header.setAttribute('aria-rowindex', '1')
    for (const [name, numeric] of [['Time', false], ['Price', true], ['Quantity', true], ['Side', false], ['Id', true]]) {
      const cell = element('div', numeric ? 'ygg-ui__num' : '', name)
      cell.setAttribute('role', 'columnheader')
      header.append(cell)
    }
    this.grid.append(header)
    this.list = new VirtualRows(this.grid, {
      rows,
      rowHeight,
      key: (execution) => execution.curruuid,
      create: () => this.createRow(),
      patch: (row, execution) => this.patch(row, execution),
    })
    root.append(this.summary, this.grid)
    return root
  }

  onMount() {
    this.listen(this.list.viewport, 'scroll', () => this.schedule(), { passive: true })
    this.listen(this.list.layer, 'click', (event) => {
      const index = this.list.indexOf(event.target)
      if (index < 0) return
      this.list.active = index
      this.select(index)
      this.schedule()
    })
    this.listen(this.grid, 'keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault()
        event.stopPropagation()
        if (this.list.active >= 0) this.select(this.list.active)
      } else if (this.list.navigate(event)) this.schedule()
    })
    this.listen(this.grid, 'focus', () => {
      if (this.list.active < 0 && this.executions.length) this.list.active = 0
      this.schedule()
    })
  }

  draw(state) {
    if (state !== this.drawn) {
      this.drawn = state
      this.executions = state?.executions ?? []
      this.list.setItems(this.executions)
      const sentence = this.sentence()
      if (this.summary.textContent !== sentence) this.summary.textContent = sentence
    }
    this.list.render()
  }

  sentence() {
    const last = this.executions[0]
    if (!last) return 'No executions'
    const price = last.lastpx === null || last.lastpx === undefined ? 'no price' : formatDecimal(last.lastpx)
    const quantity = last.lastqty === null || last.lastqty === undefined ? 'no quantity' : formatDecimal(last.lastqty)
    return `Last print ${price} x ${quantity}, ${last.side ?? 'no side'}, at ${clockParts(last.currunix).join('')}`
  }

  createRow() {
    const row = element('div', 'ygg-ui__tape-row')
    const time = element('div', 'ygg-ui__num ygg-ui__tape-time')
    const stamp = element('time')
    const clock = element('span')
    const below = element('span', 'ygg-ui__muted')
    stamp.append(clock, below)
    time.append(stamp)
    const price = element('div', 'ygg-ui__num')
    const quantity = element('div', 'ygg-ui__num')
    const side = element('div', 'ygg-ui__tape-side')
    const id = element('div', 'ygg-ui__num ygg-ui__muted ygg-ui__tape-id')
    for (const cell of [time, price, quantity, side, id]) cell.setAttribute('role', 'gridcell')
    row.append(time, price, quantity, side, id)
    this.cells.set(row, { stamp, clock, below, price, quantity, side, id })
    return row
  }

  patch(row, execution) {
    const cells = this.cells.get(row)
    const [clock, below] = clockParts(execution.currunix)
    const stamp = formatInstant(execution.currunix)
    if (cells.stamp.getAttribute('datetime') !== stamp) cells.stamp.setAttribute('datetime', stamp)
    setText(cells.clock, clock)
    setText(cells.below, below)
    setText(cells.price, execution.lastpx === null || execution.lastpx === undefined ? '' : formatDecimal(execution.lastpx))
    setText(cells.quantity, execution.lastqty === null || execution.lastqty === undefined ? '' : formatDecimal(execution.lastqty))
    const lane = LANES[execution.side]
    cells.side.classList.toggle('ygg-ui__bid', lane === 'bid')
    cells.side.classList.toggle('ygg-ui__ask', lane === 'ask')
    setText(cells.side, execution.side ?? '')
    setText(cells.id, String(execution.curruuid ?? '').slice(0, 8))
    cells.id.title = execution.curruuid ?? ''
  }

  select(index) {
    const execution = this.executions[index]
    if (!execution) return
    this.emit('ygg:select-element', { crosscode: execution.crosscode, curruuid: execution.curruuid })
  }
}

function setText(node, text) {
  if (node.textContent !== text) node.textContent = text
}
