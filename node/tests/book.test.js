'use strict'

// The book display and its spawner: `node/book.js` and the ES modules under
// `node/book/`. The modules are imported under Node, so nothing they do at
// import time may need a document, and their pure helpers are pinned here
// without one.

const assert = require('node:assert/strict')
const test = require('node:test')
const { chmodSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { pathToFileURL } = require('node:url')

const book = require('../book.js')

const source = (name) => readFileSync(join(book.assets, name), 'utf8')
const load = (name) => import(pathToFileURL(join(book.assets, name)).href)

/** An instant as the service writes it in Europe/Zurich's summer: nine fraction digits, the offset and the bracketed zone. */
const zurich = (wall) => `${wall}.000000000+02:00[Europe/Zurich]`

const CANDLES = [
  {
    start: zurich('2026-08-14T14:46:00'),
    end: zurich('2026-08-14T14:47:00'),
    bid: { open: '72.20', high: '72.30', low: '72.10', close: '72.25' },
    ask: { open: '72.30', high: '72.40', low: '72.25', close: '72.35' },
    mid: { open: '72.25', high: '72.35', low: '72.175', close: '72.30' },
    spread: { open: '0.10', high: '0.15', low: '0.05', close: '0.10' },
    bidqty: '300',
    askqty: '250',
    books: 4,
  },
  {
    start: zurich('2026-08-14T14:47:00'),
    end: zurich('2026-08-14T14:48:00'),
    bid: { open: '72.25', high: '72.45', low: '72.20', close: '72.40' },
    ask: null,
    mid: null,
    spread: null,
    bidqty: '100',
    askqty: null,
    books: 2,
  },
  {
    start: zurich('2026-08-14T14:49:00'),
    end: zurich('2026-08-14T14:50:00'),
    bid: { open: '72.40', high: '72.40', low: '72.00', close: '72.05' },
    ask: { open: '72.50', high: '72.60', low: '72.10', close: '72.15' },
    mid: { open: '72.45', high: '72.5', low: '72.05', close: '72.10' },
    spread: { open: '0.10', high: '0.20', low: '0.10', close: '0.10' },
    bidqty: '50',
    askqty: '75',
    books: 3,
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
  for (const name of ['fetchTables', 'fetchTimezones', 'fetchTickers', 'fetchCandles', 'fetchBook', 'fetchEvents', 'auditUrl', 'queryString']) {
    assert.equal(typeof api[name], 'function', name)
  }
  for (const name of ['drawCandles', 'nearestCandle', 'scaleLinear', 'niceTicks', 'layoutCandles', 'timeTicks', 'describeCandle']) {
    assert.equal(typeof chart[name], 'function', name)
  }
  for (const name of ['renderSummary', 'renderEvents', 'downloadLink', 'formatInstant', 'formatDecimal', 'sortRows', 'instantParts', 'instantNanos', 'instantText']) {
    assert.equal(typeof audit[name], 'function', name)
  }
  for (const name of ['start', 'readHash', 'writeHash', 'timezoneChoices', 'apiBase']) assert.equal(typeof app[name], 'function', name)
  assert.equal(app.spanToRange, undefined, 'the range travels as instants, never as wall clocks')
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
  // The arrow keys reach the chart under a screen reader, and what they read is announced.
  assert.match(html, /<canvas[^>]*role="application"[^>]*aria-roledescription="[^"]+"/)
  assert.match(html, /<p id="readout" class="sr-only" aria-live="polite"><\/p>/)
  // The skip link has an id the app moves focus through.
  assert.match(html, /<a id="skip" class="skip" href="#chart">/)
})

test('theme.css declares the tokens in the light scheme and in both dark scopes', () => {
  const css = source('theme.css')
  const tokens = ['--bg', '--fg', '--muted', '--panel', '--border', '--bid', '--ask', '--mid', '--accent', '--danger']
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
  // `hidden` hides whatever display an author rule gives the element - the status pill's `inline-block` included.
  assert.match(css, /\[hidden\]\s*\{\s*display:\s*none\s*!important;\s*\}/)
  assert.match(css, /\.sr-only\s*\{[^}]*clip:/)
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

/**
 * What `body` answers, run in a worker that is ended after `ms` and held to a
 * small heap: a layout that never returns fails the test instead of hanging
 * the runner or filling the machine's memory with ticks.
 */
function within(ms, body) {
  const { Worker } = require('node:worker_threads')
  const url = pathToFileURL(join(book.assets, 'chart.js')).href
  const worker = new Worker(
    `const { parentPort } = require('node:worker_threads')
    import(${JSON.stringify(url)}).then((chart) => parentPort.postMessage((${body})(chart)))`,
    { eval: true, resourceLimits: { maxOldGenerationSizeMb: 64, maxYoungGenerationSizeMb: 16 } },
  )
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      worker.terminate()
      reject(new Error(`still running after ${ms}ms`))
    }, ms)
    worker.once('message', (value) => {
      clearTimeout(timer)
      worker.terminate()
      resolve(value)
    })
    worker.once('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
  })
}

test('niceTicks returns on any finite range, down to prices a few ulps apart', async () => {
  // 72.28 and 72.28000000000003 are adjacent doubles: the step is so small that
  // counting ticks by `index += 1` from `min / step` never moved past 2^53.
  const answer = await within(5_000, `(chart) => {
    const candle = { start: '2026-08-14T12:46:00Z', end: '2026-08-14T12:47:00Z', bid: { open: '72.28', high: '72.28', low: '72.28', close: '72.28' }, ask: { open: '72.28000000000003', high: '72.28000000000003', low: '72.28000000000003', close: '72.28000000000003' } }
    const layout = chart.layoutCandles([candle], { width: 800, height: 400 })
    return {
      adjacent: chart.niceTicks(72.28, 72.28000000000003, 5),
      layout: { empty: layout.empty, ticks: layout.priceTicks.length, domain: layout.price.domain },
      wide: chart.niceTicks(-1e308, 1e308, 5),
      many: chart.niceTicks(0, 2 ** 52, 2 ** 52).length,
    }
  }`)
  assert.deepEqual(answer.adjacent, [72.28])
  assert.equal(answer.layout.empty, false)
  // A range narrower than a double can step through is padded as a flat one: one percent each side.
  assert.ok(answer.layout.domain[0] < 72 && answer.layout.domain[1] > 73, `domain ${answer.layout.domain}`)
  assert.ok(answer.layout.ticks >= 2 && answer.layout.ticks <= 12, `${answer.layout.ticks} ticks`)
  assert.deepEqual(answer.wide, [])
  assert.ok(answer.many > 0 && answer.many <= 2_501, `${answer.many} ticks`)
})

