// `node/web/depth-chart.js`: cumulative depth over the served limits, the step
// sums exact decimal text, drawn once per frame at the device pixel ratio.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { DepthChart, cumulative } from '/web/depth-chart.js'
window.cumulative = cumulative
const book = await (await fetch('/fixtures/book.json')).json()
window.book = book
const root = document.getElementById('root')
root.style.width = '640px'
window.chart = new DepthChart({ height: 240 }).mount(root)
window.draws = 0
const draw = chart.draw.bind(chart)
chart.draw = (state) => { window.draws += 1; draw(state) }
chart.update({ bid: book.bid, ask: book.ask })
`

test('the step sums are the exact text sums along the served order', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.deepEqual(await page.evaluate('chart.steps.bid.map((step) => [step.price, step.total])'), [
      ['100.5', '8'],
      ['100.25', '11'],
      ['100', '23.5'],
      [null, '28.5'],
    ])
    assert.deepEqual(await page.evaluate('chart.steps.ask.map((step) => step.total)'), ['4', '4.1', '4.3'])
    // A float would answer 0.30000000000000004 and 0.6000000000000001.
    assert.deepEqual(
      await page.evaluate("cumulative([{ price: '1', quantity: '0.1' }, { price: '2', quantity: '0.2' }, { price: '3', quantity: '0.3' }]).map((step) => step.total)"),
      ['0.1', '0.3', '0.6'],
    )
    assert.deepEqual(
      await page.evaluate("cumulative([{ price: '1', quantity: '9007199254740993' }, { price: '2', quantity: '1' }]).map((step) => step.total)"),
      ['9007199254740993', '9007199254740994'],
    )
    const label = await page.evaluate("chart.el.querySelector('canvas').getAttribute('aria-label')")
    assert.equal(label, 'Depth: best bid 100.5 x 8, best ask 101 x 4; bid depth 28.5 over 4 limits, ask depth 4.3 over 3 limits')
    assert.equal(await page.evaluate("chart.el.querySelector('canvas').getAttribute('role')"), 'img')
    // Something was drawn in the bid colour and in the ask colour.
    const painted = await page.evaluate(`(() => {
      const canvas = chart.el.querySelector('canvas')
      const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data
      let bid = 0, ask = 0
      for (let at = 0; at < data.length; at += 4) {
        if (data[at] === 255 && data[at + 1] === 138 && data[at + 2] === 0) bid += 1
        if (data[at] === 255 && data[at + 1] === 94 && data[at + 2] === 91) ask += 1
      }
      return { bid, ask }
    })()`)
    assert.ok(painted.bid > 0 && painted.ask > 0, JSON.stringify(painted))
    await page.screenshot('depth-chart')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('many updates in one frame draw once, and the canvas follows its box and the pixel ratio', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    const before = await page.evaluate('draws')
    await page.evaluate('for (let at = 0; at < 5; at += 1) chart.update({ bid: book.bid, ask: book.ask })')
    await page.frames(3)
    assert.equal(await page.evaluate('draws'), before + 1)
    assert.equal(await page.evaluate("chart.el.querySelector('canvas').width === chart.el.querySelector('canvas').clientWidth"), true)
    await page.evaluate("document.getElementById('root').style.width = '400px'")
    await page.frames(3)
    assert.equal(await page.evaluate("chart.el.querySelector('canvas').width === chart.el.querySelector('canvas').clientWidth"), true)
    await page.viewport({ deviceScaleFactor: 2 })
    await page.evaluate("document.getElementById('root').style.width = '420px'")
    await page.frames(3)
    const sized = await page.evaluate("(() => { const canvas = chart.el.querySelector('canvas'); return [canvas.width, canvas.clientWidth, canvas.height, canvas.clientHeight] })()")
    assert.equal(sized[0], sized[1] * 2)
    assert.equal(sized[2], sized[3] * 2)
    // An empty book is a sentence too, never a failure.
    await page.evaluate("chart.update({ bid: { limits: [] }, ask: { limits: [] } })")
    await page.frames()
    assert.equal(await page.evaluate("chart.el.querySelector('canvas').getAttribute('aria-label')"), 'Depth: no limits on either side')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
