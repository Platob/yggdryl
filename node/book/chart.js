// The bid and ask candles of one ticker on a 2D canvas.
//
// The layout is pure: `scaleLinear`, `niceTicks`, `layoutCandles`,
// `nearestCandle` and `timeTicks` take numbers and the candle rows the service
// answers (`api/candles`: `start` and `end` as ISO-8601 text in the zone
// asked, every price as decimal text) and answer positions, so they run under
// Node without a document. Every column stands between its own bucket's edges
// on one time scale, since a bucket that follows a zone's wall clock is not
// always as long as the one before it.
// `drawCandles` is the one door that touches a canvas: it paints the scene
// at the device pixel ratio, keeps a snapshot of it, and draws the crosshair
// and the tooltip over the snapshot as the pointer or the keyboard moves.
//
// Bid candles stand left and ask candles right in every bucket, each in its
// own hue, so the two sides are told apart by place as well as by colour; the
// mid closes as a curve through the buckets and the spread as a band in the
// lower pane. Colours come from the `--bid`, `--ask`, `--mid`, `--spread`
// tokens of `theme.css` at draw time, so a theme change is one redraw.

import { formatDecimal, formatInstant, instantNanos, instantParts } from './audit.js'

/** The padding around the panes, in CSS pixels; the left side grows to fit the price labels. */
export const DEFAULT_PADDING = Object.freeze({ top: 14, right: 14, bottom: 26, left: 52 })

/** The share of the plot height the spread pane takes. */
export const SPREAD_SHARE = 0.24

/** The gap between the price pane and the spread pane. */
export const PANE_GAP = 14

/** The widest a candle body gets, in CSS pixels. */
export const MAX_BODY_WIDTH = 24

/** The most ticks a scale is asked for, whatever the count stated. */
const MAX_TICKS = 1000

/** The narrowest span, relative to its magnitude, a double can step through: anything narrower is flat. */
const FLAT = Number.EPSILON * 16

/** Whether `[min, max]` is too narrow for its ticks or its padding to be told apart from one value. */
function isFlat(min, max) {
  return max - min <= Math.max(Math.abs(min), Math.abs(max)) * FLAT
}

/** A linear scale `domain -> range` with `invert`; a flat domain maps to the range's middle. */
export function scaleLinear([d0, d1], [r0, r1]) {
  const span = d1 - d0
  const extent = r1 - r0
  const scale = (value) => (span === 0 ? (r0 + r1) / 2 : r0 + ((value - d0) / span) * extent)
  scale.invert = (position) => (extent === 0 ? d0 : d0 + ((position - r0) / extent) * span)
  scale.domain = [d0, d1]
  scale.range = [r0, r1]
  return scale
}

/** The 1, 2, 5 step that gives about `count` ticks over a span, `count` held to 1 to 1000. */
export function tickStep(min, max, count = 5) {
  const raw = Math.abs(max - min) / Math.min(MAX_TICKS, Math.max(1, count))
  if (raw === 0 || !Number.isFinite(raw)) return 1
  const power = Math.floor(Math.log10(raw))
  const magnitude = 10 ** power
  const residual = raw / magnitude
  const factor = residual >= Math.sqrt(50) ? 10 : residual >= Math.sqrt(10) ? 5 : residual >= Math.SQRT2 ? 2 : 1
  return factor * magnitude
}

/**
 * Round tick values inside `[min, max]`, about `count` of them; a flat range -
 * or one a few ulps wide, which no step can walk - is its one value, and a
 * range that is not finite, or whose span is not, has none. The ticks are
 * counted before they are made, so any finite input returns.
 */
export function niceTicks(min, max, count = 5) {
  if (!Number.isFinite(min) || !Number.isFinite(max)) return []
  if (min > max) [min, max] = [max, min]
  if (isFlat(min, max)) return [min]
  const step = tickStep(min, max, count)
  const first = Math.ceil(min / step)
  const last = Math.floor(max / step)
  if (!Number.isSafeInteger(first) || !Number.isSafeInteger(last)) return []
  const decimals = Math.max(0, -Math.floor(Math.log10(step)))
  const round = decimals <= 20 ? (value) => Number(value.toFixed(decimals)) : (value) => Number(value.toPrecision(15))
  const ticks = []
  for (let offset = 0; offset <= last - first; offset += 1) ticks.push(round((first + offset) * step))
  return ticks
}