test('layoutCandles places every bucket at its instant, bid left and ask right, over both panes', async () => {
  const { layoutCandles, DEFAULT_PADDING } = await load('chart.js')
  const layout = layoutCandles(CANDLES, { width: 452, height: 300 })
  assert.equal(layout.empty, false)
  assert.equal(layout.priced, true)
  assert.equal(layout.interval, 60_000)
  // Three candles over four minutes: each column spans its own bucket's edges on the time scale, so the third stands in the fourth minute.
  assert.deepEqual(layout.columns.map((column) => column.index), [0, 1, 2])
  const inner = 452 - DEFAULT_PADDING.left - DEFAULT_PADDING.right
  assert.equal(layout.slot, inner / 4)
  for (const column of layout.columns) {
    assert.equal(column.x0, layout.time(column.start))
    assert.equal(column.x1, layout.time(column.end))
  }
  assert.equal(layout.columns[2].x0, DEFAULT_PADDING.left + 3 * layout.slot)
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
    assert.equal(layout.priced, false)
    assert.deepEqual(layout.columns, [])
    assert.deepEqual(layout.priceTicks, [])
  }
})

test('layoutCandles places every column from its own edges, so buckets of unequal length never share a place', async () => {
  const { layoutCandles, parseInstant } = await load('chart.js')
  // Daily Zurich candles from the day saving time ends: the first is 25 hours, the rest 24.
  const midnight = (day) => (day === 25 ? `2026-10-${day}T00:00:00+02:00[Europe/Zurich]` : `${new Date(Date.UTC(2026, 9, day)).toISOString().slice(0, 10)}T00:00:00+01:00[Europe/Zurich]`)
  const days = Array.from({ length: 21 }, (_, index) => ({
    start: midnight(25 + index),
    end: midnight(26 + index),
    bid: { open: '1', high: '2', low: '1', close: '2' },
    ask: null,
    mid: null,
    spread: null,
    books: 1,
  }))
  assert.equal(parseInstant(days[0].end) - parseInstant(days[0].start), 25 * 3_600_000)
  const layout = layoutCandles(days, { width: 900, height: 300 })
  assert.equal(layout.columns.length, 21)
  assert.equal(layout.interval, 24 * 3_600_000, 'the shortest bucket')
  layout.columns.forEach((column, index) => {
    assert.equal(column.x0, layout.time(parseInstant(days[index].start)))
    assert.equal(column.x1, layout.time(parseInstant(days[index].end)))
    if (index > 0) assert.ok(column.x0 >= layout.columns[index - 1].x1 - 1e-9, `day ${index} starts where the day before ends`)
  })
  assert.equal(new Set(layout.columns.map((column) => column.center)).size, 21, 'no two days share a place')
})

