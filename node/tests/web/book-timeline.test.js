// `node/web/book-timeline.js`: the order book along its timeline - the
// scrubber over the walk's instants, the live limits standing at the chosen
// one marked where they changed, and the audit dialog, every side, limit and
// list a collapsible item, fitting the viewport. Every browser test ends with
// the page's console empty and leaves a screenshot.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { test } from 'node:test'

import { changedLimits, instantText, liveByUuid, priceText } from '../../web/book-timeline.js'
import { FIXTURES, openPage } from './browser.js'

const BOOKS = JSON.parse(readFileSync(path.join(FIXTURES, 'books.json'), 'utf8'))

test('an instant keeps every nanosecond, and the unpriced limit reads as market', () => {
  assert.equal(instantText('1700000001000000001'), '2023-11-14T22:13:21.000000001Z')
  assert.equal(instantText('1700000003000000000'), '2023-11-14T22:13:23.000000000Z')
  assert.equal(instantText('-1'), '1969-12-31T23:59:59.999999999Z')
  assert.equal(instantText(null), '-')
  assert.equal(priceText(null), 'market')
  assert.equal(priceText('100.5'), '100.5')
})

test('a limit changed where it is new or its quantity or live entries moved', () => {
  assert.deepEqual([...changedLimits(BOOKS[0].bid, BOOKS[1].bid)], ['99.5'])
  assert.deepEqual([...changedLimits(BOOKS[1].bid, BOOKS[2].bid)], ['100.5'])
  assert.deepEqual([...changedLimits(BOOKS[3].ask, BOOKS[4].ask)], ['101.5'])
  assert.deepEqual([...changedLimits(null, BOOKS[0].ask)], ['101'])
  assert.deepEqual([...changedLimits(BOOKS[2].bid, BOOKS[3].bid)], [])
  const live = liveByUuid(BOOKS[1].bid)
  assert.equal(live.size, 2)
  for (const limit of BOOKS[1].bid.limits) for (const uuid of limit.uuids) assert.ok(live.has(uuid))
})

const PAGE = `
import { BookTimeline } from '/web/book-timeline.js'
const books = await (await fetch('/fixtures/books.json')).json()
const root = document.getElementById('root')
root.style.height = 'calc(100vh - 32px)'
window.selected = []
document.addEventListener('ygg:book-select', (event) => window.selected.push(event.detail))
window.timeline = new BookTimeline({ books }).mount(root)
window.limits = (side) => [...document.querySelectorAll('.ygg-bt__side--' + side + ' .ygg-bt__limit')].map((row) =>
  [row.querySelector('.ygg-bt__price').textContent, row.querySelector('.ygg-bt__quantity').textContent,
   row.querySelector('.ygg-bt__orders').textContent, row.classList.contains('ygg-bt__limit--changed')])
window.text = (selector) => document.querySelector(selector)?.textContent ?? null
`