/** A decimal's text as a number, `null` when it is absent or not a number. */
export function parseNumber(text) {
  if (text === null || text === undefined || text === '') return null
  const value = Number(text)
  return Number.isFinite(value) ? value : null
}

/**
 * An instant as epoch milliseconds, read by the display's one instant reader
 * (`instantParts`, so the service's `...+02:00[Europe/Zurich]` reads), `null`
 * when it does not read.
 */
export function parseInstant(value) {
  return instantParts(value)?.millis ?? null
}

function extend(bounds, value) {
  if (value === null) return
  if (value < bounds[0]) bounds[0] = value
  if (value > bounds[1]) bounds[1] = value
}

function ohlcBounds(bounds, ohlc) {
  if (!ohlc) return
  extend(bounds, parseNumber(ohlc.low))
  extend(bounds, parseNumber(ohlc.high))
  extend(bounds, parseNumber(ohlc.open))
  extend(bounds, parseNumber(ohlc.close))
}

/**
 * `[min, max]` widened by `ratio` of its span on each side, or by one percent
 * of the value a flat range stands at - a range a few ulps wide included.
 */
export function padDomain([min, max], ratio = 0.05) {
  if (isFlat(min, max)) {
    const middle = min + (max - min) / 2
    const unit = middle === 0 ? 1 : Math.abs(middle) * 0.01
    return [middle - unit, middle + unit]
  }
  const pad = (max - min) * ratio
  return [min - pad, max + pad]
}

/**
 * Where every candle stands: the price pane, the spread pane, one column per
 * bucket between its own `start` and `end` on the time scale, and the scales
 * and ticks of both panes. `width` and `height` are the canvas's CSS size.
 * `empty` says no row reads as a bucket; `priced` that some bucket states a
 * bid, an ask or a mid - buckets whose books stand with no level keep their
 * columns and the time axis without a price scale. `interval` is the shortest
 * bucket in milliseconds and `slot` its width, which size the candle bodies.
 */
export function layoutCandles(candles, { width, height, padding = DEFAULT_PADDING, spreadShare = SPREAD_SHARE, paneGap = PANE_GAP } = {}) {
  const rows = Array.isArray(candles) ? candles : []
  const innerWidth = Math.max(0, width - padding.left - padding.right)
  const innerHeight = Math.max(0, height - padding.top - padding.bottom)
  const bandHeight = Math.round(innerHeight * spreadShare)
  const plot = { x: padding.left, y: padding.top, width: innerWidth, height: Math.max(0, innerHeight - bandHeight - paneGap) }
  const band = { x: padding.left, y: plot.y + plot.height + paneGap, width: innerWidth, height: bandHeight }

  const priceBounds = [Infinity, -Infinity]
  const spreadBounds = [0, 0]
  let firstStart = Infinity
  let lastEnd = -Infinity
  let interval = Infinity
  const parsed = []
  rows.forEach((candle, index) => {
    const start = parseInstant(candle?.start)
    const end = parseInstant(candle?.end)
    if (start === null || end === null || end <= start) return
    parsed.push({ index, candle, start, end })
    if (start < firstStart) firstStart = start
    if (end > lastEnd) lastEnd = end
    if (end - start < interval) interval = end - start
    ohlcBounds(priceBounds, candle.bid)
    ohlcBounds(priceBounds, candle.ask)
    ohlcBounds(priceBounds, candle.mid)
    if (candle.spread) {
      extend(spreadBounds, parseNumber(candle.spread.low))
      extend(spreadBounds, parseNumber(candle.spread.high))
    }
  })

  const empty = parsed.length === 0
  const priced = !empty && Number.isFinite(priceBounds[0])
  const priceDomain = priced ? padDomain(priceBounds) : [0, 1]
  const spreadDomain = priced ? padDomain(spreadBounds, 0.1) : [0, 1]
  const price = scaleLinear(priceDomain, [plot.y + plot.height, plot.y])
  const spread = scaleLinear(spreadDomain, [band.y + band.height, band.y])
  const time = empty ? scaleLinear([0, 1], [plot.x, plot.x + plot.width]) : scaleLinear([firstStart, lastEnd], [plot.x, plot.x + plot.width])
  const slot = empty ? innerWidth : (innerWidth * interval) / (lastEnd - firstStart)
  const body = Math.max(1, Math.min(MAX_BODY_WIDTH, Math.floor(slot * 0.3)))
  const half = body / 2 + 1

  const columns = parsed.map(({ index, candle, start, end }) => {
    const x0 = time(start)
    const x1 = time(end)
    const center = (x0 + x1) / 2
    return { index, candle, key: instantNanos(candle.start), start, end, x0, x1, center, bidX: center - half, askX: center + half, body }
  })

  return {
    width,
    height,
    padding,
    plot,
    band,
    empty,
    priced,
    interval: empty ? 0 : interval,
    firstStart: empty ? null : firstStart,
    lastEnd: empty ? null : lastEnd,
    slot,
    columns,
    price,
    priceTicks: priced ? niceTicks(priceDomain[0], priceDomain[1], Math.max(2, Math.floor(plot.height / 44))) : [],
    spread,
    spreadTicks: priced ? niceTicks(spreadDomain[0], spreadDomain[1], 2) : [],
    time,
  }
}