test('candles with no bid, ask or mid keep their columns and the time axis, and say there is no price', async () => {
  const { layoutCandles, timeTicks, drawCandles } = await load('chart.js')
  const unpriced = CANDLES.map((candle) => ({ ...candle, bid: null, ask: null, mid: null, spread: null, bidqty: null, askqty: null }))
  const layout = layoutCandles(unpriced, { width: 452, height: 300 })
  assert.equal(layout.empty, false)
  assert.equal(layout.priced, false)
  assert.equal(layout.columns.length, 3)
  assert.deepEqual(layout.priceTicks, [])
  assert.ok(timeTicks(layout, 'Europe/Zurich').length > 0)
  const dom = fakeDom()
  const chart = drawCandles(dom.canvas, unpriced, { zone: 'Europe/Zurich', tooltip: dom.tooltip, colors: { panel: 'panel' } })
  const texts = dom.calls.filter(([name]) => name === 'fillText').map(([, text]) => text)
  assert.ok(texts.includes('No bid or ask in this range'), texts.join(' | '))
  assert.ok(!texts.includes('No candles in this range'))
  assert.ok(texts.includes('14 Aug 14:46'), 'the time axis stands')
  // The count stays reachable: hovering a column reads how many books it folded.
  chart.hover(0)
  const rows = dom.tooltip.children[1].children.filter((node) => node.tag === 'dd').map((node) => node.textContent)
  assert.deepEqual(rows, ['4'])
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

test('the time axis is labelled in the zone asked at the bucket edges, dated where the day changes', async () => {
  const { layoutCandles, timeTicks, formatTick } = await load('chart.js')
  const layout = layoutCandles(CANDLES, { width: 452, height: 300 })
  const zurich = timeTicks(layout, 'Europe/Zurich')
  assert.deepEqual(zurich.map((tick) => tick.label), ['14 Aug 14:46', '14:47', '14:49', '14:50'])
  assert.equal(zurich[0].x, layout.plot.x)
  assert.equal(zurich.at(-1).x, layout.plot.x + layout.plot.width)
  const tokyo = timeTicks(layout, 'Asia/Tokyo')
  assert.equal(tokyo[0].label, '14 Aug 21:46')
  assert.equal(formatTick(Date.parse('2026-08-14T00:00:00Z'), 'UTC', 86_400_000), '14 Aug')
  // The shortest daily bucket across a saving-time change is 23 hours, and still a day.
  assert.equal(formatTick(Date.parse('2026-03-29T00:00:00+01:00'), 'Europe/Zurich', 23 * 3_600_000), '29 Mar')
  assert.equal(formatTick(Date.parse('2026-10-25T01:00:00Z'), 'Europe/Zurich', 2 * 3_600_000), '02:00')
  assert.equal(formatTick(Date.parse('2026-08-14T00:00:00Z'), 'America/New_York', 3_600_000, true), '13 Aug 20:00')
  assert.equal(formatTick(Date.parse('2026-08-14T00:00:00Z'), 'Not/AZone', 3_600_000), '00:00')
  assert.deepEqual(timeTicks(layoutCandles([], { width: 400, height: 200 })), [])
})

test('the time axis keeps its labels apart at phone width', async () => {
  const { layoutCandles, timeTicks } = await load('chart.js')
  const minute = (index) => new Date(Date.UTC(2026, 7, 14, 12, 45 + index)).toISOString().replace('.000Z', '.000000000Z')
  const candles = Array.from({ length: 30 }, (_, index) => ({ start: minute(index), end: minute(index + 1), bid: { open: '1', high: '2', low: '1', close: '2' } }))
  const measure = (text) => text.length * 6.5
  for (const width of [300, 360, 800]) {
    const layout = layoutCandles(candles, { width, height: 300 })
    const ticks = timeTicks(layout, 'UTC', { measure })
    assert.ok(ticks.length >= 2, `${width}px keeps at least two labels`)
    assert.equal(ticks[0].label, '14 Aug 12:45')
    for (let index = 1; index < ticks.length; index += 1) {
      const before = ticks[index - 1]
      assert.ok(ticks[index].left >= before.left + measure(before.label) + 12, `${width}px: '${before.label}' and '${ticks[index].label}' overlap`)
    }
    assert.ok(ticks.every((tick) => tick.left >= 0 && tick.left + measure(tick.label) <= width), `${width}px: every label inside the canvas`)
  }
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
  const { drawCandles, describeCandle } = await load('chart.js')
  const dom = fakeDom()
  const selected = []
  const hovered = []
  const readout = { textContent: '' }
  const chart = drawCandles(dom.canvas, CANDLES, {
    zone: 'Europe/Zurich',
    // The selection is an instant, whichever spelling states it.
    selected: '2026-08-14T12:47:00Z',
    tooltip: dom.tooltip,
    readout,
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
  assert.deepEqual(rows, ['72.2 · 72.3 · 72.1 · 72.25', '72.3 · 72.4 · 72.25 · 72.35', '72.3', '0.1', '300', '250', '4'])
  assert.deepEqual(hovered, [CANDLES[0].start])
  dom.listeners.get('click')({ clientX: 10 + first.center, clientY: 40 })
  assert.deepEqual(selected, [0])
  dom.listeners.get('pointerleave')()
  assert.equal(dom.tooltip.hidden, true)
  assert.deepEqual(hovered, [CANDLES[0].start, null])
  assert.equal(readout.textContent, '', 'the pointer is not announced')

  // The keyboard walks the columns from the selection and selects with Enter.
  const key = (name) => dom.listeners.get('keydown')({ key: name, preventDefault() {} })
  key('ArrowRight')
  assert.equal(hovered.at(-1), CANDLES[2].start)
  // What the arrow keys read is announced through the live region, not only painted.
  assert.equal(readout.textContent, describeCandle(CANDLES[2], 'Europe/Zurich'))
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

  assert.equal(
    describeCandle(CANDLES[0], 'Europe/Zurich'),
    '2026-08-14 14:46:00 to 14:47:00: bid open 72.2, high 72.3, low 72.1, close 72.25; ask open 72.3, high 72.4, low 72.25, close 72.35; mid close 72.3; spread close 0.1; bid quantity 300; ask quantity 250; 4 books',
  )
  assert.equal(describeCandle({ ...CANDLES[1], books: 1 }, 'UTC'), '2026-08-14 12:47:00 to 12:48:00: bid open 72.25, high 72.45, low 72.2, close 72.4; bid quantity 100; 1 book')

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

test('the one instant reader reads what the service writes, and a wall clock in a zone', async () => {
  const { instantParts, instantNanos, instantText, formatInstant } = await load('audit.js')
  const { parseInstant, layoutCandles } = await load('chart.js')
  // Outside UTC the service writes RFC 9557 text: nine fraction digits, the offset, the zone in brackets.
  const written = '2026-08-14T14:46:39.769123456+02:00[Europe/Zurich]'
  assert.deepEqual(instantParts(written), { millis: 1_786_711_599_769, nanos: 123_456 })
  assert.equal(parseInstant(written), 1_786_711_599_769)
  assert.equal(parseInstant('2026-08-14T12:46:39.769123456Z'), 1_786_711_599_769)
  assert.equal(instantNanos(written), 1_786_711_599_769_123_456n)
  assert.equal(formatInstant(written, 'UTC', { fraction: 9 }), '2026-08-14 12:46:39.769123456')
  assert.equal(formatInstant(written, 'Europe/Zurich', { fraction: 9 }), '2026-08-14 14:46:39.769123456')
  assert.equal(layoutCandles(CANDLES, { width: 452, height: 300 }).columns.length, 3, 'the service-spelled candles lay out')
  // The one text the display sends: UTC, `Z`, the fraction only where it is not zero.
  assert.equal(instantText(written), '2026-08-14T12:46:39.769123456Z')
  assert.equal(instantText('2026-08-14T12:46:00.000000000Z'), '2026-08-14T12:46:00Z')
  assert.equal(instantText(1_786_711_619_999_999_999n), '2026-08-14T12:46:59.999999999Z')
  assert.equal(instantText(1_786_711_560_500_000_000n), '2026-08-14T12:46:00.5Z')
  // A wall clock - what a datetime-local input states, to the minute or the second - is read in the zone given, UTC by default.
  assert.equal(instantText('2026-08-14T14:50', 'Europe/Zurich'), '2026-08-14T12:50:00Z')
  assert.equal(instantText('2026-08-14T14:50:07', 'Europe/Zurich'), '2026-08-14T12:50:07Z')
  assert.equal(instantText('2026-08-14T12:50'), '2026-08-14T12:50:00Z')
  assert.equal(instantText('2026-08-14 12:50:00'), '2026-08-14T12:50:00Z')
  // A bracketed zone with no offset is the zone of the wall clock.
  assert.equal(instantText('2026-08-14T14:50:00[Europe/Zurich]'), '2026-08-14T12:50:00Z')
  // A wall clock saving time repeats is its first occurrence; one it skips reads at the offset before the jump.
  assert.equal(instantText('2026-10-25T02:30:00', 'Europe/Zurich'), '2026-10-25T00:30:00Z')
  assert.equal(instantText('2026-03-29T02:30:00', 'Europe/Zurich'), '2026-03-29T01:30:00Z')
  assert.equal(formatInstant(instantNanos('2026-10-25T02:10:00', 'Europe/Zurich'), 'Europe/Zurich', { fraction: 0 }), '2026-10-25 02:10:00')
  assert.equal(instantText('2026-08-14T12:50:00-04:00'), '2026-08-14T16:50:00Z')
  for (const text of ['2026-02-30T00:00:00Z', '2026-08-14T24:00:00Z', '2026-08-14', 'nope', '2026-08-14T12:46:39.1234567891Z', '2026-08-14T12:60:00Z']) {
    assert.equal(instantParts(text), null, text)
    assert.equal(instantNanos(text), null, text)
    assert.equal(instantText(text), '', text)
  }
  assert.equal(instantNanos(null), null)
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
    { price: '10', side: 'BUYS' },
    { price: null, side: 'SELL' },
    { price: '9.5', side: 'BUYS' },
    { price: '100', side: null },
  ]
  assert.deepEqual(sortRows(rows, 'price').map((row) => row.price), ['9.5', '10', '100', null])
  assert.deepEqual(sortRows(rows, 'price', 'desc').map((row) => row.price), ['100', '10', '9.5', null])
  assert.deepEqual(sortRows(rows, 'side').map((row) => row.side), ['BUYS', 'BUYS', 'SELL', null])
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

/** A document of `FakeElement`s: what `audit.js` and `app.js` build and reach, with focus and events. */
class FakeElement {
  constructor(document, tag, id = '') {
    this.ownerDocument = document
    this.tag = tag
    this.id = id
    this.children = []
    this.parentElement = null
    this.dataset = {}
    this.style = {}
    this.attributes = {}
    this.listeners = {}
    this.hidden = false
    this.disabled = false
    this.value = ''
    this.className = ''
    this.text = ''
  }

  get textContent() {
    return this.children.length > 0 ? this.children.map((child) => child.textContent).join('') : this.text
  }

  set textContent(value) {
    this.children = []
    this.text = String(value)
  }

  get classList() {
    const names = () => this.className.split(/\s+/).filter(Boolean)
    return {
      add: (...more) => {
        this.className = [...new Set([...names(), ...more])].join(' ')
      },
      remove: (...less) => {
        this.className = names().filter((name) => !less.includes(name)).join(' ')
      },
      contains: (name) => names().includes(name),
    }
  }

  append(...nodes) {
    for (const node of nodes) {
      node.parentElement = this
      this.children.push(node)
    }
  }

  replaceChildren(...nodes) {
    for (const child of this.children) child.parentElement = null
    this.children = []
    this.text = ''
    this.append(...nodes)
  }

  setAttribute(name, value) {
    this.attributes[name] = String(value)
  }

  getAttribute(name) {
    return name in this.attributes ? this.attributes[name] : null
  }

  addEventListener(type, listener) {
    ;(this.listeners[type] ??= []).push(listener)
  }

  removeEventListener(type, listener) {
    this.listeners[type] = (this.listeners[type] ?? []).filter((held) => held !== listener)
  }

  dispatch(type, init = {}) {
    const event = { type, defaultPrevented: false, preventDefault: () => (event.defaultPrevented = true), ...init }
    for (const listener of [...(this.listeners[type] ?? [])]) listener(event)
    return event
  }

  focus() {
    this.ownerDocument.activeElement = this
  }

  /** Whether this element hangs under `ancestor`. */
  within(ancestor) {
    for (let node = this; node; node = node.parentElement) if (node === ancestor) return true
    return false
  }

  /** Every element under this one that `predicate` keeps, in document order. */
  findAll(predicate) {
    const found = []
    const walk = (node) => {
      for (const child of node.children ?? []) {
        if (child instanceof FakeElement) {
          if (predicate(child)) found.push(child)
          walk(child)
        }
      }
    }
    walk(this)
    return found
  }
}

function fakeDocument() {
  const document = {
    activeElement: null,
    createElement: (tag) => new FakeElement(document, tag),
    createTextNode: (text) => ({ textContent: String(text) }),
  }
  document.body = new FakeElement(document, 'body')
  document.activeElement = document.body
  return document
}

test('the summary names a book by its ticker else its ISIN, and flags one the service could not rebuild', async () => {
  const { renderSummary } = await load('audit.js')
  const document = fakeDocument()
  const node = new FakeElement(document, 'div')
  document.body.append(node)
  const book = {
    transunix: '2026-08-14T12:46:59.999999999Z',
    ticker: 'HOLN',
    isincode: 'CH0012214059',
    crosscode: '3:0:CH0012214059',
    bestbid: '72.25',
    bestask: '72.35',
    bidqty: '300',
    askqty: '250',
    complete: true,
    alive: 2,
    delta: 1,
    events: 1,
    bidlimits: [{ price: '72.25', quantity: '300', uuids: ['a'], tradable: true }],
    asklimits: [{ price: '72.35', quantity: '250', uuids: ['b'], tradable: true }],
  }
  renderSummary(node, book)
  const [name] = node.findAll((element) => element.className === 'summary-name')
  assert.equal(name.textContent, 'HOLN · 3:0:CH0012214059')
  const labels = () => node.findAll((element) => element.className === 'stat-label').map((element) => element.textContent)
  // The summary counts the alive entries, the delta - the orders and quotes
  // the instant applied - and the events it recorded, an execution among them.
  assert.deepEqual(labels(), ['Best bid', 'Best ask', 'Spread', 'Mid', 'Imbalance', 'Alive', 'Delta', 'Events'])
  assert.doesNotMatch(node.textContent, /Delta book/)
  assert.match(node.textContent, /Alive2entries/)
  assert.match(node.textContent, /Delta1since the previous book/)
  assert.match(node.textContent, /Events1recorded at this instant/)

  // A book the service answers `complete: false` is a delta book: its touch
  // stands, and it is flagged, with no entry or limit counted; one stating no
  // ticker is named by its ISIN.
  renderSummary(node, { ...book, ticker: null, complete: false, alive: 0, bidlimits: [], asklimits: [] })
  assert.equal(node.findAll((element) => element.className === 'summary-name')[0].textContent, 'CH0012214059 · 3:0:CH0012214059')
  const flags = node.findAll((element) => element.className === 'chip chip-warn').map((element) => element.textContent)
  assert.deepEqual(flags, ['Delta book'])
  assert.match(node.textContent, /Best bid72\.25/)
  assert.match(node.textContent, /Alive–not rebuilt/)
  assert.equal(node.findAll((element) => element.className === 'empty').map((element) => element.textContent).join('|'), 'Not rebuilt: this is a delta book.|Not rebuilt: this is a delta book.')
})

test('sorting an audit column keeps the focus on its heading', async () => {
  const { renderEvents } = await load('audit.js')
  const document = fakeDocument()
  const node = new FakeElement(document, 'div')
  document.body.append(node)
  const rows = [{ price: '10', side: 'BUYS' }, { price: '9.5', side: 'BUYS' }]
  renderEvents(node, rows, 'bid')
  const heading = (column) => node.findAll((element) => element.tag === 'button' && element.dataset.column === column)[0]
  const before = heading('price')
  before.focus()
  before.dispatch('click')
  const after = heading('price')
  // Elements are compared by identity: a failing diff of a fake DOM would print the whole document.
  assert.ok(after !== before, 'the table was rendered again')
  assert.ok(document.activeElement === after, `the new heading holds the focus, not ${document.activeElement?.tag}`)
  assert.ok(after.within(node))
  assert.equal(node.dataset.sortColumn, 'price')
  // A render that is not a sort moves no focus.
  document.body.focus()
  renderEvents(node, rows, 'bid')
  assert.ok(document.activeElement === document.body)
})

/** The WCAG 2 contrast ratio of two `#rrggbb` colours. */
function contrast(foreground, background) {
  const luminance = ([r, g, b]) => {
    const channel = (value) => {
      const unit = value / 255
      return unit <= 0.03928 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4
    }
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
  }
  const [light, dark] = [luminance(foreground), luminance(background)].sort((a, b) => b - a)
  return (light + 0.05) / (dark + 0.05)
}

/** A colour token's value as `[r, g, b, a]`: `#rrggbb` or `rgba(r, g, b, a)`. */
function rgba(text) {
  const hex = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(text)
  if (hex) return [...hex.slice(1).map((part) => parseInt(part, 16)), 1]
  const functional = /^rgba?\(\s*(\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\s*\)$/.exec(text)
  assert.ok(functional, `a colour: ${text}`)
  return [Number(functional[1]), Number(functional[2]), Number(functional[3]), functional[4] === undefined ? 1 : Number(functional[4])]
}

test('the small text on a tinted ground reaches WCAG AA in both schemes', () => {
  const css = source('theme.css')
  const block = (opening) => {
    const open = css.indexOf('{', css.indexOf(opening))
    return Object.fromEntries([...css.slice(open, css.indexOf('}', open)).matchAll(/(--[a-z-]+):\s*([^;]+);/g)].map((match) => [match[1], match[2].trim()]))
  }
  const light = block(':root {')
  const dark = { ...light, ...block(':root[data-theme="dark"]') }
  // The error status, the Two-sided chip and the Locked/Crossed chip: each text on its soft ground over the panel.
  assert.match(css, /\.status\[data-kind="error"\]\s*\{[^}]*color:\s*var\(--danger\);[^}]*background:\s*var\(--danger-soft\);/)
  assert.match(css, /\.chip-ok\s*\{[^}]*color:\s*var\(--ok\);[^}]*background:\s*var\(--ok-soft\);/)
  assert.match(css, /\.chip-warn\s*\{[^}]*color:\s*var\(--warn\);[^}]*background:\s*var\(--warn-soft\);/)
  for (const [scheme, tokens] of [['light', light], ['dark', dark]]) {
    const [pr, pg, pb] = rgba(tokens['--panel'])
    for (const [text, ground] of [['--danger', '--danger-soft'], ['--ok', '--ok-soft'], ['--warn', '--warn-soft'], ['--muted', '--panel-raised']]) {
      const [r, g, b, a] = rgba(tokens[ground])
      const blended = [r * a + pr * (1 - a), g * a + pg * (1 - a), b * a + pb * (1 - a)]
      const ratio = contrast(rgba(tokens[text]), blended)
      assert.ok(ratio >= 4.5, `${scheme}: ${text} on ${ground} is ${ratio.toFixed(2)}:1`)
    }
  }
})

test('downloadLink points at the route and states the coding; the route names the file', async () => {
  const { downloadLink, AUDIT_TYPES, EVENT_COLUMNS } = await load('audit.js')
  const query = { table: 'books', ticker: 'ABBN.S', from: '2026-08-14T14:46:00Z', to: '2026-08-14T15:00:00Z', tz: 'UTC' }
  const csv = downloadLink('http://h:1/', query, '.csv')
  assert.equal(csv.href, 'http://h:1/api/audit.csv?table=books&ticker=ABBN.S&from=2026-08-14T14%3A46%3A00Z&to=2026-08-14T15%3A00%3A00Z&tz=UTC')
  // The service's `Content-Disposition` is the one owner of the file's name.
  assert.deepEqual(Object.keys(csv), ['href', 'label', 'type'])
  assert.equal(csv.label, 'CSV')
  assert.equal(csv.type, 'text/csv')
  assert.equal(downloadLink('http://h:1/', query, '.csv.gz').type, 'application/gzip')
  assert.equal(downloadLink('http://h:1/', query, '.csv.zst').label, 'CSV · zstd')
  assert.equal(downloadLink('http://h:1/', query, '.csv.zst').type, 'application/zstd')
  assert.throws(() => downloadLink('http://h:1/', query, '.csv.xz'), /audit suffix must be one of/)
  assert.deepEqual(Object.keys(AUDIT_TYPES), ['.csv', '.csv.gz', '.csv.zst'])
  assert.deepEqual(
    EVENT_COLUMNS.map((column) => column.key),
    ['transunix', 'role', 'marketdatakind', 'side', 'price', 'quantity', 'state', 'crosscode', 'uuid', 'prevuuid'],
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

  // Exactly the zones the service reads: the browser's first where it is one, then UTC, then the rest by name.
  assert.deepEqual(app.timezoneChoices(['UTC', 'Asia/Tokyo', 'Europe/Zurich', 'America/New_York'], 'Europe/Zurich'), ['Europe/Zurich', 'UTC', 'America/New_York', 'Asia/Tokyo'])
  assert.deepEqual(app.timezoneChoices(['UTC', 'Europe/Zurich'], 'Europe/Sofia'), ['UTC', 'Europe/Zurich'])
  assert.deepEqual(app.timezoneChoices(['Asia/Tokyo'], 'UTC'), ['Asia/Tokyo'])
  assert.deepEqual(app.timezoneChoices([], 'Europe/Zurich'), [])

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
  assert.deepEqual(book.serveArguments(), ['market', 'serve', '--bind', '127.0.0.1:0', '--path', '/'])
  assert.deepEqual(
    book.serveArguments({
      tables: ['books=/data/books', { name: 'ref', location: 's3://bucket/ref' }, { location: '/data/other' }],
      bind: '0.0.0.0:8080',
      path: '/book',
      capture: ['a.log', 'b.log'],
      args: ['--snapshot-millis', 250],
    }),
    ['market', 'serve', 'books=/data/books', 'ref=s3://bucket/ref', '/data/other', '--bind', '0.0.0.0:8080', '--path', '/book', '--capture', 'a.log', '--capture', 'b.log', '--snapshot-millis', '250'],
  )
  assert.deepEqual(book.serveArguments({ tables: 'books=/data/books', capture: 'a.log' }), ['market', 'serve', 'books=/data/books', '--bind', '127.0.0.1:0', '--path', '/', '--capture', 'a.log'])
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

test('serve resolves on the endpoint line, and rejects with the refusal printed, its stderr, or a line that is no URL', { skip: process.platform === 'win32' && 'a shell script stands in for the binary' }, async () => {
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

    // The command's own refusal: a `✗` line on stdout, after the warning report, and exit 1.
    const refusing = script(
      'refusing',
      [
        'echo "! 1 warning(s) while reading: what a file states that the reader could not keep as stated"',
        'echo "· a declaration the reader dropped"',
        'echo "✗ invalid record value at \$.capture: a capture needs a table to land in"',
        'echo "  a second line of the same refusal"',
        'exit 1',
      ].join('\n'),
    )
    await assert.rejects(book.serve({ bin: refusing, capture: ['x.log'] }), (error) => {
      assert.equal(error.message, '✗ invalid record value at $.capture: a capture needs a table to land in\n  a second line of the same refusal')
      return true
    })
    // The workflow form of the warning report is passed over the same way, and stderr follows the refusal.
    const annotated = script('annotated', 'echo "::warning title=fix reader::dropped"\necho "✗ refused" && echo "a note on stderr" >&2\nexit 1')
    await assert.rejects(book.serve({ bin: annotated }), (error) => {
      assert.equal(error.message, '✗ refused\na note on stderr')
      return true
    })
    // The argument parser's refusal is on stderr alone.
    const parser = script('parser', 'echo "error: invalid value \'0\' for \'--read-timeout <SECONDS>\'" >&2\nexit 2')
    await assert.rejects(book.serve({ bin: parser }), (error) => {
      assert.equal(error.message, "error: invalid value '0' for '--read-timeout <SECONDS>'")
      return true
    })

    const babbling = script('babbling', 'echo "hello"\nexec sleep 30')
    await assert.rejects(book.serve({ bin: babbling, tables: ['b=/tmp/b'] }), /expected the endpoint on the first line of yggdryl market serve, got "hello"/)

    const silent = script('silent', 'exit 0')
    await assert.rejects(book.serve({ bin: silent }), /yggdryl market serve exited with 0 before printing its endpoint$/)
  } finally {
    rmSync(folder, { recursive: true, force: true })
  }
})

/**
 * The `yggdryl` a Cargo build of `cli/` leaves in the workspace's target folder
 * (`cargo build -p yggdryl-cli`), or `YGGDRYL_BIN`; null when neither is there.
 */
function builtCommand() {
  if (process.env.YGGDRYL_BIN) return process.env.YGGDRYL_BIN
  const target = process.env.CARGO_TARGET_DIR ?? join(__dirname, '..', '..', 'target')
  const bin = join(target, 'debug', process.platform === 'win32' ? 'yggdryl.exe' : 'yggdryl')
  return existsSync(bin) ? bin : null
}

test('serve rejects with the refusal the command itself prints on stdout', { skip: builtCommand() === null && 'no yggdryl built: `cargo build -p yggdryl-cli` or YGGDRYL_BIN' }, async () => {
  const bin = builtCommand()
  // A capture with nothing to land in is refused before a port is taken.
  await assert.rejects(book.serve({ bin, capture: ['bridge.log'] }), (error) => {
    assert.equal(error.message, '✗ invalid record value at $.capture: a capture needs a table to land in: expected a TABLE beside --capture, got none')
    return true
  })
  // What the argument parser refuses it writes on stderr, and that is the
  // message: the crate's own reading of a timeout, which bounds it.
  await assert.rejects(book.serve({ bin, args: ['--read-timeout', '0'] }), (error) => {
    assert.match(error.message, /^error: invalid value '0' for '--read-timeout <SECONDS>': 0 is not above zero and at most 86400 seconds\n/)
    return true
  })
})

/** Whether the process `pid` is alive. */
function alive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/** Wait until `condition()` holds, polling, for at most `ms`. */
async function until(condition, ms = 5_000) {
  const deadline = Date.now() + ms
  while (!condition()) {
    if (Date.now() > deadline) return false
    await new Promise((done) => setTimeout(done, 20))
  }
  return true
}

test('serve is cancelled by its signal while it waits for the endpoint, and never outlives its parent', { skip: process.platform === 'win32' && 'a shell script stands in for the binary' }, async () => {
  const folder = mkdtempSync(join(tmpdir(), 'yggdryl-book-'))
  // A binary that binds nothing and prints nothing: what a stalled capture fold looks like from outside.
  const hang = join(folder, 'hang')
  const pidfile = join(folder, 'pid')
  writeFileSync(hang, `#!/bin/sh
echo "$$" > ${JSON.stringify(pidfile)}
echo "folding the capture" >&2
exec sleep 3600
`)
  chmodSync(hang, 0o755)
  const pid = () => Number(readFileSync(pidfile, 'utf8'))
  const started = () => {
    try {
      return readFileSync(pidfile, 'utf8').trim() !== ''
    } catch {
      return false
    }
  }
  try {
    const control = new AbortController()
    const serving = book.serve({ bin: hang, tables: ['books=/tmp/books'], signal: control.signal })
    assert.ok(await until(started), 'the binary started')
    control.abort()
    await assert.rejects(serving, (error) => {
      assert.equal(error.name, 'AbortError')
      assert.match(error.message, /^yggdryl market serve was cancelled before printing its endpoint: folding the capture$/)
      return true
    })
    assert.ok(await until(() => !alive(pid())), 'the cancelled child is ended')

    // A signal already aborted starts nothing that lives on.
    rmSync(pidfile, { force: true })
    await assert.rejects(book.serve({ bin: hang, signal: AbortSignal.abort() }), { name: 'AbortError' })

    // A parent that exits while it waits takes the child with it.
    rmSync(pidfile, { force: true })
    const parent = require('node:child_process').spawnSync(
      process.execPath,
      ['-e', `require(${JSON.stringify(join(__dirname, '..', 'book.js'))}).serve({ bin: ${JSON.stringify(hang)} }).catch(() => {}); setTimeout(() => process.exit(0), 500)`],
      { timeout: 10_000 },
    )
    assert.equal(parent.status, 0)
    assert.ok(started(), 'the orphan-to-be started')
    const orphan = pid()
    const ended = await until(() => !alive(orphan))
    if (!ended) process.kill(orphan, 'SIGKILL')
    assert.ok(ended, 'the child ends with its parent')
  } finally {
    rmSync(folder, { recursive: true, force: true })
  }
})

// ---------------------------------------------------------------------------
// The page: `start` over a document of fake elements and a fetch that answers
// the book service's routes the way `rust/src/graph/serve.rs` writes them.
// ---------------------------------------------------------------------------

/** An answer a route gives with a status other than 200. */
class Reply {
  constructor(status, body) {
    this.status = status
    this.body = body
  }
}

/** The one-minute HOLN candles of 12:46 to 12:49 UTC, spelled in `tz` as the service spells them. */
function serviceCandles(tz) {
  const spell = (ms) => {
    const iso = new Date(ms).toISOString().slice(0, 19)
    if (tz === 'Europe/Zurich') return `${new Date(ms + 7_200_000).toISOString().slice(0, 19)}.000000000+02:00[Europe/Zurich]`
    if (tz === 'America/New_York') return `${new Date(ms - 14_400_000).toISOString().slice(0, 19)}.000000000-04:00[America/New_York]`
    return `${iso}.000000000Z`
  }
  const first = Date.UTC(2026, 7, 14, 12, 46)
  return Array.from({ length: 3 }, (_, index) => ({
    start: spell(first + index * 60_000),
    end: spell(first + (index + 1) * 60_000),
    bid: { open: '72.2', high: '72.3', low: '72.1', close: '72.25' },
    ask: { open: '72.3', high: '72.4', low: '72.25', close: '72.35' },
    mid: { open: '72.25', high: '72.35', low: '72.175', close: '72.3' },
    spread: { open: '0.1', high: '0.15', low: '0.05', close: '0.1' },
    bidqty: '300',
    askqty: '250',
    books: 4,
  }))
}

const SERVICE = Object.freeze({
  'api/tables': [{ name: 'books', url: 'file:///books' }],
  'api/timezones': ['UTC', 'America/New_York', 'Europe/Zurich'],
  // One book per key, by key: a ticker-keyed one, an ISIN-keyed one stating
  // its ticker, and the one keyed by the ISIN that states none.
  'api/tickers': [
    { key: 'ABBN.S', ticker: 'ABBN.S', crosscode: '3:0:ABBN.S', from: '2026-08-14T12:46:39Z', to: '2026-08-14T21:59:47Z', books: 33 },
    { key: 'CH0012214059', ticker: 'HOLN', crosscode: '3:0:CH0012214059', from: '2026-08-14T12:46:39Z', to: '2026-08-14T12:49:40Z', books: 9 },
    { key: 'XX0000000000', ticker: null, crosscode: '3:0:XX0000000000', from: '2026-08-14T12:46:39Z', to: '2026-08-14T12:47:00Z', books: 2 },
  ],
  'api/candles': (request) => ({ table: request.params.table, ticker: request.params.ticker, candles: serviceCandles(request.params.tz) }),
  'api/book': (request) => {
    const listed = SERVICE['api/tickers'].find((entry) => entry.key === request.params.ticker)
    const isincode = /^[A-Z]{2}[0-9A-Z]{9}[0-9]$/.test(request.params.ticker) ? request.params.ticker : null
    return { transunix: request.params.at, ticker: listed?.ticker ?? null, isincode, crosscode: listed?.crosscode ?? null, bestbid: '72.25', bestask: '72.35', complete: true, alive: 2, delta: 1, events: 0, bidlimits: [], asklimits: [] }
  },
  'api/events': (request) => ({ rows: [{ transunix: request.params.from, role: 'alive', side: request.params.side === 'bid' ? 'BUYS' : 'SELL', price: '72.25', crosscode: `${request.params.ticker}-${request.params.side}` }], truncated: false }),
})

/**
 * `start` over a fake document at `hash`, the browser in `zone` (the process's
 * `TZ` until `close()`), the window's fetch answering `routes` over `SERVICE`:
 * a route is a value, a `Reply`, or a function of the request answering
 * either or a promise of either. Answers `{ document, window, byId, requests,
 * asked(route), idle(), close() }`.
 */
async function openPage({ hash = '', zone = 'UTC', routes = {} } = {}) {
  const { start } = await load('app.js')
  const document = fakeDocument()
  document.baseURI = 'http://h:1/'
  const listeners = {}
  const window = {
    location: { hash, pathname: '/', search: '' },
    history: {
      replaceState(_state, _title, url) {
        const at = url.indexOf('#')
        window.location.hash = at < 0 ? '' : url.slice(at)
      },
    },
    devicePixelRatio: 1,
    setTimeout: (fn) => setTimeout(fn, 0),
    clearTimeout: (handle) => clearTimeout(handle),
    getComputedStyle: () => ({ getPropertyValue: () => '' }),
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    fetch: (url, init) => fetch(url, init),
    addEventListener: (type, listener) => (listeners[type] ??= []).push(listener),
    dispatch: (type) => (listeners[type] ?? []).forEach((listener) => listener({ type })),
  }
  document.defaultView = window
  const context = new Proxy({ measureText: (text) => ({ width: String(text).length * 6 }), globalAlpha: 1 }, {
    get: (target, name) => (name in target ? target[name] : () => {}),
    set: (target, name, value) => ((target[name] = value), true),
  })
  const canvas = (element) =>
    Object.assign(element, {
      getContext: () => context,
      getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 320 }),
    })
  document.createElement = (tag) => (tag === 'canvas' ? canvas(new FakeElement(document, tag)) : new FakeElement(document, tag))
  const elements = new Map()
  const box = new FakeElement(document, 'div')
  document.getElementById = (id) => {
    if (!elements.has(id)) {
      const element = new FakeElement(document, id === 'chart' ? 'canvas' : 'div', id)
      if (id === 'chart') {
        canvas(element)
        box.append(element)
      } else {
        document.body.append(element)
      }
      elements.set(id, element)
    }
    return elements.get(id)
  }
  const requests = []
  const service = { ...SERVICE, ...routes }
  const fetch = (url, init = {}) => {
    const parsed = new URL(url)
    const request = { route: parsed.pathname.slice(1), params: Object.fromEntries(parsed.searchParams), signal: init.signal }
    requests.push(request)
    return new Promise((resolve, reject) => {
      const abort = () => reject(Object.assign(new Error('The operation was aborted.'), { name: 'AbortError' }))
      if (init.signal?.aborted) return abort()
      init.signal?.addEventListener('abort', abort, { once: true })
      const route = service[request.route]
      Promise.resolve(typeof route === 'function' ? route(request) : route ?? new Reply(404, { error: `no route ${request.route}` })).then((answer) => {
        const { status, body } = answer instanceof Reply ? answer : { status: 200, body: answer }
        resolve({ ok: status < 400, status, statusText: '', text: async () => JSON.stringify(body) })
      })
    })
  }
  const previousZone = process.env.TZ
  process.env.TZ = zone
  start(document)
  // Every answer the stub gives and every debounce the page waits: a few turns of the loop past a timer each.
  const idle = async () => {
    for (let turn = 0; turn < 12; turn += 1) {
      await new Promise((done) => setTimeout(done, 2))
      await new Promise((done) => setImmediate(done))
    }
  }
  await idle()
  return {
    document,
    window,
    byId: document.getElementById,
    requests,
    asked: (route) => requests.filter((request) => request.route === route),
    idle,
    close() {
      if (previousZone === undefined) delete process.env.TZ
      else process.env.TZ = previousZone
    },
  }
}

/** Hover the first bucket with the keyboard and select it, as a reader does. */
function selectFirstBucket(page) {
  const chart = page.byId('chart')
  chart.dispatch('keydown', { key: 'Home' })
  chart.dispatch('keydown', { key: 'Enter' })
}

test('the page offers the zones the service reads, the browser zone only where it is one', async () => {
  const sofia = await openPage({ zone: 'Europe/Sofia' })
  try {
    const offered = sofia.byId('tz').children.map((option) => option.value)
    assert.deepEqual(offered, ['UTC', 'America/New_York', 'Europe/Zurich'])
    assert.equal(sofia.byId('tz').value, 'UTC')
    assert.equal(sofia.asked('api/candles').at(-1).params.tz, 'UTC')
    assert.doesNotMatch(sofia.byId('status').textContent, /could not be read/)
  } finally {
    sofia.close()
  }
  const zurich = await openPage({ zone: 'Europe/Zurich' })
  try {
    assert.deepEqual(zurich.byId('tz').children.map((option) => option.value), ['Europe/Zurich', 'UTC', 'America/New_York'])
    assert.equal(zurich.asked('api/candles').at(-1).params.tz, 'Europe/Zurich')
  } finally {
    zurich.close()
  }
  // A service that cannot list its zones leaves UTC and the zone the view is in.
  const failing = await openPage({ zone: 'Europe/Zurich', routes: { 'api/timezones': new Reply(500, { error: 'boom' }) } })
  try {
    assert.deepEqual(failing.byId('tz').children.map((option) => option.value), ['Europe/Zurich', 'UTC'])
    assert.equal(failing.byId('tz').value, 'Europe/Zurich')
  } finally {
    failing.close()
  }
})

test('the page sends the range as instants: a ticker span, a minute input, a zone change', async () => {
  const page = await openPage({ zone: 'Europe/Zurich' })
  try {
    // The first ticker's span, as the service states it, and the inputs showing it in the zone.
    let candles = page.asked('api/candles').at(-1).params
    assert.deepEqual([candles.ticker, candles.from, candles.to, candles.tz], ['ABBN.S', '2026-08-14T12:46:39Z', '2026-08-14T21:59:47Z', 'Europe/Zurich'])
    assert.equal(page.byId('from').value, '2026-08-14T14:46:39')
    assert.equal(page.byId('to').value, '2026-08-14T23:59:47')
    // A datetime-local input at a whole minute drops its seconds; the page reads the wall clock in the zone.
    page.byId('from').value = '2026-08-14T14:50'
    page.byId('from').dispatch('change')
    await page.idle()
    candles = page.asked('api/candles').at(-1).params
    assert.equal(candles.from, '2026-08-14T12:50:00Z')
    assert.match(page.window.location.hash, /from=2026-08-14T12%3A50%3A00Z/)
    // A zone change keeps the instants and shows them again in the new zone.
    page.byId('tz').value = 'America/New_York'
    page.byId('tz').dispatch('change')
    await page.idle()
    candles = page.asked('api/candles').at(-1).params
    assert.deepEqual([candles.from, candles.to, candles.tz], ['2026-08-14T12:50:00Z', '2026-08-14T21:59:47Z', 'America/New_York'])
    assert.equal(page.byId('from').value, '2026-08-14T08:50:00')
    assert.equal(page.byId('to').value, '2026-08-14T17:59:47')
    // The downloads carry the same instants, and leave the file's name to the route.
    const anchors = page.byId('downloads').children
    assert.equal(anchors.length, 3)
    assert.match(anchors[0].href, /from=2026-08-14T12%3A50%3A00Z&to=2026-08-14T21%3A59%3A47Z/)
    assert.equal(anchors[0].getAttribute('download'), '')
  } finally {
    page.close()
  }
})

test('the book list is keyed by the book key the service lists and labelled by its ticker, else its key', async () => {
  const page = await openPage()
  try {
    const options = page.byId('ticker').children
    assert.deepEqual(options.map((option) => option.value), ['ABBN.S', 'CH0012214059', 'XX0000000000'])
    assert.deepEqual(options.map((option) => option.textContent), ['ABBN.S · 33 books', 'HOLN · 9 books', 'XX0000000000 · 2 books'])
    // Every query names the book by its key, under the service's `ticker` parameter.
    page.byId('ticker').value = 'CH0012214059'
    page.byId('ticker').dispatch('change')
    await page.idle()
    const candles = page.asked('api/candles').at(-1).params
    assert.deepEqual([candles.ticker, candles.from, candles.to], ['CH0012214059', '2026-08-14T12:46:39Z', '2026-08-14T12:49:40Z'])
    assert.equal(page.byId('chart-title').textContent, 'HOLN · 1m · UTC')
    assert.match(page.window.location.hash, /ticker=CH0012214059/)
    page.byId('ticker').value = 'XX0000000000'
    page.byId('ticker').dispatch('change')
    await page.idle()
    assert.equal(page.asked('api/candles').at(-1).params.ticker, 'XX0000000000')
    assert.equal(page.byId('chart-title').textContent, 'XX0000000000 · 1m · UTC')
  } finally {
    page.close()
  }
})

test('a selected bucket reads its last book strictly before its end, in every zone', async () => {
  // A link naming the ticker of one listed book reads that book by its key,
  // as the service reads its `ticker` parameter.
  const page = await openPage({ zone: 'Europe/Zurich', hash: '#table=books&ticker=HOLN' })
  try {
    assert.equal(page.asked('api/candles').at(-1).params.ticker, 'CH0012214059')
    selectFirstBucket(page)
    await page.idle()
    const book = page.asked('api/book').at(-1).params
    assert.equal(book.at, '2026-08-14T12:46:59.999999999Z')
    const events = page.asked('api/events').map((request) => request.params)
    assert.deepEqual(events.map((params) => [params.side, params.from, params.to]), [
      ['bid', '2026-08-14T12:46:00Z', '2026-08-14T12:47:00Z'],
      ['ask', '2026-08-14T12:46:00Z', '2026-08-14T12:47:00Z'],
    ])
    assert.match(page.window.location.hash, /at=2026-08-14T12%3A46%3A00Z/)
    assert.match(page.byId('summary').textContent, /HOLN/)
  } finally {
    page.close()
  }
})

test('a shared link opens on its selected bucket', async () => {
  const hash = '#table=books&ticker=CH0012214059&from=2026-08-14T12%3A45%3A00Z&to=2026-08-14T13%3A15%3A00Z&tz=UTC&interval=1m&at=2026-08-14T12%3A47%3A00Z'
  const page = await openPage({ hash })
  try {
    assert.equal(page.asked('api/book').length, 1, 'the book of the linked bucket is read')
    assert.equal(page.asked('api/book')[0].params.at, '2026-08-14T12:47:59.999999999Z')
    assert.equal(page.asked('api/events').length, 2)
    assert.equal(page.window.location.hash, hash, 'the link keeps its bucket')
    assert.match(page.byId('summary').textContent, /HOLN/)
    // A naive wall clock in a hand-written link is read in the link's zone.
    const naive = await openPage({ hash: '#table=books&ticker=HOLN&from=2026-08-14T14:45:00&to=2026-08-14T15:15:00&tz=Europe/Zurich' })
    try {
      const candles = naive.asked('api/candles').at(-1).params
      assert.deepEqual([candles.from, candles.to], ['2026-08-14T12:45:00Z', '2026-08-14T13:15:00Z'])
    } finally {
      naive.close()
    }
  } finally {
    page.close()
  }
})

test('a bucket read that lands after the view moved on is dropped', async () => {
  const held = []
  const hold = (answer) => (request) => (request.params.ticker === 'CH0012214059' ? new Promise((done) => held.push(() => done(answer(request)))) : answer(request))
  const page = await openPage({ hash: '#table=books&ticker=HOLN', routes: { 'api/book': hold(SERVICE['api/book']), 'api/events': hold(SERVICE['api/events']) } })
  try {
    selectFirstBucket(page)
    await page.idle()
    assert.equal(held.length, 3, 'the book and both sides are in flight')
    page.byId('ticker').value = 'ABBN.S'
    page.byId('ticker').dispatch('change')
    await page.idle()
    held.forEach((release) => release())
    await page.idle()
    assert.equal(page.asked('api/candles').at(-1).params.ticker, 'ABBN.S')
    assert.doesNotMatch(page.byId('summary').textContent, /HOLN|CH0012214059/)
    assert.doesNotMatch(page.byId('bid-events').textContent, /HOLN|CH0012214059/)
    assert.doesNotMatch(page.window.location.hash, /at=/)
    assert.equal(page.byId('readout').textContent, '', 'the live region holds no reading of the view before')
  } finally {
    page.close()
  }
})

test('the skip link moves the focus without touching the hash, and a pasted view link is followed', async () => {
  const page = await openPage({ hash: '#table=books&ticker=HOLN&tz=UTC' })
  try {
    const hash = page.window.location.hash
    const click = page.byId('skip').dispatch('click')
    assert.equal(click.defaultPrevented, true)
    assert.ok(page.document.activeElement === page.byId('chart'), `the chart holds the focus, not ${page.document.activeElement?.id}`)
    assert.equal(page.window.location.hash, hash)
    // Another view pasted into the same tab: the page says it is loading it, then shows it.
    page.window.location.hash = '#table=books&ticker=ABBN.S&tz=UTC&interval=5m'
    page.window.dispatch('hashchange')
    assert.match(page.byId('status').textContent, /Loading/)
    await page.idle()
    const candles = page.asked('api/candles').at(-1).params
    assert.deepEqual([candles.ticker, candles.interval], ['ABBN.S', '5m'])
    assert.equal(page.byId('interval').value, '5m')
    // A fragment naming no view, such as a bare `#chart`, is not a view: the page writes its own back.
    page.window.location.hash = '#chart'
    page.window.dispatch('hashchange')
    await page.idle()
    assert.match(page.window.location.hash, /ticker=ABBN\.S/)
  } finally {
    page.close()
  }
})
