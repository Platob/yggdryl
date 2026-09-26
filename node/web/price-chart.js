// Prices over time for the books in a window: the served `bboMidpoint` of each
// book as a step line, candles folded by `candles.js` from the executions the
// books carry, and one marker per execution at its `lastpx`. There is no
// last-price line: a book row states no `lastpx`, and a book's executions are
// ordered by uuid, so the last one held is no latest print - the markers show
// every print. Time is a bigint nanosecond instant: a pixel is a difference
// from `window.from`, scaled; nothing converts an instant to a number.

import { aggregateCandles } from './candles.js'
import { fitCanvas, tickLabel, tokens, watchSize, watchTheme } from './canvas.js'
import { Component, element } from './component.js'
import { compareDecimal, formatDecimal, maxDecimal, minDecimal, toFloat } from './decimal.js'
import { compareInstant, formatClock, formatInstant, fromPixels, instantText, nanosPerPixel, parseInstant, toPixels } from './instant.js'
import { linear, padded, ticks } from './scales.js'

const MARGIN = { top: 10, right: 12, bottom: 22, left: 58 }
const COLOURS = ['bid', 'bid-soft', 'ask', 'ask-soft', 'muted', 'line', 'text', 'focus']
const PAGE_BUCKETS = 10n

export class PriceChart extends Component {
  constructor({ bucketNs = 1_000_000_000n, height = 240 } = {}) {
    const bucket = typeof bucketNs === 'bigint' ? bucketNs : parseInstant(String(bucketNs))
    if (bucket <= 0n) throw new TypeError('expected a positive bucket width in nanoseconds')
    super({ bucketNs: bucket, height })
    this.series = { midpoint: [], executions: [] }
    this.candles = []
    this.books = []
    this.cross = null
    this.from = 0n
    this.to = 0n
    this.end = 0n
    this.drawn = undefined
    this.stops = []
  }

  render() {
    const root = element('figure', 'ygg-ui__chart ygg-ui__price')
    this.canvas = element('canvas', 'ygg-ui__chart-canvas')
    this.canvas.setAttribute('role', 'img')
    this.canvas.setAttribute('aria-label', 'Prices: no books')
    this.canvas.style.height = `${this.props.height}px`
    // The readout is the keyboard's crosshair: a slider over the window's buckets.
    this.readout = element('div', 'ygg-ui__chart-readout ygg-ui__num')
    this.readout.setAttribute('role', 'slider')
    this.readout.setAttribute('aria-label', 'Crosshair instant')
    this.readout.setAttribute('aria-valuemin', '0')
    this.readout.tabIndex = 0
    root.append(this.canvas, this.readout)
    return root
  }

  onMount() {
    this.stops = [watchSize(this.canvas, () => this.schedule()), watchTheme(() => this.schedule())]
    this.listen(this.canvas, 'pointermove', (event) => {
      this.cross = this.instantAt(event.offsetX)
      this.schedule()
    })
    this.listen(this.canvas, 'pointerleave', () => {
      if (document.activeElement !== this.readout) {
        this.cross = null
        this.schedule()
      }
    })
    this.listen(this.canvas, 'click', (event) => {
      this.cross = this.instantAt(event.offsetX)
      this.seek()
      this.schedule()
    })
    this.listen(this.readout, 'keydown', (event) => this.onKey(event))
  }

  onDestroy() {
    for (const stop of this.stops.splice(0)) stop()
  }

  /** The instant under a pixel offset of the canvas, inside the window. */
  instantAt(offsetX) {
    const at = fromPixels(offsetX - MARGIN.left, this.from, this.perPixel ?? 1n)
    return instantText(at < this.from ? this.from : at > this.to ? this.to : at)
  }