/**
 * The column nearest an x position inside the plot, as `{ index, candle,
 * column }`, or `null` outside the plot or when there is no column.
 */
export function nearestCandle(candles, x, layout) {
  if (!layout || layout.columns.length === 0) return null
  const { plot } = layout
  if (!Number.isFinite(x) || x < plot.x - layout.slot / 2 || x > plot.x + plot.width + layout.slot / 2) return null
  let best = null
  let distance = Infinity
  for (const column of layout.columns) {
    const gap = Math.abs(column.center - x)
    if (gap < distance) {
      distance = gap
      best = column
    }
  }
  return best === null ? null : { index: best.index, candle: best.candle, column: best }
}

const formatters = new Map()

function formatter(zone, options) {
  const key = `${zone}|${JSON.stringify(options)}`
  let cached = formatters.get(key)
  if (cached === undefined) {
    try {
      cached = new Intl.DateTimeFormat('en-GB', { timeZone: zone, hourCycle: 'h23', ...options })
    } catch {
      cached = new Intl.DateTimeFormat('en-GB', { timeZone: 'UTC', hourCycle: 'h23', ...options })
    }
    formatters.set(key, cached)
  }
  return cached
}

/** The shortest bucket the time axis reads as a day: a day across a saving-time change is 23 or 25 hours, an hour at most two. */
const DAY_LIKE = 20 * 3_600_000

/** The label of a time tick: the day for a daily interval, else the time, dated where the day changes. */
export function formatTick(ms, zone, interval, withDate = false) {
  const date = new Date(ms)
  if (interval >= DAY_LIKE) return formatter(zone, { day: '2-digit', month: 'short' }).format(date)
  const time = formatter(zone, { hour: '2-digit', minute: '2-digit' }).format(date)
  return withDate ? `${formatter(zone, { day: '2-digit', month: 'short' }).format(date)} ${time}` : time
}

/** The width a tick label takes where no canvas measures it: about an 11px system font. */
const APPROXIMATE_MEASURE = (text) => text.length * 6.5

/**
 * The x ticks of a layout in the zone, each `{ x, ms, label, left }`: the
 * bucket edges - every column's start and the last end - from the left,
 * each kept only where its label, drawn from `left` and measured by
 * `measure`, clears the one before by `gap` pixels and fits the canvas, so
 * the labels never overlap however narrow the chart. A label is dated where
 * the day changes from the label before it.
 */