test('the timeline stands at the last book and moves through the walk', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.frames()
    assert.equal(await page.evaluate("text('.ygg-bt__symbol')"), 'ALPHA')
    assert.equal(await page.evaluate("text('.ygg-bt__instant')"), '2023-11-14T22:13:23.000000000Z')
    assert.equal(await page.evaluate("text('.ygg-bt__position')"), '5 / 5')
    assert.deepEqual(await page.evaluate("limits('bid')"), [['100.5', '3', '1', false]])
    assert.deepEqual(await page.evaluate("limits('ask')"), [['101.5', '1', '1', true]])
    assert.equal(await page.evaluate("document.querySelectorAll('.ygg-bt__mark').length"), 5)
    assert.equal(await page.evaluate("document.querySelector('.ygg-bt__mark--current').dataset.index"), '4')
    assert.equal(await page.evaluate("document.querySelector('[aria-label=\"Next book\"]').disabled"), true)
    await page.screenshot('book-timeline-dark')

    // The keyboard and the buttons move one book; each move is announced once.
    await page.evaluate("document.querySelector('[aria-label=\"Previous book\"]').click()")
    await page.frames()
    assert.equal(await page.evaluate("text('.ygg-bt__position')"), '4 / 5')
    await page.evaluate("timeline.el.focus(); timeline.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }))")
    await page.frames()
    assert.equal(await page.evaluate("text('.ygg-bt__position')"), '1 / 5')
    assert.deepEqual(await page.evaluate("limits('bid')"), [['100', '5', '1', true]])
    await page.evaluate("timeline.select(1)")
    assert.deepEqual(await page.evaluate("limits('bid')"), [['100', '5', '1', false], ['99.5', '2', '1', true]])
    assert.match(await page.evaluate("text('.ygg-bt__stats')"), /1 executed/)
    assert.deepEqual(
      await page.evaluate('selected.map((detail) => detail.index)'),
      [3, 0, 1],
    )
    assert.equal(await page.evaluate('selected[2].currunix'), '1700000001000000001')
    // A mark stands at its book.
    await page.evaluate("document.querySelector('.ygg-bt__mark[data-index=\"2\"]').click()")
    assert.equal(await page.evaluate("text('.ygg-bt__position')"), '3 / 5')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('the audit dialog holds the whole book as collapsible items and fits the viewport', async () => {
  const { page, close } = await openPage(PAGE, { theme: 'light' })
  try {
    await page.evaluate('timeline.select(1)')
    await page.evaluate("document.querySelector('.ygg-bt__head .ygg-bt__button').click()")
    await page.frames()
    assert.equal(await page.evaluate('timeline.dialog.open'), true)
    assert.equal(await page.evaluate("text('.ygg-bt__dialog h2')"), 'ALPHA at 2023-11-14T22:13:21.000000001Z')
    // The book, both sides open; each limit and each list closed until asked for.
    const summaries = () => page.evaluate("[...document.querySelectorAll('.ygg-bt__dialog-body > details > summary')].map((node) => node.textContent)")
    assert.deepEqual(await summaries(), ['Book', 'Bid · 2 limits', 'Ask · 1 limit', 'Executions · 1', 'Deltas · 1'])
    const limitItems = "[...document.querySelectorAll('.ygg-bt__audit-limits > details')]"
    assert.deepEqual(await page.evaluate(`${limitItems}.map((node) => [node.firstChild.textContent, node.open])`), [
      ['100 × 5 · 1 live', false],
      ['99.5 × 2 · 1 live', false],
      ['101 × 4 · 1 live', false],
    ])
    // A limit opens onto the entries resting at it, joined by curruuid.
    await page.evaluate(`${limitItems}[1].firstChild.click()`)
    await page.frames()
    const rested = await page.evaluate(`[...${limitItems}[1].querySelectorAll('tbody tr')].map((row) => row.cells[6].textContent)`)
    assert.deepEqual(rested, BOOKS[1].bid.limits[1].uuids)
    await page.evaluate("[...document.querySelectorAll('.ygg-bt__dialog-head button')].find((node) => node.textContent === 'Expand all').click()")
    await page.frames()
    assert.equal(await page.evaluate("document.querySelectorAll('.ygg-bt__dialog details:not([open])').length"), 0)
    // However much is open, the dialog stays within the viewport and its body scrolls.
    const fit = await page.evaluate("(() => { const box = timeline.dialog.getBoundingClientRect(); return box.top >= 0 && box.bottom <= innerHeight && box.right <= innerWidth })()")
    assert.equal(fit, true)
    await page.screenshot('book-timeline-audit-light')
    await page.evaluate("[...document.querySelectorAll('.ygg-bt__dialog-head button')].find((node) => node.textContent === 'Collapse all').click()")
    assert.equal(await page.evaluate("document.querySelectorAll('.ygg-bt__dialog details[open]').length"), 0)
    await page.press('Escape')
    await page.frames()
    assert.equal(await page.evaluate('timeline.dialog.open'), false)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('an empty walk says so and offers nothing to move or audit', async () => {
  const { page, close } = await openPage(`
import { BookTimeline } from '/web/book-timeline.js'
window.timeline = new BookTimeline({ title: 'ALPHA' }).mount(document.getElementById('root'))
`)
  try {
    assert.equal(await page.evaluate("document.querySelector('.ygg-bt__symbol').textContent"), 'ALPHA')
    assert.equal(await page.evaluate("document.querySelector('.ygg-bt__position').textContent"), '0 / 0')
    assert.equal(await page.evaluate("document.querySelector('.ygg-bt__head .ygg-bt__button').disabled"), true)
    assert.equal(await page.evaluate("document.querySelectorAll('.ygg-bt__empty').length"), 2)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
