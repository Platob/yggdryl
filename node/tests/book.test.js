'use strict'

// The book display and its spawner: `node/book.js` and the ES modules under
// `node/book/`. The modules are imported under Node, so nothing they do at
// import time may need a document, and their pure helpers are pinned here
// without one.

const assert = require('node:assert/strict')
const test = require('node:test')
const { chmodSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { pathToFileURL } = require('node:url')

const book = require('../book.js')

const source = (name) => readFileSync(join(book.assets, name), 'utf8')
const load = (name) => import(pathToFileURL(join(book.assets, name)).href)

const CANDLES = [
  {
    start: '2026-08-14T14:46:00+02:00',
    end: '2026-08-14T14:47:00+02:00',
    bid: { open: '72.20', high: '72.30', low: '72.10', close: '72.25' },
    ask: { open: '72.30', high: '72.40', low: '72.25', close: '72.35' },
    mid: { open: '72.25', high: '72.35', low: '72.175', close: '72.30' },
    spread: { open: '0.10', high: '0.15', low: '0.05', close: '0.10' },
    bidqty: '300',
    askqty: '250',
    books: 4,
    executions: 1,
    volume: '300',
  },
  {
    start: '2026-08-14T14:47:00+02:00',
    end: '2026-08-14T14:48:00+02:00',
    bid: { open: '72.25', high: '72.45', low: '72.20', close: '72.40' },
    ask: null,
    mid: null,
    spread: null,
    bidqty: '100',
    askqty: null,
    books: 2,
    executions: 0,
    volume: '0',
  },
  {
    start: '2026-08-14T14:49:00+02:00',
    end: '2026-08-14T14:50:00+02:00',
    bid: { open: '72.40', high: '72.40', low: '72.00', close: '72.05' },
    ask: { open: '72.50', high: '72.60', low: '72.10', close: '72.15' },
    mid: { open: '72.45', high: '72.5', low: '72.05', close: '72.10' },
    spread: { open: '0.10', high: '0.20', low: '0.10', close: '0.10' },
    bidqty: '50',
    askqty: '75',
    books: 3,
    executions: 2,
    volume: '125',
  },
]

test('every asset the CLI embeds exists, is non-empty, and nothing else is shipped beside them', () => {
  assert.deepEqual(
    [...book.assetFiles],
    ['index.html', 'theme.css', 'theme.js', 'api.js', 'chart.js', 'audit.js', 'app.js', 'favicon.svg'],
  )
  for (const name of book.assetFiles) {
    assert.ok(statSync(join(book.assets, name)).size > 0, `${name} is empty`)
  }
  // The folder's own package.json only tells Node the .js files are modules.
  assert.deepEqual(readdirSync(book.assets).sort(), [...book.assetFiles, 'package.json'].sort())
  assert.deepEqual(JSON.parse(source('package.json')), { type: 'module' })
})

test('every module imports under Node and exports its doors', async () => {
  const theme = await load('theme.js')
  const api = await load('api.js')
  const chart = await load('chart.js')
  const audit = await load('audit.js')
  const app = await load('app.js')
  for (const name of ['resolveTheme', 'applyTheme', 'toggleTheme', 'readTheme', 'nextTheme']) assert.equal(typeof theme[name], 'function', name)
  for (const name of ['fetchTables', 'fetchTickers', 'fetchCandles', 'fetchBook', 'fetchEvents', 'auditUrl', 'queryString']) {
    assert.equal(typeof api[name], 'function', name)
  }
  for (const name of ['drawCandles', 'nearestCandle', 'scaleLinear', 'niceTicks', 'layoutCandles']) assert.equal(typeof chart[name], 'function', name)
  for (const name of ['renderSummary', 'renderEvents', 'downloadLink', 'formatInstant', 'formatDecimal', 'sortRows']) {
    assert.equal(typeof audit[name], 'function', name)
  }
  for (const name of ['start', 'readHash', 'writeHash', 'timezoneChoices', 'spanToRange', 'apiBase']) assert.equal(typeof app[name], 'function', name)
})

test('index.html references the stylesheet, the icon and the app, which imports every other module', () => {
  const html = source('index.html')
  assert.match(html, /<link rel="stylesheet" href="theme\.css"/)
  assert.match(html, /<link rel="icon" href="favicon\.svg"/)
  assert.match(html, /<script type="module" src="app\.js"><\/script>/)
  assert.match(html, /<meta name="viewport" content="width=device-width, initial-scale=1"/)
  assert.match(html, /<meta name="color-scheme" content="light dark"/)
  const app = source('app.js')
  for (const module of ['theme.js', 'api.js', 'chart.js', 'audit.js']) {
    assert.match(app, new RegExp(`from '\\./${module.replace('.', '\\.')}'`), `app.js imports ${module}`)
  }
  // Every element the app reaches by id is in the shell.
  const ids = [...app.matchAll(/byId\('([a-z-]+)'\)/g)].map((match) => match[1])
  assert.ok(ids.length >= 14)
  for (const id of ids) assert.match(html, new RegExp(`id="${id}"`), `index.html has #${id}`)
  // The chart is reachable by keyboard and the selectors are labelled.
  assert.match(html, /<canvas[^>]*tabindex="0"/)
  assert.match(html, /aria-label="Selection"/)
})

test('theme.css declares the tokens in the light scheme and in both dark scopes', () => {
  const css = source('theme.css')
  const tokens = ['--bg', '--fg', '--muted', '--panel', '--border', '--bid', '--ask', '--mid', '--accent']
  const block = (opening) => {
    const at = css.indexOf(opening)
    assert.notEqual(at, -1, opening)
    const open = css.indexOf('{', at)
    const close = css.indexOf('}', open)
    return css.slice(open, close)
  }
  const light = block(':root {')
  const system = block(':root:not([data-theme="light"])')
  const stamped = block(':root[data-theme="dark"]')
  assert.match(css.slice(0, css.indexOf(':root:not([data-theme="light"])')), /@media \(prefers-color-scheme: dark\)/)
  for (const token of tokens) {
    for (const [name, body] of [['light', light], ['system dark', system], ['stamped dark', stamped]]) {
      assert.match(body, new RegExp(`${token}:\\s*[^;]+;`), `${token} in the ${name} block`)
    }
  }
  // The two dark scopes state the same values.
  const values = (body) => Object.fromEntries([...body.matchAll(/(--[a-z-]+):\s*([^;]+);/g)].map((match) => [match[1], match[2].trim()]))
  assert.deepEqual(values(system), values(stamped))
  assert.match(css, /@media \(prefers-reduced-motion: reduce\)/)
  assert.match(css, /:focus-visible\s*\{/)
})

test('scaleLinear maps and inverts, and a flat domain lands in the middle of the range', async () => {
  const { scaleLinear } = await load('chart.js')
  const scale = scaleLinear([0, 10], [100, 0])
  assert.equal(scale(0), 100)
  assert.equal(scale(10), 0)
  assert.equal(scale(2.5), 75)
  assert.equal(scale.invert(75), 2.5)
  assert.deepEqual(scale.domain, [0, 10])
  assert.deepEqual(scale.range, [100, 0])
  const flat = scaleLinear([5, 5], [0, 40])
  assert.equal(flat(5), 20)
  assert.equal(flat(9), 20)
  assert.equal(flat.invert(20), 5)
})

test('niceTicks lands on round values inside the range', async () => {
  const { niceTicks, tickStep } = await load('chart.js')
  assert.deepEqual(niceTicks(0, 10, 5), [0, 2, 4, 6, 8, 10])
  assert.deepEqual(niceTicks(72.1, 72.9, 4), [72.2, 72.4, 72.6, 72.8])
  assert.deepEqual(niceTicks(-1.5, 1.5, 3), [-1, 0, 1])
  assert.deepEqual(niceTicks(0, 1234, 4), [0, 200, 400, 600, 800, 1000, 1200])
  assert.deepEqual(niceTicks(3, 3), [3])
  assert.deepEqual(niceTicks(10, 0, 5), [0, 2, 4, 6, 8, 10])
  assert.deepEqual(niceTicks(Number.NaN, 1), [])
  assert.equal(tickStep(0, 100, 5), 20)
  assert.equal(tickStep(0, 1, 5), 0.2)
  assert.equal(tickStep(0, 0), 1)
})

test('layoutCandles places every bucket at its instant, bid left and ask right, over both panes', async () => {
  const { layoutCandles, DEFAULT_PADDING } = await load('chart.js')
  const layout = layoutCandles(CANDLES, { width: 452, height: 300 })
  assert.equal(layout.empty, false)
  assert.equal(layout.interval, 60_000)
  // Three candles over four minutes: the third stands in the fourth slot.
  assert.equal(layout.slots, 4)
  assert.deepEqual(layout.columns.map((column) => column.position), [0, 1, 3])
  assert.deepEqual(layout.columns.map((column) => column.index), [0, 1, 2])
  const inner = 452 - DEFAULT_PADDING.left - DEFAULT_PADDING.right
  assert.equal(layout.slot, inner / 4)
  const [first] = layout.columns
  assert.equal(first.x0, DEFAULT_PADDING.left)
  assert.equal(first.center, DEFAULT_PADDING.left + layout.slot / 2)
  assert.ok(first.bidX < first.center && first.center < first.askX)
  assert.equal(first.askX - first.bidX, first.body + 2)
  assert.ok(first.body >= 1 && first.body <= 24)
  // The price scale covers every low and high with a margin, top of the plot first.
  assert.ok(layout.price.domain[0] < 72 && layout.price.domain[1] > 72.6)
  assert.equal(layout.price(layout.price.domain[1]), layout.plot.y)
  assert.equal(layout.price(layout.price.domain[0]), layout.plot.y + layout.plot.height)
  assert.ok(layout.priceTicks.length >= 2)
  assert.ok(layout.priceTicks.every((tick) => tick >= layout.price.domain[0] && tick <= layout.price.domain[1]))
  // The spread pane sits under the plot and starts at zero.
  assert.ok(layout.band.y > layout.plot.y + layout.plot.height)
  assert.equal(layout.band.y + layout.band.height, 300 - DEFAULT_PADDING.bottom)
  assert.ok(layout.spread.domain[0] <= 0 && layout.spread.domain[1] > 0.2)
  assert.equal(layout.time(layout.firstStart), layout.plot.x)
  assert.equal(layout.time(layout.lastEnd), layout.plot.x + layout.plot.width)
})

test('layoutCandles over nothing, or over rows it cannot read, is the empty layout', async () => {
  const { layoutCandles } = await load('chart.js')
  for (const rows of [[], null, undefined, [{ start: 'nope', end: 'nope' }], [{ start: '2026-01-01T00:01:00Z', end: '2026-01-01T00:00:00Z' }]]) {
    const layout = layoutCandles(rows, { width: 400, height: 200 })
    assert.equal(layout.empty, true)
    assert.deepEqual(layout.columns, [])
    assert.deepEqual(layout.priceTicks, [])
    assert.equal(layout.slots, 1)
  }
})

test('nearestCandle answers the column under a position inside the plot and nothing outside it', async () => {
  const { layoutCandles, nearestCandle } = await load('chart.js')
  const layout = layoutCandles(CANDLES, { width: 452, height: 300 })
  const [first, second, third] = layout.columns
  assert.equal(nearestCandle(CANDLES, first.center, layout).index, 0)
  assert.equal(nearestCandle(CANDLES, second.x0 + 1, layout).index, 1)
  // The empty third slot is nearer the last candle than the second.
  assert.equal(nearestCandle(CANDLES, third.x0 - layout.slot * 0.4, layout).index, 2)
  assert.equal(nearestCandle(CANDLES, third.x0 - layout.slot * 0.6, layout).index, 1)
  assert.equal(nearestCandle(CANDLES, -50, layout), null)
  assert.equal(nearestCandle(CANDLES, layout.width + 50, layout), null)
  assert.equal(nearestCandle(CANDLES, Number.NaN, layout), null)
  assert.equal(nearestCandle([], 100, layoutCandles([], { width: 452, height: 300 })), null)
  assert.equal(nearestCandle(CANDLES, 100, null), null)
})

test('the time axis is labelled in the zone asked, dated where the day changes', async () => {
  const { layoutCandles, timeTicks, formatTick } = await load('chart.js')
  const layout = layoutCandles(CANDLES, { width: 452, height: 300 })
  const zurich = timeTicks(layout, 'Europe/Zurich', 4)
  assert.deepEqual(zurich.map((tick) => tick.label), ['14 Aug 14:46', '14:47', '14:48', '14:49', '14:50'])
  assert.equal(zurich[0].x, layout.plot.x)
  const tokyo = timeTicks(layout, 'Asia/Tokyo', 4)
  assert.equal(tokyo[0].label, '14 Aug 21:46')
  assert.equal(formatTick(Date.parse('2026-08-14T00:00:00Z'), 'UTC', 86_400_000), '14 Aug')
  assert.equal(formatTick(Date.parse('2026-08-14T00:00:00Z'), 'America/New_York', 3_600_000, true), '13 Aug 20:00')
  assert.equal(formatTick(Date.parse('2026-08-14T00:00:00Z'), 'Not/AZone', 3_600_000), '00:00')
  assert.deepEqual(timeTicks(layoutCandles([], { width: 400, height: 200 })), [])
})

/** A recording 2D context and the canvas, document and elements around it: what `drawCandles` touches. */
function fakeDom({ width = 600, height = 320 } = {}) {
  const calls = []
  const context = new Proxy(
    {
      measureText: (text) => ({ width: String(text).length * 6 }),
      globalAlpha: 1,
    },
    {
      get(target, name) {
        if (name in target) return target[name]
        return (...args) => calls.push([name, ...args])
      },
      set(target, name, value) {
        target[name] = value
        return true
      },
    },
  )
  const element = (tag) => {
    const node = {
      tag,
      children: [],
      dataset: {},
      style: {},
      className: '',
      hidden: false,
      textContent: '',
      offsetWidth: 180,
      offsetHeight: 90,
      attributes: {},
      ownerDocument: null,
      append(...nodes) {
        node.children.push(...nodes)
      },
      replaceChildren() {
        node.children = []
      },
      setAttribute(name, value) {
        node.attributes[name] = value
      },
    }
    return node
  }
  const document = {
    createElement(tag) {
      if (tag === 'canvas') return canvas(false)
      const node = element(tag)
      node.ownerDocument = document
      return node
    },
    createTextNode: (text) => ({ tag: '#text', textContent: text }),
    defaultView: { devicePixelRatio: 2, getComputedStyle: () => ({ getPropertyValue: () => '' }) },
  }
  const listeners = new Map()
  const canvas = (main) => ({
    width,
    height,
    clientWidth: width,
    clientHeight: height,
    ownerDocument: document,
    getContext: () => context,
    getBoundingClientRect: () => ({ left: 10, top: 20, width, height }),
    addEventListener: main ? (type, fn) => listeners.set(type, fn) : () => {},
    removeEventListener: main ? (type) => listeners.delete(type) : () => {},
  })
  const tooltip = element('div')
  tooltip.ownerDocument = document
  tooltip.hidden = true
  return { canvas: canvas(true), tooltip, calls, listeners, document }
}

test('drawCandles paints the scene at the device ratio, answers hover and keys, and lets go on destroy', async () => {
  const { drawCandles } = await load('chart.js')
  const dom = fakeDom()
  const selected = []
  const hovered = []
  const chart = drawCandles(dom.canvas, CANDLES, {
    zone: 'Europe/Zurich',
    selected: CANDLES[1].start,
    tooltip: dom.tooltip,
    onSelect: (candle, index) => selected.push(index),
    onHover: (candle) => hovered.push(candle?.start ?? null),
    colors: { bid: 'bid', ask: 'ask', mid: 'mid', spread: 'spread', panel: 'panel', accent: 'accent' },
  })
  assert.ok(chart)
  assert.equal(dom.canvas.width, 1200)
  assert.equal(dom.canvas.height, 640)
  assert.deepEqual(dom.calls.find(([name]) => name === 'setTransform'), ['setTransform', 2, 0, 0, 2, 0, 0])
  // The panel ground first, then five candles (three bid, two ask) each with a wick.
  const fills = dom.calls.filter(([name]) => name === 'fillRect')
  assert.deepEqual(fills[0], ['fillRect', 0, 0, 600, 320])
  const wicks = dom.calls.filter(([name]) => name === 'stroke').length
  assert.ok(wicks >= 5, `at least five wicks, got ${wicks}`)
  assert.ok(dom.calls.some(([name, text]) => name === 'fillText' && text === '14 Aug 14:46'), 'the time axis is labelled in Zurich')
  assert.ok(chart.layout.columns.length === 3)
  assert.deepEqual([...dom.listeners.keys()].sort(), ['blur', 'click', 'keydown', 'pointerleave', 'pointermove'])

  // Hovering the first column fills and places the tooltip; leaving hides it.
  const first = chart.layout.columns[0]
  dom.listeners.get('pointermove')({ clientX: 10 + first.center, clientY: 20 + chart.layout.plot.y + 10 })
  assert.equal(dom.tooltip.hidden, false)
  assert.equal(dom.tooltip.children[0].textContent, '2026-08-14 14:46:00.000 → 2026-08-14 14:47:00.000')
  const rows = dom.tooltip.children[1].children.filter((node) => node.tag === 'dd').map((node) => node.textContent)
  assert.deepEqual(rows, ['72.2 · 72.3 · 72.1 · 72.25', '72.3 · 72.4 · 72.25 · 72.35', '72.3', '0.1', '300', '250', '4 · 1', '300'])
  assert.deepEqual(hovered, [CANDLES[0].start])
  dom.listeners.get('click')({ clientX: 10 + first.center, clientY: 40 })
  assert.deepEqual(selected, [0])
  dom.listeners.get('pointerleave')()
  assert.equal(dom.tooltip.hidden, true)
  assert.deepEqual(hovered, [CANDLES[0].start, null])

  // The keyboard walks the columns from the selection and selects with Enter.
  const key = (name) => dom.listeners.get('keydown')({ key: name, preventDefault() {} })
  key('ArrowRight')
  assert.equal(hovered.at(-1), CANDLES[2].start)
  key('ArrowRight')
  assert.equal(hovered.at(-1), CANDLES[0].start, 'the walk wraps')
  key('ArrowLeft')
  assert.equal(hovered.at(-1), CANDLES[2].start)
  key('Home')
  assert.equal(hovered.at(-1), CANDLES[0].start)
  key('End')
  key('Enter')
  assert.deepEqual(selected, [0, 2])
  key('Escape')
  assert.equal(hovered.at(-1), null)
  assert.equal(dom.tooltip.hidden, true)

  chart.destroy()
  assert.equal(dom.listeners.size, 0)

  // An empty set paints the message and installs the same handlers.
  const empty = fakeDom()
  const nothing = drawCandles(empty.canvas, [], { emptyText: 'Nothing here', colors: { panel: 'panel' } })
  assert.equal(nothing.layout.empty, true)
  assert.ok(empty.calls.some(([name, text]) => name === 'fillText' && text === 'Nothing here'))
  empty.listeners.get('keydown')({ key: 'ArrowRight', preventDefault() {} })
  assert.equal(drawCandles({ getContext: () => null }, CANDLES), null, 'no 2D context, no chart')
})

test('formatInstant spells an instant in a zone from nanoseconds, milliseconds or ISO text', async () => {
  const { formatInstant, instantParts } = await load('audit.js')
  const nanos = 1_786_711_599_769_123_456n // 2026-08-14T12:46:39.769123456Z
  assert.equal(formatInstant(nanos, 'UTC'), '2026-08-14 12:46:39.769')
  assert.equal(formatInstant(nanos, 'Europe/Zurich'), '2026-08-14 14:46:39.769')
  assert.equal(formatInstant(String(nanos), 'UTC', { fraction: 9 }), '2026-08-14 12:46:39.769123456')
  assert.equal(formatInstant('2026-08-14T12:46:39.769Z', 'Asia/Tokyo'), '2026-08-14 21:46:39.769')
  assert.equal(formatInstant('2026-08-14T14:46:39.769123+02:00', 'UTC', { fraction: 6 }), '2026-08-14 12:46:39.769123')
  assert.equal(formatInstant('2026-08-14T12:46:39Z', 'UTC', { fraction: 0, separator: 'T' }), '2026-08-14T12:46:39')
  assert.equal(formatInstant(Date.UTC(2026, 7, 14, 12, 46, 39, 5), 'UTC'), '2026-08-14 12:46:39.005')
  assert.equal(formatInstant(new Date('2026-08-14T00:00:00Z'), 'America/New_York'), '2026-08-13 20:00:00.000')
  // A negative nanosecond count still lands in its millisecond.
  assert.equal(formatInstant(-1n, 'UTC', { fraction: 9 }), '1969-12-31 23:59:59.999999999')
  // An unknown zone reads as UTC; an unreadable value is the empty string.
  assert.equal(formatInstant(0, 'Nowhere/Zone'), '1970-01-01 00:00:00.000')
  for (const value of ['', 'nope', null, undefined, {}, Number.NaN]) assert.equal(formatInstant(value, 'UTC'), '')
  assert.deepEqual(instantParts(nanos), { millis: 1_786_711_599_769, nanos: 123_456 })
  assert.deepEqual(instantParts('2026-08-14T12:46:39.7691Z'), { millis: 1_786_711_599_769, nanos: 100_000 })
})

test('formatDecimal drops the trailing zeros a decimal carries and leaves other text alone', async () => {
  const { formatDecimal } = await load('audit.js')
  assert.equal(formatDecimal('72.280000000000000000'), '72.28')
  assert.equal(formatDecimal('300'), '300')
  assert.equal(formatDecimal('300.000'), '300')
  assert.equal(formatDecimal('-0.500'), '-0.5')
  assert.equal(formatDecimal('-0.000'), '0')
  assert.equal(formatDecimal('+12.50'), '12.5')
  assert.equal(formatDecimal('007.10'), '7.1')
  assert.equal(formatDecimal('0.123456789123'), '0.12345678')
  assert.equal(formatDecimal('0.123456789123', { fraction: 4 }), '0.1234')
  assert.equal(formatDecimal('0.333333333333333333', { fraction: 4 }), '0.3333')
  assert.equal(formatDecimal(1.5), '1.5')
  assert.equal(formatDecimal(null), '')
  assert.equal(formatDecimal(undefined), '')
  assert.equal(formatDecimal(''), '')
  assert.equal(formatDecimal('1e3'), '1e3')
  assert.equal(formatDecimal('market'), 'market')
})

test('sortRows orders decimals by value and text by name, absent cells last either way', async () => {
  const { sortRows, compareCells } = await load('audit.js')
  const rows = [
    { price: '10', side: 'BUY' },
    { price: null, side: 'SELL' },
    { price: '9.5', side: 'BUY' },
    { price: '100', side: null },
  ]
  assert.deepEqual(sortRows(rows, 'price').map((row) => row.price), ['9.5', '10', '100', null])
  assert.deepEqual(sortRows(rows, 'price', 'desc').map((row) => row.price), ['100', '10', '9.5', null])
  assert.deepEqual(sortRows(rows, 'side').map((row) => row.side), ['BUY', 'BUY', 'SELL', null])
  // Stable: the two BUY rows keep their order both ways.
  assert.deepEqual(sortRows(rows, 'side').map((row) => row.price), ['10', '9.5', null, '100'])
  assert.deepEqual(sortRows(rows, 'side', 'desc').map((row) => row.price), ['null', '10', '9.5', '100'].map((price) => (price === 'null' ? null : price)))
  assert.deepEqual(rows.map((row) => row.price), ['10', null, '9.5', '100'], 'the input is untouched')
  assert.equal(compareCells('2', '10') < 0, true)
  assert.equal(compareCells('b', 'a') > 0, true)
  assert.equal(compareCells(null, undefined), 0)
  assert.equal(compareCells('', 'x') > 0, true)
})

test('the API builds one URL per route from a plain query, in the route order, skipping what is unset', async () => {
  const api = await load('api.js')
  assert.equal(api.queryString({}), '')
  assert.equal(api.queryString({ a: 1, b: null, c: undefined, d: 'x y/z' }), '?a=1&d=x%20y%2Fz')
  assert.equal(api.normalizeBase('http://h:1/books'), 'http://h:1/books/')
  assert.equal(api.normalizeBase('http://h:1/books/?x=1#y'), 'http://h:1/books/')
  assert.equal(api.apiUrl('http://h:1/books', 'api/tables'), 'http://h:1/books/api/tables')
  const query = { table: 'books', ticker: 'ABBN.S', from: '2026-08-14T14:46:00', to: '2026-08-14T15:00:00', tz: 'Europe/Zurich', interval: '5m' }
  assert.equal(
    api.apiUrl('http://h:1/', 'api/candles', api.candleParams(query)),
    'http://h:1/api/candles?table=books&ticker=ABBN.S&from=2026-08-14T14%3A46%3A00&to=2026-08-14T15%3A00%3A00&tz=Europe%2FZurich&interval=5m',
  )
  assert.deepEqual(Object.keys(api.bookParams({ tz: 'UTC', at: 'x', ticker: 't', table: 'b' })), ['table', 'ticker', 'at', 'tz'])
  assert.deepEqual(Object.keys(api.eventParams({})), ['table', 'ticker', 'from', 'to', 'tz', 'side', 'limit'])
  assert.equal(api.auditUrl('http://h:1/', { ...query, side: 'bid' }, '.csv.gz'), 'http://h:1/api/audit.csv.gz?table=books&ticker=ABBN.S&from=2026-08-14T14%3A46%3A00&to=2026-08-14T15%3A00%3A00&tz=Europe%2FZurich&side=bid')
  assert.equal(api.auditUrl('http://h:1/', query), 'http://h:1/api/audit.csv?table=books&ticker=ABBN.S&from=2026-08-14T14%3A46%3A00&to=2026-08-14T15%3A00%3A00&tz=Europe%2FZurich')
  assert.throws(() => api.auditUrl('http://h:1/', query, '.parquet'), /audit suffix must be one of \.csv, \.csv\.gz, \.csv\.zst, got "\.parquet"/)
  assert.deepEqual([...api.AUDIT_SUFFIXES], ['.csv', '.csv.gz', '.csv.zst'])
})

test('the fetchers ask their route through the fetch they are handed and refuse with the service text', async () => {
  const api = await load('api.js')
  const calls = []
  const answering = (status, body) => async (url, init) => {
    calls.push({ url, accept: init.headers.accept })
    return { ok: status < 400, status, statusText: 'Whatever', text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) }
  }
  const tables = await api.fetchTables('http://h:1/p', { fetch: answering(200, [{ name: 'books', url: 'file:///b' }]) })
  assert.deepEqual(tables, [{ name: 'books', url: 'file:///b' }])
  assert.deepEqual(calls, [{ url: 'http://h:1/p/api/tables', accept: 'application/json' }])
  await api.fetchTickers('http://h:1/', 'books', { fetch: answering(200, []) })
  assert.equal(calls[1].url, 'http://h:1/api/tickers?table=books')
  await api.fetchBook('http://h:1/', { table: 'b', ticker: 't', at: '2026-08-14T14:47:00+02:00', tz: 'UTC' }, { fetch: answering(200, {}) })
  assert.equal(calls[2].url, 'http://h:1/api/book?table=b&ticker=t&at=2026-08-14T14%3A47%3A00%2B02%3A00&tz=UTC')
  await api.fetchEvents('http://h:1/', { table: 'b', ticker: 't', from: 'a', to: 'z', tz: 'UTC', side: 'ask', limit: 5000 }, { fetch: answering(200, { rows: [], truncated: false }) })
  assert.equal(calls[3].url, 'http://h:1/api/events?table=b&ticker=t&from=a&to=z&tz=UTC&side=ask&limit=5000')
  await api.fetchCandles('http://h:1/', { table: 'b', ticker: 't', from: 'a', to: 'z', tz: 'UTC', interval: '1m' }, { fetch: answering(200, { candles: [] }) })
  assert.equal(calls[4].url, 'http://h:1/api/candles?table=b&ticker=t&from=a&to=z&tz=UTC&interval=1m')

  await assert.rejects(api.fetchTickers('http://h:1/', 'nope', { fetch: answering(404, { error: 'no table named nope' }) }), (error) => {
    assert.ok(error instanceof api.ApiError)
    assert.equal(error.message, 'no table named nope')
    assert.equal(error.status, 404)
    assert.equal(error.url, 'http://h:1/api/tickers?table=nope')
    return true
  })
  await assert.rejects(api.fetchTables('http://h:1/', { fetch: answering(500, 'boom') }), /500 Whatever/)
  await assert.rejects(api.fetchTables('http://h:1/', { fetch: answering(200, 'not json') }), /the answer is not JSON/)
  await assert.rejects(
    api.fetchTables('http://h:1/', {
      fetch: async () => {
        throw new TypeError('network down')
      },
    }),
    /the request failed: network down/,
  )
  await assert.rejects(api.fetchTables('http://h:1/', { fetch: null }), /no fetch implementation/)
})

test('downloadLink names the file after the ticker and the range and states the coding', async () => {
  const { downloadLink, AUDIT_TYPES, EVENT_COLUMNS } = await load('audit.js')
  const query = { table: 'books', ticker: 'ABBN.S', from: '2026-08-14T14:46:00', to: '2026-08-14T15:00:00', tz: 'UTC' }
  const csv = downloadLink('http://h:1/', query, '.csv')
  assert.equal(csv.href, 'http://h:1/api/audit.csv?table=books&ticker=ABBN.S&from=2026-08-14T14%3A46%3A00&to=2026-08-14T15%3A00%3A00&tz=UTC')
  assert.equal(csv.filename, 'audit-ABBN.S-2026-08-14T14-46-00-2026-08-14T15-00-00.csv')
  assert.equal(csv.label, 'CSV')
  assert.equal(csv.type, 'text/csv')
  assert.equal(downloadLink('http://h:1/', query, '.csv.gz').filename, 'audit-ABBN.S-2026-08-14T14-46-00-2026-08-14T15-00-00.csv.gz')
  assert.equal(downloadLink('http://h:1/', query, '.csv.gz').type, 'application/gzip')
  assert.equal(downloadLink('http://h:1/', query, '.csv.zst').label, 'CSV · zstd')
  assert.equal(downloadLink('http://h:1/', query, '.csv.zst').type, 'application/zstd')
  assert.throws(() => downloadLink('http://h:1/', query, '.csv.xz'), /audit suffix must be one of/)
  assert.deepEqual(Object.keys(AUDIT_TYPES), ['.csv', '.csv.gz', '.csv.zst'])
  assert.deepEqual(
    EVENT_COLUMNS.map((column) => column.key),
    ['currunix', 'role', 'marketdatakind', 'side', 'price', 'quantity', 'state', 'crosscode', 'curruuid', 'prevuuid'],
  )
})

test('the theme cycles system, light, dark, resolves against the system, and survives a broken storage', async () => {
  const theme = await load('theme.js')
  assert.deepEqual([...theme.THEMES], ['system', 'light', 'dark'])
  assert.equal(theme.nextTheme('system'), 'light')
  assert.equal(theme.nextTheme('light'), 'dark')
  assert.equal(theme.nextTheme('dark'), 'system')
  assert.equal(theme.nextTheme('bogus'), 'system')
  assert.equal(theme.resolveTheme('system', true), 'dark')
  assert.equal(theme.resolveTheme('system', false), 'light')
  assert.equal(theme.resolveTheme('light', true), 'light')
  assert.equal(theme.resolveTheme('dark', false), 'dark')
  assert.equal(theme.resolveTheme(undefined, false), 'light')

  const store = new Map()
  const storage = { getItem: (key) => store.get(key) ?? null, setItem: (key, value) => store.set(key, value), removeItem: (key) => store.delete(key) }
  const root = { dataset: {} }
  assert.equal(theme.readTheme(storage), 'system')
  assert.equal(theme.applyTheme('dark', { root, storage, prefersDark: false }), 'dark')
  assert.equal(root.dataset.theme, 'dark')
  assert.equal(store.get(theme.STORAGE_KEY), 'dark')
  assert.equal(theme.readTheme(storage), 'dark')
  assert.equal(theme.toggleTheme('dark', { root, storage, prefersDark: true }), 'system')
  assert.equal(root.dataset.theme, undefined)
  assert.equal(store.has(theme.STORAGE_KEY), false)
  assert.equal(theme.applyTheme('bogus', { root, storage, prefersDark: true }), 'dark')
  assert.equal(root.dataset.theme, undefined)
  store.set(theme.STORAGE_KEY, 'purple')
  assert.equal(theme.readTheme(storage), 'system')

  const broken = {
    getItem() {
      throw new Error('blocked')
    },
    setItem() {
      throw new Error('blocked')
    },
    removeItem() {
      throw new Error('blocked')
    },
  }
  assert.equal(theme.readTheme(broken), 'system')
  assert.equal(theme.applyTheme('light', { root, storage: broken, prefersDark: true }), 'light')
  assert.equal(root.dataset.theme, 'light')
  assert.equal(theme.systemPrefersDark(undefined), false)
  assert.equal(theme.systemPrefersDark({ matchMedia: () => ({ matches: true }) }), true)
  assert.equal(theme.themeLabel('dark'), 'Dark theme')
  assert.equal(theme.themeLabel('system'), 'System theme')
})

test('the app reads and writes its hash, orders the zones, spans a ticker and finds its API base', async () => {
  const app = await load('app.js')
  assert.deepEqual([...app.INTERVALS], ['30s', '1m', '5m', '15m', '1h', '1d'])
  const state = { table: 'books', ticker: 'ABBN.S', from: '2026-08-14T14:46:00', to: '2026-08-14T15:00:00', tz: 'Europe/Zurich', interval: '1m', at: '2026-08-14T14:47:00+02:00' }
  const hash = app.writeHash(state)
  assert.equal(
    hash,
    '#table=books&ticker=ABBN.S&from=2026-08-14T14%3A46%3A00&to=2026-08-14T15%3A00%3A00&tz=Europe%2FZurich&interval=1m&at=2026-08-14T14%3A47%3A00%2B02%3A00',
  )
  assert.deepEqual(app.readHash(hash), state)
  assert.deepEqual(app.readHash('#table=books&ticker=&junk=1'), { table: 'books' })
  assert.deepEqual(app.readHash(''), {})
  assert.equal(app.writeHash({}), '')
  assert.equal(app.writeHash({ at: null, from: '' }), '')

  assert.deepEqual(app.timezoneChoices(['UTC', 'Asia/Tokyo', 'Europe/Zurich', 'America/New_York'], 'Europe/Zurich'), ['Europe/Zurich', 'UTC', 'America/New_York', 'Asia/Tokyo'])
  assert.deepEqual(app.timezoneChoices(['Asia/Tokyo'], 'UTC'), ['UTC', 'Asia/Tokyo'])
  assert.deepEqual(app.timezoneChoices([], ''), ['UTC'])

  assert.deepEqual(app.spanToRange('2026-08-14T12:46:39.769Z', '2026-08-14T21:59:49.868Z', 'Europe/Zurich'), { from: '2026-08-14T14:46:39', to: '2026-08-14T23:59:50' })
  assert.deepEqual(app.spanToRange('2026-08-14T12:46:39Z', '2026-08-14T12:46:39Z', 'UTC'), { from: '2026-08-14T12:46:39', to: '2026-08-14T12:46:40' })
  assert.deepEqual(app.spanToRange('nope', '2026-08-14T12:46:39Z'), { from: '', to: '' })

  assert.equal(app.apiBase('http://127.0.0.1:8080/books/index.html?x=1#y'), 'http://127.0.0.1:8080/books/')
  assert.equal(app.apiBase('http://127.0.0.1:8080/books'), 'http://127.0.0.1:8080/books/')
  assert.equal(app.apiBase('http://127.0.0.1:8080/books/'), 'http://127.0.0.1:8080/books/')
  assert.equal(app.apiBase('http://127.0.0.1:8080/'), 'http://127.0.0.1:8080/')
  assert.equal(app.apiBase('http://127.0.0.1:8080'), 'http://127.0.0.1:8080/')

  let fired = 0
  const timers = []
  const debounced = app.debounce(() => (fired += 1), 10, (fn) => timers.push(fn) - 1, (handle) => (timers[handle] = null))
  debounced()
  debounced()
  debounced()
  assert.equal(timers.filter(Boolean).length, 1)
  timers.filter(Boolean).forEach((fn) => fn())
  assert.equal(fired, 1)
})

test('serveArguments spells the command line: tables, bind, path, one --capture per log, then the rest', () => {
  assert.deepEqual(book.serveArguments(), ['serve', '--bind', '127.0.0.1:0', '--path', '/'])
  assert.deepEqual(
    book.serveArguments({
      tables: ['books=/data/books', { name: 'ref', location: 's3://bucket/ref' }, { location: '/data/other' }],
      bind: '0.0.0.0:8080',
      path: '/book',
      capture: ['a.log', 'b.log'],
      args: ['--snapshot-millis', 250],
    }),
    ['serve', 'books=/data/books', 'ref=s3://bucket/ref', '/data/other', '--bind', '0.0.0.0:8080', '--path', '/book', '--capture', 'a.log', '--capture', 'b.log', '--snapshot-millis', '250'],
  )
  assert.deepEqual(book.serveArguments({ tables: 'books=/data/books', capture: 'a.log' }), ['serve', 'books=/data/books', '--bind', '127.0.0.1:0', '--path', '/', '--capture', 'a.log'])
  assert.throws(() => book.serveArguments({ tables: [42] }), /a table is 'name=location', a location, or \{ name, location \}, got 42/)
  assert.throws(() => book.serveArguments({ tables: [{ name: 'x' }] }), TypeError)
  assert.throws(() => book.serveArguments({ tables: [''] }), TypeError)
})

test('serve rejects cleanly when the binary is absent', async () => {
  await assert.rejects(book.serve({ bin: '/nonexistent/yggdryl', tables: ['books=/tmp/books'] }), (error) => {
    assert.match(error.message, /cannot start \/nonexistent\/yggdryl: spawn \/nonexistent\/yggdryl ENOENT/)
    assert.equal(error.cause.code, 'ENOENT')
    return true
  })
})

test('serve resolves on the endpoint line, and rejects with stderr or a line that is no URL', { skip: process.platform === 'win32' && 'a shell script stands in for the binary' }, async () => {
  const folder = mkdtempSync(join(tmpdir(), 'yggdryl-book-'))
  const script = (name, body) => {
    const path = join(folder, name)
    writeFileSync(path, `#!/bin/sh\n${body}\n`)
    chmodSync(path, 0o755)
    return path
  }
  try {
    const serving = script('serving', 'echo "table books over file:///tmp/books" >&2\necho "http://127.0.0.1:4242/book/"\necho "note after the endpoint"\nexec sleep 30')
    const started = await book.serve({ bin: serving, tables: ['books=/tmp/books'], path: '/book' })
    assert.equal(started.endpoint, 'http://127.0.0.1:4242/book/')
    assert.equal(started.process.exitCode, null)
    const status = await started.close()
    assert.equal(status.signal, 'SIGTERM')
    assert.deepEqual(await started.close(), status, 'closing twice answers the same status')

    const refusing = script('refusing', 'echo "a capture needs a table to land in" >&2\nexit 2')
    await assert.rejects(book.serve({ bin: refusing, capture: ['x.log'] }), /yggdryl serve exited with 2 before printing its endpoint: a capture needs a table to land in/)

    const babbling = script('babbling', 'echo "hello"\nexec sleep 30')
    await assert.rejects(book.serve({ bin: babbling, tables: ['b=/tmp/b'] }), /expected the endpoint on the first line of yggdryl serve, got "hello"/)

    const silent = script('silent', 'exit 0')
    await assert.rejects(book.serve({ bin: silent }), /yggdryl serve exited with 0 before printing its endpoint$/)
  } finally {
    rmSync(folder, { recursive: true, force: true })
  }
})