export function timeTicks(layout, zone = 'UTC', { measure = APPROXIMATE_MEASURE, gap = 12 } = {}) {
  if (layout.empty) return []
  const edges = [...new Set([...layout.columns.map((column) => column.start), layout.lastEnd])].sort((a, b) => a - b)
  const day = formatter(zone, { day: '2-digit', month: 'short', year: 'numeric' })
  const ticks = []
  let right = -Infinity
  let previousDay = null
  for (const ms of edges) {
    const currentDay = day.format(new Date(ms))
    const label = formatTick(ms, zone, layout.interval, previousDay !== currentDay)
    const width = measure(label)
    const x = layout.time(ms)
    const left = Math.min(x + 3, layout.width - width - 2)
    if (left < right + gap || left < 0) continue
    ticks.push({ x, ms, label, left })
    right = left + width
    previousDay = currentDay
  }
  return ticks
}

/**
 * A bucket read out as one sentence, what the chart announces as the
 * keyboard reaches it: its edges in the zone, each side's open, high, low and
 * close, the mid and spread closes, the touch quantities and how many books
 * it folded - whatever the bucket states.
 */
export function describeCandle(candle, zone = 'UTC') {
  const start = formatInstant(candle.start, zone, { fraction: 0 })
  const end = formatInstant(candle.end, zone, { fraction: 0 })
  const sameDay = start.slice(0, 10) === end.slice(0, 10)
  const parts = []
  for (const side of ['bid', 'ask']) {
    const ohlc = candle[side]
    if (ohlc) parts.push(`${side} open ${formatDecimal(ohlc.open)}, high ${formatDecimal(ohlc.high)}, low ${formatDecimal(ohlc.low)}, close ${formatDecimal(ohlc.close)}`)
  }
  if (candle.mid) parts.push(`mid close ${formatDecimal(candle.mid.close)}`)
  if (candle.spread) parts.push(`spread close ${formatDecimal(candle.spread.close)}`)
  if (candle.bidqty !== null && candle.bidqty !== undefined) parts.push(`bid quantity ${formatDecimal(candle.bidqty)}`)
  if (candle.askqty !== null && candle.askqty !== undefined) parts.push(`ask quantity ${formatDecimal(candle.askqty)}`)
  const books = candle.books ?? 0
  parts.push(`${books} book${books === 1 ? '' : 's'}`)
  return `${start} to ${sameDay ? end.slice(11) : end}: ${parts.join('; ')}`
}

const FALLBACK_COLORS = Object.freeze({
  bid: '#13946a',
  ask: '#e34948',
  mid: '#2a78d6',
  spread: '#eb6834',
  accent: '#4a3aa7',
  fg: '#16181d',
  muted: '#6b7280',
  border: '#d9dbe0',
  grid: '#eceef2',
  panel: '#ffffff',
})

/** The chart's colours: the canvas's CSS tokens, `overrides` winning, the fallbacks last. */
export function readColors(canvas, overrides = null) {
  const colors = { ...FALLBACK_COLORS }
  const view = canvas?.ownerDocument?.defaultView
  if (view?.getComputedStyle) {
    const style = view.getComputedStyle(canvas)
    for (const name of Object.keys(colors)) {
      const value = style.getPropertyValue(`--${name}`).trim()
      if (value) colors[name] = value
    }
  }
  return overrides ? { ...colors, ...overrides } : colors
}

const controllers = new WeakMap()

function ohlcOf(candle, side) {
  const ohlc = candle?.[side]
  if (!ohlc) return null
  const open = parseNumber(ohlc.open)
  const high = parseNumber(ohlc.high)
  const low = parseNumber(ohlc.low)
  const close = parseNumber(ohlc.close)
  if (open === null || high === null || low === null || close === null) return null
  return { open, high, low, close }
}

