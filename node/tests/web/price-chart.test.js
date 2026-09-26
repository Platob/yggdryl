// `node/web/price-chart.js`: the midpoint line, candles and execution markers
// over bigint instants; a crosshair that seeks.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const T0 = 1_700_000_000_000_000_000n
const at = (ns) => (T0 + BigInt(ns)).toString()

const PAGE = `
import { PriceChart } from '/web/price-chart.js'
const books = await (await fetch('/fixtures/books.json')).json()
window.books = books
window.seeks = []
document.addEventListener('ygg:seek', (event) => window.seeks.push(event.detail))
const root = document.getElementById('root')
root.style.width = '800px'
window.chart = new PriceChart({ bucketNs: 500_000_000n, height: 260 }).mount(root)
chart.update({ books, window: { from: books[0].currunix, to: books.at(-1).currunix } })
`

test('the chart reads the served midpoints and executions into a line, candles and markers', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.deepEqual(await page.evaluate('chart.series.midpoint'), [
      [at(1_000_000_000), '100.5'],
      [at(1_000_000_001), '100.5'],
      [at(1_500_000_000), '100.75'],
      [at(2_500_000_000), '100.75'],
      [at(3_000_000_000), '101'],
    ])
    // A last price is a served fact or nothing: the book row states none, and a
    // book's executions are ordered by uuid, so its last one is no latest print.
    assert.deepEqual(await page.evaluate('books.map((book) => book.lastpx)'), [null, null, null, null, null])
    assert.deepEqual(await page.evaluate('Object.keys(chart.series)'), ['midpoint', 'executions'])
    assert.equal(await page.evaluate('chart.series.executions.length'), 4)
    const candles = await page.evaluate('chart.candles.map((c) => [String(c.open), c.first, c.high, c.low, c.last, c.count])')
    assert.deepEqual(candles, [
      [at(1_000_000_000), '100.5', '100.5', '100.5', '100.5', 1],
      [at(1_500_000_000), '100.75', '100.75', '100.75', '100.75', 1],
      [at(3_000_000_000), '101', '101', '100.25', '100.25', 2],
    ])
    const label = await page.evaluate("chart.el.querySelector('canvas').getAttribute('aria-label')")
    assert.equal(
      label,
      'Prices from 2023-11-14T22:13:21.000000000Z to 2023-11-14T22:13:23.000000000Z: 5 books, 4 executions, last midpoint 101',
    )
    await page.screenshot('price-chart')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('the crosshair follows the pointer and the arrows, and seeks with a bigint instant as text', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.hover('.ygg-ui__price canvas')
    await page.frames()
    const hovered = await page.evaluate('chart.cross')
    assert.match(hovered, /^\d+$/)
    assert.ok(BigInt(hovered) > T0 + 1_000_000_000n && BigInt(hovered) < T0 + 3_000_000_000n)
    await page.click('.ygg-ui__price canvas')
    assert.equal((await page.evaluate('seeks')).length, 1)
    assert.match((await page.evaluate('seeks[0].at')), /^\d+$/)
    await page.screenshot('price-chart-crosshair')
    await page.evaluate("chart.el.querySelector('[role=slider]').focus()")
    await page.press('Home')
    await page.frames()
    assert.equal(await page.evaluate('chart.cross'), at(1_000_000_000))
    await page.press('ArrowRight')
    await page.press('ArrowRight')
    await page.frames()
    assert.equal(await page.evaluate('chart.cross'), at(2_000_000_000), 'two buckets of half a second')
    const slider = await page.evaluate("(() => { const node = chart.el.querySelector('[role=slider]'); return [node.getAttribute('aria-valuenow'), node.getAttribute('aria-valuetext')] })()")
    assert.deepEqual(slider, ['2', '2023-11-14T22:13:22.000000000Z, midpoint 100.75'])
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('seeks.at(-1)'), { at: at(2_000_000_000) })
    await page.press('End')
    await page.frames()
    assert.equal(await page.evaluate('chart.cross'), at(3_000_000_000))
    // The arrows the chart handles never reach the page's own shortcuts.
    const escaped = await page.evaluate(`(() => {
      let reached = 0
      document.addEventListener('keydown', () => { reached += 1 }, { once: true })
      chart.el.querySelector('[role=slider]').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }))
      return reached
    })()`)
    assert.equal(escaped, 0)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
