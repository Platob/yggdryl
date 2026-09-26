// One side of a book as the service served it: `side.limits`, one row per
// limit in the order given - best first, the unpriced limit last - and
// nothing sorted or summed here. Only the rows in view are in the DOM
// (`virtual.js`), keyed by price text so an update patches them in place.

import { Component, element } from './component.js'
import { compareDecimal, formatDecimal, toFloat } from './decimal.js'
import { Tooltip } from './tooltip.js'
import { VirtualRows } from './virtual.js'

const UNPRICED = '∅'

export class Ladder extends Component {
  constructor({ side = 'bid', rows = 40, rowHeight = 24, tooltip } = {}) {
    if (side !== 'bid' && side !== 'ask') throw new TypeError(`expected side 'bid' or 'ask', got ${JSON.stringify(side)}`)
    super({ side, rows, rowHeight })
    this.limits = []
    this.side = {}
    this.cells = new WeakMap()
    this.hovered = null
    this.pointing = false
    this.dismissed = false
    this.drawn = undefined
    this.tooltip = tooltip ?? null
    this.ownsTooltip = !tooltip
  }

  get label() {
    return this.props.side === 'bid' ? 'Bid' : 'Ask'
  }

  /** Bid cells read orders, quantity, price - the price beside the spread; ask the reverse. */
  columns(cells) {
    return this.props.side === 'bid' ? [...cells].reverse() : cells
  }

  render() {
    const { side, rows, rowHeight } = this.props
    const root = element('section', `ygg-ui__ladder ygg-ui__ladder--${side}`)
    root.setAttribute('aria-label', `${this.label} ladder`)
    this.summary = element('p', 'ygg-ui__ladder-summary ygg-ui__muted')
    this.summary.setAttribute('aria-live', 'polite')
    this.grid = element('div', 'ygg-ui__ladder-grid')
    this.grid.setAttribute('role', 'grid')
    this.grid.setAttribute('aria-label', `${this.label} limits`)
    this.grid.setAttribute('aria-colcount', '3')
    this.grid.tabIndex = 0
    const header = element('div', 'ygg-ui__ladder-row ygg-ui__ladder-head')
    header.setAttribute('role', 'row')
    header.setAttribute('aria-rowindex', '1')
    for (const name of this.columns(['Price', 'Quantity', 'Orders'])) {
      const cell = element('div', 'ygg-ui__num', name)
      cell.setAttribute('role', 'columnheader')
      header.append(cell)
    }
    this.grid.append(header)
    this.list = new VirtualRows(this.grid, {
      rows,
      rowHeight,
      key: (limit) => (limit.price === null || limit.price === undefined ? UNPRICED : String(limit.price)),
      create: () => this.createRow(),
      patch: (row, limit) => this.patch(row, limit),
    })
    root.append(this.summary, this.grid)
    return root
  }

  onMount() {
    if (this.ownsTooltip) this.tooltip = new Tooltip().mount(document.body)
    const layer = this.list.layer
    this.listen(this.list.viewport, 'scroll', () => this.schedule(), { passive: true })
    this.listen(layer, 'click', (event) => {
      const index = this.list.indexOf(event.target)
      if (index < 0) return
      this.list.active = index
      this.pick(index)
      this.schedule()
    })
    this.listen(layer, 'pointerover', (event) => {
      const row = event.target.closest?.('[role="row"]')
      if (!row || row === this.hovered) return
      this.hovered = row
      this.pointing = true
      this.dismissed = false
      this.describe()
    })
    this.listen(layer, 'pointerleave', () => {
      this.hovered = null
      this.pointing = false
      this.describe()
    })
    this.listen(this.grid, 'keydown', (event) => this.onKey(event))
    this.listen(this.grid, 'focus', () => {
      if (this.list.active < 0 && this.limits.length) this.list.active = 0
      this.pointing = false
      this.schedule()
    })
    this.listen(this.grid, 'blur', () => this.describe())
  }

  onDestroy() {
    if (this.ownsTooltip) this.tooltip?.destroy()
  }

  draw(state) {
    if (state !== this.drawn) {
      this.drawn = state
      this.side = state?.side ?? {}
      this.limits = this.side.limits ?? []
      // A consolidated book names each entry's own ticker: a lookup by uuid.
      this.tickers = state?.global
        ? new Map((this.side.live ?? []).filter((entry) => entry.ticker != null).map((entry) => [entry.curruuid, entry.ticker]))
        : null
      let widest
      for (const limit of this.limits) if (widest === undefined || compareDecimal(limit.quantity, widest) > 0) widest = limit.quantity
      this.widest = widest === undefined ? 0 : toFloat(widest)
      this.list.setItems(this.limits)
      this.grid.setAttribute('aria-label', state?.symbol ? `${this.label} limits, ${state.symbol}` : `${this.label} limits`)
      const sentence = this.sentence()
      if (this.summary.textContent !== sentence) this.summary.textContent = sentence
    }
    this.list.render()
    if (this.hovered && !this.list.layer.contains(this.hovered)) this.hovered = null
    this.describe()
  }