function paintCandle(context, x, ohlc, scale, body, color, panel) {
  const top = scale(Math.max(ohlc.open, ohlc.close))
  const bottom = scale(Math.min(ohlc.open, ohlc.close))
  context.strokeStyle = color
  context.fillStyle = color
  context.lineWidth = 1
  context.beginPath()
  context.moveTo(x, scale(ohlc.high))
  context.lineTo(x, scale(ohlc.low))
  context.stroke()
  const height = Math.max(1, bottom - top)
  const left = x - body / 2
  if (ohlc.close < ohlc.open) {
    context.fillRect(left, top, body, height)
  } else if (body >= 3 && height >= 3) {
    context.fillStyle = panel
    context.fillRect(left, top, body, height)
    context.lineWidth = 1.5
    context.strokeRect(left + 0.75, top + 0.75, body - 1.5, height - 1.5)
  } else {
    context.fillRect(left, top, body, height)
  }
}

function paintCurve(context, points, color, width) {
  context.strokeStyle = color
  context.lineWidth = width
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.beginPath()
  let open = false
  for (const point of points) {
    if (point === null) {
      open = false
      continue
    }
    if (open) context.lineTo(point.x, point.y)
    else context.moveTo(point.x, point.y)
    open = true
  }
  context.stroke()
}

function withAlpha(context, alpha, paint) {
  const previous = context.globalAlpha
  context.globalAlpha = alpha
  paint()
  context.globalAlpha = previous
}

function paintScene(context, layout, colors, { zone, selected, emptyText, unpricedText, font }) {
  const { plot, band } = layout
  context.fillStyle = colors.panel
  context.fillRect(0, 0, layout.width, layout.height)
  context.font = font
  context.textBaseline = 'middle'

  if (layout.empty) {
    context.fillStyle = colors.muted
    context.textAlign = 'center'
    context.fillText(emptyText, layout.width / 2, layout.height / 2)
    return
  }

  // The selection wash first, under everything it marks.
  const chosen = selected === null ? undefined : layout.columns.find((column) => column.key === selected)
  if (chosen) {
    withAlpha(context, 0.12, () => {
      context.fillStyle = colors.accent
      context.fillRect(chosen.x0, plot.y, chosen.x1 - chosen.x0, band.y + band.height - plot.y)
    })
    context.fillStyle = colors.accent
    context.fillRect(chosen.x0, plot.y - 3, chosen.x1 - chosen.x0, 2)
  }

  // Hairline grid and the price labels on the left.
  context.strokeStyle = colors.grid
  context.lineWidth = 1
  context.fillStyle = colors.muted
  context.textAlign = 'right'
  for (const tick of layout.priceTicks) {
    const y = Math.round(layout.price(tick)) + 0.5
    context.beginPath()
    context.moveTo(plot.x, y)
    context.lineTo(plot.x + plot.width, y)
    context.stroke()
    context.fillText(formatDecimal(String(tick)), plot.x - 8, y)
  }
  for (const tick of layout.spreadTicks) {
    const y = Math.round(layout.spread(tick)) + 0.5
    context.beginPath()
    context.moveTo(band.x, y)
    context.lineTo(band.x + band.width, y)
    context.stroke()
    context.fillText(formatDecimal(String(tick)), band.x - 8, y)
  }
  context.strokeStyle = colors.border
  context.beginPath()
  context.moveTo(plot.x, Math.round(plot.y + plot.height) + 0.5)
  context.lineTo(plot.x + plot.width, Math.round(plot.y + plot.height) + 0.5)
  context.moveTo(band.x, Math.round(band.y + band.height) + 0.5)
  context.lineTo(band.x + band.width, Math.round(band.y + band.height) + 0.5)
  context.stroke()

  // The time axis under the spread pane, its labels measured so none overlaps the one before.
  context.textAlign = 'left'
  context.fillStyle = colors.muted
  for (const tick of timeTicks(layout, zone, { measure: (text) => context.measureText(text).width })) {
    context.strokeStyle = colors.grid
    context.beginPath()
    context.moveTo(Math.round(tick.x) + 0.5, band.y + band.height)
    context.lineTo(Math.round(tick.x) + 0.5, band.y + band.height + 4)
    context.stroke()
    context.fillText(tick.label, tick.left, band.y + band.height + 13)
  }

  // Buckets whose books stood with no level: their columns and the axis stand, and no price does.
  if (!layout.priced) {
    context.textAlign = 'center'
    context.fillText(unpricedText, plot.x + plot.width / 2, plot.y + plot.height / 2)
    return
  }

  // The spread band: the low-high area as a wash and the close as a line.
  const highs = []
  const lows = []
  const closes = []
  for (const column of layout.columns) {
    const spread = column.candle.spread
    const low = parseNumber(spread?.low)
    const high = parseNumber(spread?.high)
    const close = parseNumber(spread?.close)
    if (low === null || high === null || close === null) {
      highs.push(null)
      lows.push(null)
      closes.push(null)
      continue
    }
    highs.push({ x: column.center, y: layout.spread(high) })
    lows.push({ x: column.center, y: layout.spread(low) })
    closes.push({ x: column.center, y: layout.spread(close) })
  }
  withAlpha(context, 0.14, () => {
    context.fillStyle = colors.spread
    let run = []
    const flush = () => {
      if (run.length > 0) {
        context.beginPath()
        run.forEach(([high], index) => (index === 0 ? context.moveTo(high.x, high.y) : context.lineTo(high.x, high.y)))
        for (let index = run.length - 1; index >= 0; index -= 1) context.lineTo(run[index][1].x, run[index][1].y)
        context.closePath()
        context.fill()
      }
      run = []
    }
    highs.forEach((high, index) => {
      if (high === null) flush()
      else run.push([high, lows[index]])
    })
    flush()
  })
  paintCurve(context, closes, colors.spread, 1.5)

  // The candles, bid left and ask right of each bucket's centre.
  for (const column of layout.columns) {
    const bid = ohlcOf(column.candle, 'bid')
    const ask = ohlcOf(column.candle, 'ask')
    if (bid) paintCandle(context, column.bidX, bid, layout.price, column.body, colors.bid, colors.panel)
    if (ask) paintCandle(context, column.askX, ask, layout.price, column.body, colors.ask, colors.panel)
  }

  // The mid curve over the candles.
  const mids = layout.columns.map((column) => {
    const close = parseNumber(column.candle.mid?.close)
    return close === null ? null : { x: column.center, y: layout.price(close) }
  })
  paintCurve(context, mids, colors.mid, 2)
}

