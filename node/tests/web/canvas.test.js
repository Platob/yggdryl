// `node/web/canvas.js`: the canvas sizing, tokens and redraw signals the charts share.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { tickLabel } from '../../web/canvas.js'
import { openPage } from './browser.js'

test('a tick label is the float at twelve significant digits', () => {
  assert.equal(tickLabel(0.1 + 0.2), '0.3')
  assert.equal(tickLabel(100), '100')
  assert.equal(tickLabel(99.60000000000001), '99.6')
})

test('a canvas is sized at the pixel ratio, reads the theme tokens and redraws on theme change', async () => {
  const { page, close } = await openPage(`
import { fitCanvas, tokens, watchTheme } from '/web/canvas.js'
const canvas = document.createElement('canvas')
canvas.style.cssText = 'display:block;width:300px;height:100px'
document.getElementById('root').append(canvas)
window.canvas = canvas
window.fit = () => { const { width, height, ratio } = fitCanvas(canvas); return [width, height, ratio, canvas.width, canvas.height] }
window.read = () => tokens(document.body, ['bid', 'page'])
window.changes = 0
window.stop = watchTheme(() => { window.changes += 1 })
`)
  try {
    assert.deepEqual(await page.evaluate('fit()'), [300, 100, 1, 300, 100])
    await page.viewport({ deviceScaleFactor: 2 })
    assert.deepEqual(await page.evaluate('fit()'), [300, 100, 2, 600, 200])
    assert.deepEqual(await page.evaluate('read()'), { bid: '#ff8a00', page: '#000000' })
    await page.evaluate("document.documentElement.dataset.theme = 'light'")
    await page.waitFor('changes === 1')
    assert.deepEqual(await page.evaluate('read()'), { bid: '#ff8a00', page: '#ffffff' })
    await page.evaluate("stop(); document.documentElement.dataset.theme = 'dark'")
    await page.frames()
    assert.equal(await page.evaluate('changes'), 1, 'a stopped watch hears nothing')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a pixel ratio change alone asks for a redraw, and a stopped watch asks for none', async () => {
  const { page, close } = await openPage(`
import { fitCanvas, watchSize } from '/web/canvas.js'
const canvas = document.createElement('canvas')
canvas.style.cssText = 'display:block;width:300px;height:100px'
document.getElementById('root').append(canvas)
window.redraws = []
window.stop = watchSize(canvas, () => { const { ratio } = fitCanvas(canvas); redraws.push([ratio, canvas.width]) })
`)
  try {
    await page.waitFor('redraws.length === 1')
    assert.deepEqual(await page.evaluate('redraws'), [[1, 300]])
    // The box keeps its CSS size; only the device pixels under it change.
    await page.viewport({ deviceScaleFactor: 2 })
    await page.waitFor('redraws.length === 2')
    assert.deepEqual(await page.evaluate('redraws.at(-1)'), [2, 600], 'the bitmap is refitted at the new ratio')
    // Re-armed: the next change is heard too. DevTools emulation re-evaluates a
    // media query after its first ratio change only when the viewport changes
    // as well, so the page narrows; the canvas keeps its 300px box.
    await page.viewport({ deviceScaleFactor: 3, width: 1270 })
    await page.waitFor('redraws.length === 3')
    assert.deepEqual(await page.evaluate('redraws.at(-1)'), [3, 900])
    await page.evaluate('stop()')
    await page.viewport({ deviceScaleFactor: 1, width: 1260 })
    await page.frames(4)
    assert.equal(await page.evaluate('devicePixelRatio'), 1)
    assert.equal(await page.evaluate('redraws.length'), 3, 'a stopped watch hears nothing')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
