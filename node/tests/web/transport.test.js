// `node/web/transport.js`: every control emits its command id; the scrubber
// is the book index and its label the full instant, one nanosecond apart kept.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const T0 = 1_700_000_000_000_000_000n
const at = (ns) => (T0 + BigInt(ns)).toString()

const PAGE = `
import { Transport } from '/web/transport.js'
window.intents = []
document.addEventListener('ygg:transport', (event) => window.intents.push(event.detail))
window.transport = new Transport().mount(document.getElementById('root'))
window.base = { index: 1, count: 5, at: '${at(1_000_000_001)}', first: '${at(1_000_000_000)}', last: '${at(3_000_000_000)}', playing: false, speed: 1, grid: false, isTick: false }
transport.update(base)
window.q = (selector) => transport.el.querySelector(selector)
`

test('the scrubber is the index and its label every digit of the instant', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    const range = await page.evaluate("[q('input[type=range]').min, q('input[type=range]').max, q('input[type=range]').value, q('input[type=range]').getAttribute('aria-valuetext')]")
    assert.deepEqual(range, ['0', '4', '1', '2023-11-14T22:13:21.000000001Z'])
    assert.equal(await page.evaluate("q('.ygg-ui__transport-at').textContent"), '2023-11-14T22:13:21.000000001Z')
    assert.equal(await page.evaluate("q('.ygg-ui__transport-position').textContent"), '2 of 5')
    await page.evaluate("transport.update({ ...base, index: 0, at: base.first })")
    await page.frames()
    assert.equal(await page.evaluate("q('.ygg-ui__transport-at').textContent"), '2023-11-14T22:13:21.000000000Z', 'one nanosecond earlier reads as another instant')
    assert.equal(await page.evaluate("q('[data-command=\"transport.back\"]').getAttribute('aria-disabled')"), 'true')
    await page.click('[data-command="transport.back"]')
    assert.deepEqual(await page.evaluate('intents'), [], 'a disabled step emits nothing')
    // Every control carries a name and is reachable by Tab.
    const unnamed = await page.evaluate("[...transport.el.querySelectorAll('button, input, select')].filter((node) => !(node.getAttribute('aria-label') || node.labels?.length)).length")
    assert.equal(unnamed, 0)
    await page.screenshot('transport')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('each control emits the shortcut command ids and values', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    for (const command of ['transport.first', 'transport.backSource', 'transport.back', 'transport.toggle', 'transport.forward', 'transport.forwardSource', 'transport.last', 'transport.grid']) {
      await page.click(`[data-command="${command}"]`)
    }
    assert.deepEqual(await page.evaluate('intents'), [
      { command: 'transport.first' },
      { command: 'transport.backSource' },
      { command: 'transport.back' },
      { command: 'transport.toggle', value: true },
      { command: 'transport.forward' },
      { command: 'transport.forwardSource' },
      { command: 'transport.last' },
      { command: 'transport.grid', value: true },
    ])
    await page.evaluate("intents.length = 0; q('input[type=range]').focus()")
    await page.press('ArrowRight')
    assert.deepEqual(await page.evaluate('intents'), [{ command: 'transport.seek', value: 2 }])
    await page.evaluate("intents.length = 0; const select = q('select'); select.value = '2'; select.dispatchEvent(new Event('change', { bubbles: true }))")
    assert.deepEqual(await page.evaluate('intents'), [{ command: 'transport.speed', value: 2 }])
    assert.deepEqual(await page.evaluate("[...q('select').options].map((option) => option.value)"), ['0.25', '0.5', '1', '2', '4', '8', '16'])
    // The jump field refuses what is not an instant, naming it, and emits the text it read.
    await page.evaluate("intents.length = 0; transport.focusJump()")
    await page.type('12:00')
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('intents'), [])
    assert.equal(await page.evaluate("q('.ygg-ui__transport-jump input').getAttribute('aria-invalid')"), 'true')
    assert.match(await page.evaluate("q('.ygg-ui__transport-error').textContent"), /expected an instant as decimal nanoseconds/)
    await page.evaluate("q('.ygg-ui__transport-jump input').value = ''")
    await page.type('1700000002000000000')
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('intents'), [{ command: 'transport.jump', value: '1700000002000000000' }])
    assert.equal(await page.evaluate("q('.ygg-ui__transport-jump input').getAttribute('aria-invalid')"), 'false')
    // The state reads back: playing, the grid, a tick.
    await page.evaluate("transport.update({ ...base, playing: true, grid: true, isTick: true, speed: 4 })")
    await page.frames()
    const shown = await page.evaluate("[q('[data-command=\"transport.toggle\"]').getAttribute('aria-label'), q('[data-command=\"transport.toggle\"]').getAttribute('aria-pressed'), q('[data-command=\"transport.grid\"]').getAttribute('aria-pressed'), q('.ygg-ui__transport-tick').hidden, q('select').value]")
    assert.deepEqual(shown, ['Pause', 'true', 'true', false, '4'])
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
