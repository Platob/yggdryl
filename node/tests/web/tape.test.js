// `node/web/tape.js`: executions newest first as served, virtualized, every
// nanosecond of the instant kept, a selection naming the element.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { Tape } from '/web/tape.js'
const { executions } = await (await fetch('/fixtures/executions.json')).json()
window.executions = executions
window.selected = []
document.addEventListener('ygg:select-element', (event) => window.selected.push(event.detail))
const root = document.getElementById('root')
root.style.width = '560px'
window.tape = new Tape({ rows: 10 }).mount(root)
tape.update({ executions })
window.rowsOf = () => [...tape.el.querySelectorAll('[role=row]:not([aria-rowindex="1"])')].map((row) =>
  [...row.querySelectorAll('[role=gridcell]')].map((cell) => cell.textContent))
`

test('the tape shows the served executions in served order, one nanosecond apart kept distinct', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.deepEqual(await page.evaluate('rowsOf()'), [
      ['22:13:23.000000001', '101.25', '0.5', 'SELL', '0192f0a0'],
      ['22:13:23.000000000', '101', '1.5', 'BUY', '0192f0a0'],
      ['22:13:21.500000000', '100.75', '2', 'SELL', '0192f0a0'],
      ['22:13:21.000000001', '100.5', '1', 'BUY', '0192f0a0'],
    ])
    const stamps = await page.evaluate("[...tape.el.querySelectorAll('time')].map((node) => node.getAttribute('datetime'))")
    assert.deepEqual(stamps.slice(0, 2), ['2023-11-14T22:13:23.000000001Z', '2023-11-14T22:13:23.000000000Z'])
    assert.equal(new Set(stamps).size, 4)
    const summary = await page.evaluate("(() => { const node = tape.el.querySelector('[aria-live]'); return [node.getAttribute('aria-live'), node.textContent] })()")
    assert.deepEqual(summary, ['polite', 'Last print 101.25 x 0.5, SELL, at 22:13:23.000000001'])
    assert.deepEqual(await page.evaluate("[...tape.el.querySelectorAll('[role=row] [role=gridcell]:nth-of-type(4)')].map((cell) => cell.className.includes('ygg-ui__ask') ? 'ask' : cell.className.includes('ygg-ui__bid') ? 'bid' : '')"), ['ask', 'bid', 'ask', 'bid'])
    await page.screenshot('tape')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a click or Enter names the element; a newer print patches in place', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.click('.ygg-ui__tape [aria-rowindex="3"]')
    assert.deepEqual(await page.evaluate('selected'), [{ crosscode: 'ALPHA-X-20', curruuid: '0192f0a0-0000-7000-8000-e00000000020' }])
    await page.evaluate("tape.el.querySelector('[role=grid]').focus()")
    // The keyboard continues from the clicked row.
    await page.press('ArrowDown')
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('selected.at(-1)'), { crosscode: 'ALPHA-X-11', curruuid: '0192f0a0-0000-7000-8000-e00000000011' })
    await page.evaluate(`(() => {
      tape.el.querySelector('[aria-rowindex="2"]').marker = 'kept'
      const newer = { ...executions[0], curruuid: '0192f0a0-0000-7000-8000-e00000000099', crosscode: 'ALPHA-X-99', currunix: '1700000004000000000', lastpx: '102' }
      tape.update({ executions: [newer, ...executions] })
    })()`)
    await page.frames()
    assert.equal(await page.evaluate("tape.el.querySelector('[aria-rowindex=\"3\"]').marker"), 'kept', 'the older print moved down, not rebuilt')
    assert.equal(await page.evaluate("tape.el.querySelector('[aria-live]').textContent"), 'Last print 102 x 0.5, SELL, at 22:13:24.000000000')
    await page.evaluate(`tape.update({ executions: Array.from({ length: 5000 }, (_, at) => ({ ...executions[0], curruuid: 'u-' + at, currunix: String(1700000004000000000n - BigInt(at)) })) })`)
    await page.frames()
    const rendered = await page.evaluate("tape.el.querySelectorAll('[role=row]').length")
    assert.ok(rendered < 200, `${rendered} rows in the DOM`)
    assert.equal(await page.evaluate("tape.el.querySelector('[role=grid]').getAttribute('aria-rowcount')"), '5001')
    await page.evaluate("tape.update({ executions: [] })")
    await page.frames()
    assert.equal(await page.evaluate("tape.el.querySelector('[aria-live]').textContent"), 'No executions')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