  onKey(event) {
    const bucket = this.props.bucketNs
    const held = this.cross === null ? this.from : parseInstant(this.cross)
    const moves = { ArrowRight: bucket, ArrowLeft: -bucket, PageDown: bucket * PAGE_BUCKETS, PageUp: -bucket * PAGE_BUCKETS }
    let next
    if (event.key in moves) next = held + moves[event.key]
    else if (event.key === 'Home') next = this.from
    else if (event.key === 'End') next = this.to
    else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      event.stopPropagation()
      if (this.cross === null) this.cross = instantText(held)
      this.seek()
      return
    } else return
    event.preventDefault()
    event.stopPropagation()
    this.cross = instantText(next < this.from ? this.from : next > this.to ? this.to : next)
    this.schedule()
  }

  seek() {
    if (this.cross !== null) this.emit('ygg:seek', { at: this.cross })
  }

  draw(state) {
    if (state !== this.drawn) {
      this.drawn = state
      const books = state?.books ?? []
      this.books = books
      this.from = parseInstant(state?.window?.from ?? books[0]?.currunix ?? '0')
      this.to = parseInstant(state?.window?.to ?? books.at(-1)?.currunix ?? '0')
      const midpoint = []
      const executions = []
      for (const book of books) {
        if (book.bboMidpoint !== null && book.bboMidpoint !== undefined) midpoint.push([book.currunix, book.bboMidpoint])
        executions.push(...(book.executions ?? []))
      }
      this.series = { midpoint, executions }
      this.candles = aggregateCandles(executions, { origin: this.from, bucketNs: this.props.bucketNs })
      // The drawn span reaches the close of the last bucket, so its candle shows whole.
      const close = this.candles.at(-1)?.close
      this.end = close !== undefined && close > this.to ? close : this.to
      this.canvas.setAttribute('aria-label', this.sentence())
      this.readout.setAttribute('aria-valuemax', String((this.to - this.from) / this.props.bucketNs))
    }
    this.paint()
    this.describeCross()
  }

  sentence() {
    const { midpoint, executions } = this.series
    if (!this.books.length) return 'Prices: no books'
    const parts = [`${this.books.length} ${this.books.length === 1 ? 'book' : 'books'}`, `${executions.length} ${executions.length === 1 ? 'execution' : 'executions'}`]
    if (midpoint.length) parts.push(`last midpoint ${formatDecimal(midpoint.at(-1)[1])}`)
    return `Prices from ${formatInstant(this.from)} to ${formatInstant(this.to)}: ${parts.join(', ')}`
  }

  /** The served book at or before `at`, by a binary search over the books' own instants. */
  bookAt(at) {
    let lo = 0
    let hi = this.books.length - 1
    let found = null
    while (lo <= hi) {
      const mid = (lo + hi) >> 1
      if (compareInstant(this.books[mid].currunix, at) <= 0) {
        found = this.books[mid]
        lo = mid + 1
      } else hi = mid - 1
    }
    return found
  }

  describeCross() {
    if (this.cross === null) {
      this.readout.textContent = 'Point at the chart or use the arrow keys'
      this.readout.setAttribute('aria-valuetext', 'no instant')
      this.readout.setAttribute('aria-valuenow', '0')
      return
    }
    const book = this.bookAt(this.cross)
    const mid = book?.bboMidpoint
    const text = mid === null || mid === undefined ? formatInstant(this.cross) : `${formatInstant(this.cross)}, midpoint ${formatDecimal(mid)}`
    this.readout.textContent = text
    this.readout.setAttribute('aria-valuetext', text)
    this.readout.setAttribute('aria-valuenow', String((parseInstant(this.cross) - this.from) / this.props.bucketNs))
  }

  paint() {
    const { context, width, height } = fitCanvas(this.canvas)
    const colour = tokens(this.el, COLOURS)
    const plot = { left: MARGIN.left, right: width - MARGIN.right, top: MARGIN.top, bottom: height - MARGIN.bottom }
    this.perPixel = nanosPerPixel(this.from, this.end, Math.max(1, plot.right - plot.left))
    const xOf = (ns) => plot.left + toPixels(ns, this.from, this.perPixel)
    context.font = '11px ui-monospace, monospace'
    const { midpoint, executions } = this.series
    const prices = [...midpoint.map(([, price]) => price), ...this.candles.flatMap((candle) => [candle.high, candle.low])]
    if (!prices.length) {
      context.fillStyle = colour.muted
      context.fillText('No prices in this window', plot.left, (plot.top + plot.bottom) / 2)
      return
    }
    const [lo, hi] = padded(toFloat(minDecimal(...prices)), toFloat(maxDecimal(...prices)), 0.08)
    const y = linear(lo, hi, plot.bottom, plot.top)

    // Grid: round prices across, even instants along.
    context.lineWidth = 1
    context.strokeStyle = colour.line
    context.fillStyle = colour.muted
    context.textAlign = 'right'
    context.textBaseline = 'middle'
    for (const value of ticks(lo, hi, 5)) {
      const at = Math.round(y(value)) + 0.5
      context.beginPath()
      context.moveTo(plot.left, at)
      context.lineTo(plot.right, at)
      context.stroke()
      context.fillText(tickLabel(value), plot.left - 6, at)
    }
    context.textAlign = 'center'
    context.textBaseline = 'top'
    const span = this.end - this.from
    const stops = Math.max(1, Math.min(6, Math.floor((plot.right - plot.left) / 120)))
    for (let step = 0; step <= stops && span > 0n; step += 1) {
      const ns = this.from + (span * BigInt(step)) / BigInt(stops)
      const at = Math.round(xOf(ns)) + 0.5
      context.beginPath()
      context.moveTo(at, plot.top)
      context.lineTo(at, plot.bottom)
      context.stroke()
      context.textAlign = step === 0 ? 'left' : step === stops ? 'right' : 'center'
      context.fillText(formatClock(ns), at, plot.bottom + 5)
    }

    // Everything below stays inside the plot: a bucket may outlast the window.
    context.save()
    context.beginPath()
    context.rect(plot.left, plot.top, plot.right - plot.left, plot.bottom - plot.top)
    context.clip()

    // Candles: a body from the first to the last price, a wick from low to high.
    for (const candle of this.candles) {
      const left = xOf(candle.open)
      const right = xOf(candle.close)
      const width = Math.max(1, right - left - 2)
      const centre = left + (right - left) / 2
      const rising = compareDecimal(candle.last, candle.first) >= 0
      context.strokeStyle = rising ? colour.bid : colour.ask
      context.fillStyle = rising ? colour['bid-soft'] : colour['ask-soft']
      context.beginPath()
      context.moveTo(centre, y(toFloat(candle.high)))
      context.lineTo(centre, y(toFloat(candle.low)))
      context.stroke()
      const top = y(toFloat(rising ? candle.last : candle.first))
      const bottom = y(toFloat(rising ? candle.first : candle.last))
      context.fillRect(left + 1, top, width, Math.max(1, bottom - top))
      context.strokeRect(left + 1, top, width, Math.max(1, bottom - top))
    }

    this.stepLine(context, midpoint, xOf, y, xOf(this.end), colour.muted, [4, 3])

    context.fillStyle = colour.text
    for (const execution of executions) {
      if (execution.lastpx === null || execution.lastpx === undefined) continue
      context.beginPath()
      context.arc(xOf(execution.currunix), y(toFloat(execution.lastpx)), 3, 0, Math.PI * 2)
      context.fill()
    }

    if (this.cross !== null) {
      const at = Math.round(xOf(this.cross)) + 0.5
      context.strokeStyle = colour.focus
      context.setLineDash([])
      context.beginPath()
      context.moveTo(at, plot.top)
      context.lineTo(at, plot.bottom)
      context.stroke()
    }
    context.restore()
  }

  /** A value held from each served point until the next, out to `end`. */
  stepLine(context, points, xOf, y, end, stroke, dash) {
    if (!points.length) return
    context.save()
    context.strokeStyle = stroke
    context.lineWidth = 1.5
    context.setLineDash(dash)
    context.beginPath()
    let level = y(toFloat(points[0][1]))
    context.moveTo(xOf(points[0][0]), level)
    for (const [at, price] of points.slice(1)) {
      const x = xOf(at)
      context.lineTo(x, level)
      level = y(toFloat(price))
      context.lineTo(x, level)
    }
    context.lineTo(end, level)
    context.stroke()
    context.restore()
  }
}
