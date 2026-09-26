// `node/web/metrics.js`: exactly the readings the service served, as a
// definition list, the BBO line the only live region.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { Metrics } from '/web/metrics.js'
const book = await (await fetch('/fixtures/book.json')).json()
const books = await (await fetch('/fixtures/books.json')).json()
window.book = book
window.books = books
const root = document.getElementById('root')
root.style.width = '420px'
window.metrics = new Metrics().mount(root)
metrics.update({ book })
window.pairs = () => [...metrics.el.querySelectorAll('dt')].map((term) => [term.textContent, term.nextElementSibling.textContent])
`

test('the metrics render the served readings verbatim', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.deepEqual(await page.evaluate('pairs()'), [
      ['Spread', '0.5'],
      ['Midpoint', '100.75'],
      ['Median quantity', '6'],
      ['Crossed', 'no'],
      ['Locked', 'no'],
      ['Imbalance, 1 level', '0.333333333333333333'],
      ['Imbalance, 5 levels', '0.737804878048780487'],
      ['Imbalance, 10 levels', '0.737804878048780487'],
      ['Bid depth, 1 level', '8'],
      ['Bid depth, 5 levels', '28.5'],
      ['Bid depth, 10 levels', '28.5'],
      ['Ask depth, 1 level', '4'],
      ['Ask depth, 5 levels', '4.3'],
      ['Ask depth, 10 levels', '4.3'],
      ['Executions', '2'],
      ['Bid deltas', '1'],
      ['Ask deltas', '0'],
      ['Grid tick', 'no'],
    ])
    const live = await page.evaluate("[...metrics.el.querySelectorAll('[aria-live]')].map((node) => [node.getAttribute('aria-live'), node.textContent])")
    assert.deepEqual(live, [['polite', 'Bid 100.5 x 8, ask 101 x 4']])
    await page.screenshot('metrics')
    await page.evaluate('metrics.update({ book: books[3] })')
    await page.frames()
    const tick = Object.fromEntries(await page.evaluate('pairs()'))
    assert.equal(tick['Grid tick'], 'yes')
    assert.equal(tick.Spread, '—', 'a null reading is shown as absent, never as zero')
    assert.equal(tick['Imbalance, 5 levels'], '—')
    assert.equal(tick['Bid deltas'], '0')
    await page.evaluate("metrics.update({ book: { ...book, bid: { ...book.bid, price: null, quantity: null } } })")
    await page.frames()
    assert.equal(await page.evaluate("metrics.el.querySelector('[aria-live]').textContent"), 'Bid none, ask 101 x 4')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
