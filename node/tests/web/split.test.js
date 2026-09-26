// `node/web/split.js`: resizable panes with keyboard-operable separators.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

test('separators resize their panes from the keyboard and the pointer', async () => {
  const { page, close } = await openPage(`
import { Split } from '/web/split.js'
window.Split = Split
const root = document.getElementById('root')
root.style.width = '906px'
window.split = new Split({ direction: 'horizontal', sizes: [30, 70], label: 'ladders and charts' }).mount(root)
split.el.style.height = '200px'
split.panes[0].textContent = 'ladders'
split.panes[1].textContent = 'charts'
window.resized = []
document.addEventListener('ygg:resize', (event) => resized.push(event.detail.sizes))
window.widths = () => split.panes.map((pane) => Math.round(pane.getBoundingClientRect().width))
window.separator = () => split.el.querySelector('[role=separator]')
`)
  try {
    const described = await page.evaluate("[separator().getAttribute('aria-valuenow'), separator().getAttribute('aria-valuemin'), separator().getAttribute('aria-valuemax'), separator().getAttribute('aria-orientation'), separator().getAttribute('aria-controls') === split.panes[0].id, separator().getAttribute('aria-label')]")
    assert.deepEqual(described, ['30', '10', '90', 'vertical', true, 'Resize ladders and charts'])
    const [left, right] = await page.evaluate('widths()')
    assert.ok(Math.abs(left / (left + right) - 0.3) < 0.01, `${left} / ${right}`)
    await page.evaluate('separator().focus()')
    await page.press('ArrowRight')
    await page.press('ArrowRight')
    await page.frames()
    assert.equal(await page.evaluate("separator().getAttribute('aria-valuenow')"), '34')
    assert.deepEqual(await page.evaluate('resized.at(-1)'), [34, 66])
    const [grown] = await page.evaluate('widths()')
    assert.ok(grown > left, `${grown} > ${left}`)
    await page.press('Home')
    await page.frames()
    assert.equal(await page.evaluate("separator().getAttribute('aria-valuenow')"), '10')
    await page.press('End')
    await page.frames()
    assert.equal(await page.evaluate("separator().getAttribute('aria-valuenow')"), '90')
    await page.press('ArrowLeft')
    await page.frames()
    assert.equal(await page.evaluate("separator().getAttribute('aria-valuenow')"), '88')
    // The pointer drags the separator: 90 px of 900 is ten points.
    await page.evaluate("split.update({ sizes: [50, 50] })")
    await page.frames()
    const start = await page.centre('[role=separator]')
    await page.mouse('mouseMoved', start.x, start.y)
    await page.mouse('mousePressed', start.x, start.y)
    await page.mouse('mouseMoved', start.x + 45, start.y)
    await page.mouse('mouseMoved', start.x + 90, start.y)
    await page.mouse('mouseReleased', start.x + 90, start.y)
    await page.frames()
    const dragged = Number(await page.evaluate("separator().getAttribute('aria-valuenow')"))
    assert.ok(Math.abs(dragged - 60) <= 1, `${dragged}`)
    await page.screenshot('split')
    // Three stacked panes have two separators; a stated direction is one of two.
    await page.evaluate("window.three = new Split({ direction: 'vertical', sizes: [1, 1, 2] }).mount(document.getElementById('root'))")
    await page.frames()
    assert.deepEqual(await page.evaluate("[...three.el.querySelectorAll('[role=separator]')].map((node) => [node.getAttribute('aria-orientation'), node.getAttribute('aria-valuenow')])"), [['horizontal', '25'], ['horizontal', '25']])
    assert.match(await page.evaluate("(() => { try { new Split({ direction: 'diagonal' }); return 'accepted' } catch (error) { return error.message } })()"), /expected direction 'horizontal' or 'vertical'/)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
