// Cumulative depth of the two served sides on one canvas: x is the price, y
// the running quantity along each side's `limits` in the order served - best
// first - so a bid steps down and to the left and an ask up and to the right.
// The running total is the exact text sum of the served quantities; a float is
// read from it only to place a pixel. The unpriced limit adds to the total and
// has no price to stand at, so it is counted and not drawn.

import { fitCanvas, tickLabel, tokens, watchSize, watchTheme } from './canvas.js'
import { Component, element } from './component.js'
import { formatDecimal, maxDecimal, minDecimal, sumDecimal, toFloat } from './decimal.js'
import { linear, padded, ticks } from './scales.js'

const MARGIN = { top: 10, right: 10, bottom: 22, left: 48 }
const COLOURS = ['bid', 'bid-soft', 'ask', 'ask-soft', 'muted', 'line', 'text']

/** The running totals of a side's served limits, in served order: `[{ price, total }]`, exact text. */
export function cumulative(limits = []) {
  let running = '0'
  return limits.map((limit) => {
    running = sumDecimal([running, limit.quantity])
    return { price: limit.price ?? null, total: running }
  })
}

export class DepthChart extends Component {
  constructor({ height = 220 } = {}) {
    super({ height })
    this.steps = { bid: [], ask: [] }
    this.drawn = undefined
    this.stops = []
  }

  render() {
    const root = element('figure', 'ygg-ui__chart ygg-ui__depth')
    this.canvas = element('canvas', 'ygg-ui__chart-canvas')
    this.canvas.setAttribute('role', 'img')
    this.canvas.setAttribute('aria-label', 'Depth: no book')
    this.canvas.style.height = `${this.props.height}px`
    root.append(this.canvas)
    return root
  }

  onMount() {
    this.stops = [watchSize(this.canvas, () => this.schedule()), watchTheme(() => this.schedule())]
  }

  onDestroy() {
    for (const stop of this.stops.splice(0)) stop()
  }

  draw(state) {
    if (state !== this.drawn) {
      this.drawn = state
      this.sides = { bid: state?.bid ?? {}, ask: state?.ask ?? {} }
      this.steps = { bid: cumulative(this.sides.bid.limits), ask: cumulative(this.sides.ask.limits) }
      this.canvas.setAttribute('aria-label', this.sentence())
    }
    this.paint()
  }

  sentence() {
    const { bid, ask } = this.steps
    if (!bid.length && !ask.length) return 'Depth: no limits on either side'
    const best = (name) => {
      const side = this.sides[name]
      return side.price === null || side.price === undefined
        ? `no best ${name}`
        : `best ${name} ${formatDecimal(side.price)} x ${formatDecimal(side.quantity)}`
    }
    const depth = (name) => {
      const steps = this.steps[name]
      return steps.length ? `${name} depth ${steps.at(-1).total} over ${steps.length} ${steps.length === 1 ? 'limit' : 'limits'}` : `no ${name} limits`
    }
    return `Depth: ${best('bid')}, ${best('ask')}; ${depth('bid')}, ${depth('ask')}`
  }

  paint() {
    const { context, width, height } = fitCanvas(this.canvas)
    const colour = tokens(this.el, COLOURS)
    const plot = { left: MARGIN.left, right: width - MARGIN.right, top: MARGIN.top, bottom: height - MARGIN.bottom }
    const priced = (steps) => steps.filter((step) => step.price !== null)
    const bid = priced(this.steps.bid)
    const ask = priced(this.steps.ask)
    context.font = '11px ui-monospace, monospace'
    if (!bid.length && !ask.length) {
      context.fillStyle = colour.muted
      context.fillText('No priced depth', plot.left, (plot.top + plot.bottom) / 2)
      return
    }
    const prices = [...bid, ...ask].map((step) => step.price)
    const [lo, hi] = padded(toFloat(minDecimal(...prices)), toFloat(maxDecimal(...prices)), 0.05)
    const top = toFloat(maxDecimal(...[...bid, ...ask].map((step) => step.total))) * 1.08
    const x = linear(lo, hi, plot.left, plot.right)
    const y = linear(0, top, plot.bottom, plot.top)

    context.strokeStyle = colour.line
    context.fillStyle = colour.muted
    context.lineWidth = 1
    context.textAlign = 'center'
    context.textBaseline = 'top'
    for (const value of ticks(lo, hi, Math.max(2, Math.floor((plot.right - plot.left) / 90)))) {
      const at = Math.round(x(value)) + 0.5
      context.beginPath()
      context.moveTo(at, plot.top)
      context.lineTo(at, plot.bottom)
      context.stroke()
      context.fillText(tickLabel(value), at, plot.bottom + 5)
    }
    context.textAlign = 'right'
    context.textBaseline = 'middle'
    for (const value of ticks(0, top, 4)) {
      const at = Math.round(y(value)) + 0.5
      context.beginPath()
      context.moveTo(plot.left, at)
      context.lineTo(plot.right, at)
      context.stroke()
      context.fillText(tickLabel(value), plot.left - 6, at)
    }

    this.side(context, bid, x, y, plot.left, colour.bid, colour['bid-soft'])
    this.side(context, ask, x, y, plot.right, colour.ask, colour['ask-soft'])
  }

  /** One side's steps from its best price out to `edge`, filled down to zero. */
  side(context, steps, x, y, edge, stroke, fill) {
    if (!steps.length) return
    const zero = y(0)
    const path = new Path2D()
    let level = zero
    path.moveTo(x(toFloat(steps[0].price)), zero)
    for (const step of steps) {
      const at = x(toFloat(step.price))
      path.lineTo(at, level)
      level = y(toFloat(step.total))
      path.lineTo(at, level)
    }
    path.lineTo(edge, level)
    const area = new Path2D(path)
    area.lineTo(edge, zero)
    area.closePath()
    context.fillStyle = fill
    context.fill(area)
    context.strokeStyle = stroke
    context.lineWidth = 2
    context.stroke(path)
  }
}