function paintCrosshair(context, layout, colors, column, y, font) {
  const { plot, band } = layout
  const x = Math.round(column.center) + 0.5
  context.save()
  context.strokeStyle = colors.muted
  context.lineWidth = 1
  context.setLineDash([3, 3])
  context.beginPath()
  context.moveTo(x, plot.y)
  context.lineTo(x, band.y + band.height)
  if (y !== null && y >= plot.y && y <= plot.y + plot.height) {
    const line = Math.round(y) + 0.5
    context.moveTo(plot.x, line)
    context.lineTo(plot.x + plot.width, line)
  }
  context.stroke()
  context.setLineDash([])
  if (y !== null && y >= plot.y && y <= plot.y + plot.height) {
    const label = formatDecimal(layout.price.invert(y).toFixed(4))
    context.font = font
    context.textBaseline = 'middle'
    context.textAlign = 'right'
    const width = context.measureText(label).width + 8
    context.fillStyle = colors.fg
    context.fillRect(plot.x - width - 4, y - 8, width, 16)
    context.fillStyle = colors.panel
    context.fillText(label, plot.x - 8, y)
  }
  context.restore()
}

function tooltipRows(candle, zone) {
  const rows = []
  const ohlc = (name, side) => {
    const values = candle[side]
    if (!values) return
    rows.push([name, `${formatDecimal(values.open)} · ${formatDecimal(values.high)} · ${formatDecimal(values.low)} · ${formatDecimal(values.close)}`, side])
  }
  ohlc('Bid O·H·L·C', 'bid')
  ohlc('Ask O·H·L·C', 'ask')
  if (candle.mid) rows.push(['Mid close', formatDecimal(candle.mid.close), 'mid'])
  if (candle.spread) rows.push(['Spread close', formatDecimal(candle.spread.close), 'spread'])
  if (candle.bidqty !== null && candle.bidqty !== undefined) rows.push(['Bid qty', formatDecimal(candle.bidqty), 'bid'])
  if (candle.askqty !== null && candle.askqty !== undefined) rows.push(['Ask qty', formatDecimal(candle.askqty), 'ask'])
  rows.push(['Books', String(candle.books ?? 0), null])
  return { head: `${formatInstant(candle.start, zone)} → ${formatInstant(candle.end, zone)}`, rows }
}