  /** "Bid: best 100.5 x 8, 4 limits", from the side's served best. */
  sentence() {
    const count = this.limits.length
    if (!count) return `${this.label}: no limits`
    const { price, quantity } = this.side
    const best = price === null || price === undefined ? 'none' : `${formatDecimal(price)} x ${formatDecimal(quantity)}`
    return `${this.label}: best ${best}, ${count} ${count === 1 ? 'limit' : 'limits'}`
  }

  createRow() {
    const side = this.props.side
    const row = element('div', 'ygg-ui__ladder-row')
    const fill = element('div', `ygg-ui__ladder-fill ygg-ui__fill-${side}`)
    fill.setAttribute('aria-hidden', 'true')
    const price = element('div', `ygg-ui__num ygg-ui__${side}`)
    const quantity = element('div', 'ygg-ui__num')
    const count = element('div', 'ygg-ui__num ygg-ui__muted')
    for (const cell of [price, quantity, count]) cell.setAttribute('role', 'gridcell')
    row.append(fill, ...this.columns([price, quantity, count]))
    this.cells.set(row, { price, quantity, count })
    return row
  }

  patch(row, limit) {
    const cells = this.cells.get(row)
    const unpriced = limit.price === null || limit.price === undefined
    row.classList.toggle('ygg-ui__unpriced', unpriced)
    setText(cells.price, unpriced ? UNPRICED : formatDecimal(limit.price))
    if (unpriced) cells.price.setAttribute('aria-label', 'no price')
    else cells.price.removeAttribute('aria-label')
    setText(cells.quantity, formatDecimal(limit.quantity))
    setText(cells.count, String(limit.uuids?.length ?? 0))
    // The bar is a pixel share of the widest served quantity, nothing more.
    const share = this.widest > 0 ? Math.min(100, (toFloat(limit.quantity) / this.widest) * 100) : 0
    row.style.setProperty('--ygg-ui-fill', `${share.toFixed(2)}%`)
  }

  onKey(event) {
    if (event.key === 'Escape') {
      this.dismissed = true
      return
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      event.stopPropagation()
      if (this.list.active >= 0) this.pick(this.list.active)
      return
    }
    if (this.list.navigate(event)) {
      this.pointing = false
      this.dismissed = false
      this.schedule()
    }
  }

  pick(index) {
    const limit = this.limits[index]
    if (!limit) return
    this.emit('ygg:limit-select', { price: limit.price ?? null, uuids: [...(limit.uuids ?? [])] })
  }

  /** The tooltip follows the pointer's row, else the focused active row. */
  describe() {
    const tooltip = this.tooltip
    if (!tooltip) return
    const focused = document.activeElement === this.grid
    const row = this.pointing ? this.hovered : focused ? this.list.rowAt(this.list.active) : null
    if (!row || this.dismissed) {
      const target = tooltip.target
      if (target && (this.list.layer.contains(target) || !target.isConnected)) tooltip.hide()
      return
    }
    tooltip.show(row, this.content(this.limits[Number(row.dataset.index)]))
  }

  /** The uuids of one limit, grouped by the served ticker on a consolidated book. */
  content(limit) {
    const box = element('div', 'ygg-ui__tooltip-body')
    const uuids = limit?.uuids ?? []
    const price = limit?.price === null || limit?.price === undefined ? 'no price' : formatDecimal(limit.price)
    box.append(element('div', 'ygg-ui__tooltip-title', `${uuids.length} ${uuids.length === 1 ? 'entry' : 'entries'} at ${price}`))
    if (this.tickers?.size) {
      const groups = new Map()
      for (const id of uuids) {
        const ticker = this.tickers.get(id) ?? UNPRICED
        if (!groups.has(ticker)) groups.set(ticker, [])
        groups.get(ticker).push(id)
      }
      for (const [ticker, ids] of groups) {
        const group = element('div', 'ygg-ui__tooltip-group')
        group.append(element('span', 'ygg-ui__tooltip-key', ticker), list(ids))
        box.append(group)
      }
    } else {
      box.append(list(uuids))
    }
    return box
  }
}

function list(items) {
  const node = element('ul', 'ygg-ui__tooltip-list')
  for (const item of items) node.append(element('li', undefined, item))
  return node
}

function setText(node, text) {
  if (node.textContent !== text) node.textContent = text
}
