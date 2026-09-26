// `node/web/ladder.js`: the served `limits` in served order, virtualized,
// patched in place, keyboard reachable; and the theme it is drawn in.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { Ladder } from '/web/ladder.js'
const book = await (await fetch('/fixtures/book.json')).json()
window.book = book
window.picked = []
document.addEventListener('ygg:limit-select', (event) => window.picked.push(event.detail))
const root = document.getElementById('root')
root.style.display = 'grid'
root.style.gridTemplateColumns = '1fr 1fr'
root.style.gap = '8px'
window.bid = new Ladder({ side: 'bid', rows: 12 }).mount(root)
window.ask = new Ladder({ side: 'ask', rows: 12 }).mount(root)
window.bid.update({ side: book.bid, symbol: 'ALPHA', global: false })
window.ask.update({ side: book.ask, symbol: 'ALPHA', global: false })
window.rowsOf = (ladder) => [...ladder.el.querySelectorAll('[role=row][aria-rowindex]:not([aria-rowindex="1"])')].map((row) =>
  [...row.querySelectorAll('[role=gridcell]')].map((cell) => cell.textContent))
`

test('the ladder renders the served limits in served order, the unpriced limit last', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    // Bid columns read orders, quantity, price (the price beside the spread); ask price first.
    assert.deepEqual(await page.evaluate('rowsOf(bid)'), [
      ['2', '8', '100.5'],
      ['1', '3', '100.25'],
      ['1', '12.5', '100'],
      ['1', '5', '∅'],
    ])
    assert.deepEqual(await page.evaluate('rowsOf(ask)'), [
      ['101', '4', '1'],
      ['101.5', '0.1', '1'],
      ['102', '0.2', '2'],
    ])
    assert.equal(await page.evaluate("bid.el.querySelector('[aria-rowindex=\"5\"]').classList.contains('ygg-ui__unpriced')"), true)
    assert.equal(await page.evaluate("bid.el.querySelectorAll('.ygg-ui__unpriced').length"), 1)
    assert.equal(await page.evaluate("bid.el.querySelector('[role=grid]').getAttribute('aria-rowcount')"), '5')
    assert.equal(await page.evaluate("bid.el.querySelector('[role=grid]').getAttribute('aria-label')"), 'Bid limits, ALPHA')
    const summary = await page.evaluate("(() => { const node = bid.el.querySelector('[aria-live]'); return [node.getAttribute('aria-live'), node.textContent] })()")
    assert.deepEqual(summary, ['polite', 'Bid: best 100.5 x 8, 4 limits'])
    assert.equal(await page.evaluate("ask.el.querySelector('[aria-live]').textContent"), 'Ask: best 101 x 4, 3 limits')
    // The side is carried by a glyph class, not by colour alone.
    assert.equal(await page.evaluate("bid.el.querySelectorAll('.ygg-ui__bid').length"), 4)
    assert.equal(await page.evaluate("ask.el.querySelectorAll('.ygg-ui__ask').length"), 3)
    await page.screenshot('ladder-dark')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('an update patches rows in place, keyed by price text', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.evaluate(`(() => {
      bid.el.querySelector('[aria-rowindex="2"]').marker = 'kept'
      const side = structuredClone(book.bid)
      side.limits = [side.limits[1], side.limits[0], { ...side.limits[2], quantity: '13' }, side.limits[3]]
      side.price = '100.25'
      side.quantity = '3'
      bid.update({ side, symbol: 'ALPHA', global: false })
    })()`)
    await page.frames()
    assert.deepEqual(await page.evaluate('rowsOf(bid).map((row) => row[2])'), ['100.25', '100.5', '100', '∅'], 'served order, never re-sorted')
    assert.equal(await page.evaluate("bid.el.querySelector('[aria-rowindex=\"3\"]').marker"), 'kept', 'the 100.5 row moved, not rebuilt')
    assert.equal(await page.evaluate("bid.el.querySelectorAll('[aria-rowindex=\"4\"] [role=gridcell]')[1].textContent"), '13')
    assert.equal(await page.evaluate("bid.el.querySelector('[aria-live]').textContent"), 'Bid: best 100.25 x 3, 4 limits')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('click, keyboard and hover reach a limit and its uuids', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.click('.ygg-ui__ladder--bid [aria-rowindex="3"]')
    assert.deepEqual(await page.evaluate('picked'), [{ price: '100.25', uuids: ['0192f0a0-0000-7000-8000-b00000000003'] }])
    await page.evaluate("bid.el.querySelector('[role=grid]').focus()")
    await page.press('End')
    await page.press('Enter')
    await page.frames()
    const last = await page.evaluate('picked.at(-1)')
    assert.deepEqual(last, { price: null, uuids: ['0192f0a0-0000-7000-8000-b00000000005'] })
    assert.match(await page.evaluate("bid.el.querySelector('[role=grid]').getAttribute('aria-activedescendant')"), /\S/)
    // The focused row describes itself through the shared tooltip.
    const described = await page.evaluate(`(() => {
      const row = bid.el.querySelector('[aria-rowindex="5"]')
      const tip = document.getElementById(row.getAttribute('aria-describedby'))
      return [tip.getAttribute('role'), tip.hidden, tip.textContent]
    })()`)
    assert.equal(described[0], 'tooltip')
    assert.equal(described[1], false)
    assert.match(described[2], /0192f0a0-0000-7000-8000-b00000000005/)
    await page.press('Escape')
    assert.equal(await page.evaluate("document.querySelector('[role=tooltip]').hidden"), true)
    await page.hover('.ygg-ui__ladder--bid [aria-rowindex="2"]')
    assert.match(await page.evaluate("document.querySelector('[role=tooltip]').textContent"), /b00000000001.*b00000000002/)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a consolidated side groups a limit uuids by the served ticker', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.evaluate(`(() => {
      const side = structuredClone(book.bid)
      side.live[1].ticker = 'BETA'
      bid.update({ side, symbol: 'GLOBAL', global: true })
    })()`)
    await page.frames()
    await page.hover('.ygg-ui__ladder--bid [aria-rowindex="2"]')
    const groups = await page.evaluate("[...document.querySelectorAll('[role=tooltip] .ygg-ui__tooltip-group')].map((group) => group.textContent)")
    assert.deepEqual(groups, ['ALPHA0192f0a0-0000-7000-8000-b00000000001', 'BETA0192f0a0-0000-7000-8000-b00000000002'])
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a thousand-limit ladder keeps its DOM under 200 rows and scrolls to the end', async (t) => {
  const { page, close } = await openPage(PAGE)
  try {
    // Measured: from the update to the frame after the one draw, in the page's own clock.
    const drawn = await page.evaluate(`(async () => {
      const limits = Array.from({ length: 1000 }, (_, at) => ({ price: String(1000 - at) + '.5', quantity: String(at + 1), uuids: ['u-' + at] }))
      limits.push({ price: null, quantity: '7', uuids: ['u-market'] })
      const started = performance.now()
      bid.update({ side: { price: '1000.5', quantity: '1', limits, live: [], deltas: [] }, symbol: 'ALPHA', global: false })
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
      return performance.now() - started
    })()`)
    t.diagnostic(`1001-limit ladder: update to drawn frame ${drawn.toFixed(1)} ms`)
    const rendered = await page.evaluate("bid.el.querySelectorAll('[role=row]').length")
    assert.ok(rendered < 200, `${rendered} rows in the DOM`)
    assert.equal(await page.evaluate("bid.el.querySelector('[role=grid]').getAttribute('aria-rowcount')"), '1002')
    await page.evaluate("bid.el.querySelector('[role=grid]').focus()")
    const jumped = await page.evaluate(`(async () => {
      const started = performance.now()
      bid.el.querySelector('[role=grid]').dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }))
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
      return performance.now() - started
    })()`)
    t.diagnostic(`1001-limit ladder: End to drawn frame ${jumped.toFixed(1)} ms`)
    assert.deepEqual(await page.evaluate('rowsOf(bid).at(-1)'), ['1', '7', '∅'])
    assert.equal(await page.evaluate("bid.el.querySelector('.ygg-ui__unpriced').getAttribute('aria-rowindex')"), '1002')
    assert.ok((await page.evaluate("bid.el.querySelectorAll('[role=row]').length")) < 200)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('both themes render, and reduced motion stops the transitions', async () => {
  const { page, close } = await openPage(PAGE, { theme: null })
  try {
    const token = "getComputedStyle(document.documentElement).getPropertyValue('--ygg-ui-page').trim()"
    await page.emulate({ colorScheme: 'dark' })
    assert.equal(await page.evaluate(token), '#000000')
    await page.settle()
    await page.screenshot('ladder-scheme-dark')
    await page.emulate({ colorScheme: 'light' })
    assert.equal(await page.evaluate(token), '#ffffff')
    assert.equal(
      await page.evaluate("getComputedStyle(bid.el.querySelector('.ygg-ui__bid')).color"),
      'rgb(154, 77, 0)',
      'the light bid text is the AA token, never the fill orange',
    )
    await page.settle()
    await page.screenshot('ladder-scheme-light')
    const motion = "getComputedStyle(document.documentElement).getPropertyValue('--ygg-ui-motion').trim()"
    assert.equal(await page.evaluate(motion), '120ms')
    await page.emulate({ colorScheme: 'light', reducedMotion: 'reduce' })
    assert.equal(await page.evaluate(motion), '0ms')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