function fillTooltip(tooltip, candle, zone) {
  const document = tooltip.ownerDocument
  tooltip.replaceChildren()
  const { head, rows } = tooltipRows(candle, zone)
  const title = document.createElement('div')
  title.className = 'tooltip-head'
  title.textContent = head
  tooltip.append(title)
  const table = document.createElement('dl')
  table.className = 'tooltip-rows'
  for (const [label, value, key] of rows) {
    const term = document.createElement('dt')
    if (key) {
      const swatch = document.createElement('span')
      swatch.className = `key key-${key}`
      swatch.setAttribute('aria-hidden', 'true')
      term.append(swatch)
    }
    term.append(document.createTextNode(label))
    const detail = document.createElement('dd')
    detail.textContent = value
    table.append(term, detail)
  }
  tooltip.append(table)
}

function placeTooltip(tooltip, layout, x, y) {
  tooltip.hidden = false
  const width = tooltip.offsetWidth || 200
  const height = tooltip.offsetHeight || 120
  let left = x + 14
  if (left + width > layout.width - 4) left = x - width - 14
  if (left < 4) left = 4
  let top = y - height / 2
  if (top < 4) top = 4
  if (top + height > layout.height - 4) top = Math.max(4, layout.height - height - 4)
  tooltip.style.left = `${Math.round(left)}px`
  tooltip.style.top = `${Math.round(top)}px`
}

/**
 * Paint `candles` on `canvas` and wire its hover, click and keyboard
 * handling. `options`: `zone` (IANA, the axis and the tooltip's), `selected`
 * (the `start` of the chosen bucket, any spelling of that instant),
 * `onSelect(candle, index)`, `onHover(candle | null)`, `tooltip` (an element
 * positioned inside the canvas's box), `readout` (a live region the bucket
 * the keyboard reaches is read into, `describeCandle`), `colors` (overrides
 * of the CSS tokens), `emptyText` (no bucket), `unpricedText` (buckets with
 * no bid, ask or mid). Answers `{ layout, redraw(), destroy(), hover(index),
 * select(index) }`, or `null` where the canvas has no 2D context. A second
 * call on the same canvas replaces the first.
 */
export function drawCandles(canvas, candles, options = {}) {
  const context = canvas?.getContext?.('2d')
  if (!context) return null
  controllers.get(canvas)?.destroy()

  const rows = Array.isArray(candles) ? candles : []
  const {
    zone = 'UTC',
    onSelect,
    onHover,
    tooltip = null,
    readout = null,
    emptyText = 'No candles in this range',
    unpricedText = 'No bid or ask in this range',
  } = options
  const selected = instantNanos(options.selected ?? null)
  const view = canvas.ownerDocument?.defaultView ?? globalThis
  const ratio = Math.max(1, view.devicePixelRatio || 1)
  const rect = canvas.getBoundingClientRect?.() ?? { width: 0, height: 0 }
  const width = Math.max(1, Math.round(rect.width || canvas.clientWidth || canvas.width))
  const height = Math.max(1, Math.round(rect.height || canvas.clientHeight || canvas.height))
  canvas.width = Math.round(width * ratio)
  canvas.height = Math.round(height * ratio)
  context.setTransform(ratio, 0, 0, ratio, 0, 0)

  const colors = readColors(canvas, options.colors)
  const font = options.font ?? '11px system-ui, -apple-system, "Segoe UI", sans-serif'
  context.font = font
  const draft = layoutCandles(rows, { width, height })
  const widest = Math.max(0, ...[...draft.priceTicks, ...draft.spreadTicks].map((tick) => context.measureText(formatDecimal(String(tick))).width))
  const padding = { ...DEFAULT_PADDING, left: Math.max(DEFAULT_PADDING.left, Math.ceil(widest) + 16) }
  const layout = layoutCandles(rows, { width, height, padding })

  const scene = { zone, selected, emptyText, unpricedText, font }
  paintScene(context, layout, colors, scene)

  let snapshot = null
  try {
    snapshot = canvas.ownerDocument.createElement('canvas')
    snapshot.width = canvas.width
    snapshot.height = canvas.height
    snapshot.getContext('2d').drawImage(canvas, 0, 0)
  } catch {
    snapshot = null
  }

  const restore = () => {
    if (snapshot) {
      context.save()
      context.setTransform(1, 0, 0, 1, 0, 0)
      context.drawImage(snapshot, 0, 0)
      context.restore()
    } else {
      paintScene(context, layout, colors, scene)
    }
  }

  let hovered = null
  const showAt = (found, x, y) => {
    restore()
    if (found === null) {
      hovered = null
      if (tooltip) tooltip.hidden = true
      onHover?.(null)
      return
    }
    hovered = found
    paintCrosshair(context, layout, colors, found.column, y, font)
    if (tooltip) {
      fillTooltip(tooltip, found.candle, zone)
      placeTooltip(tooltip, layout, x ?? found.column.center, y ?? layout.plot.y + layout.plot.height / 2)
    }
    onHover?.(found.candle, found.index)
  }

  const point = (event) => {
    const box = canvas.getBoundingClientRect()
    return { x: event.clientX - box.left, y: event.clientY - box.top }
  }
  const onPointerMove = (event) => {
    const { x, y } = point(event)
    showAt(nearestCandle(rows, x, layout), x, y)
  }
  const onPointerLeave = () => showAt(null)
  const onClick = (event) => {
    const { x } = point(event)
    const found = nearestCandle(rows, x, layout)
    if (found) onSelect?.(found.candle, found.index)
  }
  const columnAt = (position) => layout.columns.find((column) => column.index === position) ?? null
  // What the keyboard reaches is read out, since the canvas and the tooltip are only pixels to a screen reader.
  const hover = (index) => {
    const column = columnAt(index)
    if (column === null) {
      showAt(null)
      return
    }
    showAt({ index: column.index, candle: column.candle, column }, null, null)
    if (readout) readout.textContent = describeCandle(column.candle, zone)
  }
  const onKeyDown = (event) => {
    if (layout.columns.length === 0) return
    const order = layout.columns.map((column) => column.index)
    const current = hovered ? order.indexOf(hovered.index) : layout.columns.findIndex((column) => selected !== null && column.key === selected)
    let next = null
    switch (event.key) {
      case 'ArrowLeft':
        next = current <= 0 ? order.length - 1 : current - 1
        break
      case 'ArrowRight':
        next = current < 0 || current >= order.length - 1 ? 0 : current + 1
        break
      case 'Home':
        next = 0
        break
      case 'End':
        next = order.length - 1
        break
      case 'Enter':
      case ' ':
        if (hovered) onSelect?.(hovered.candle, hovered.index)
        event.preventDefault()
        return
      case 'Escape':
        showAt(null)
        return
      default:
        return
    }
    event.preventDefault()
    hover(order[next])
  }

  canvas.addEventListener('pointermove', onPointerMove)
  canvas.addEventListener('pointerleave', onPointerLeave)
  canvas.addEventListener('click', onClick)
  canvas.addEventListener('keydown', onKeyDown)
  canvas.addEventListener('blur', onPointerLeave)

  const controller = {
    layout,
    colors,
    redraw: () => drawCandles(canvas, candles, options),
    hover,
    select: (index) => {
      const column = columnAt(index)
      if (column) onSelect?.(column.candle, column.index)
    },
    destroy: () => {
      canvas.removeEventListener('pointermove', onPointerMove)
      canvas.removeEventListener('pointerleave', onPointerLeave)
      canvas.removeEventListener('click', onClick)
      canvas.removeEventListener('keydown', onKeyDown)
      canvas.removeEventListener('blur', onPointerLeave)
      if (tooltip) tooltip.hidden = true
      controllers.delete(canvas)
    },
  }
  controllers.set(canvas, controller)
  return controller
}
