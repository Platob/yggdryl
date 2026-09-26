// `node/web/lifecycle.js`: one element's chain in served order, the
// predecessor link, and the metadata and security ids in the served key order.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { Lifecycle } from '/web/lifecycle.js'
const chain = await (await fetch('/fixtures/lifecycle.json')).json()
window.chain = chain
window.selected = []
document.addEventListener('ygg:select-element', (event) => window.selected.push(event.detail))
const root = document.getElementById('root')
root.style.width = '760px'
window.lifecycle = new Lifecycle().mount(root)
lifecycle.update(chain)
window.cells = () => [...lifecycle.el.querySelectorAll('.ygg-ui__lifecycle-chain tbody tr')].map((row) => [...row.cells].map((cell) => cell.textContent))
window.pairs = (name) => [...lifecycle.el.querySelectorAll('[data-table="' + name + '"] tbody tr')].map((row) => [...row.cells].map((cell) => cell.textContent))
`

test('the chain renders in served order with its states, instants and predecessor links', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.equal(await page.evaluate("lifecycle.el.querySelector('h3').textContent"), 'ALPHA-O-1')
    assert.deepEqual(await page.evaluate('cells()'), [
      ['1', '20NEW', 'order_event', '2023-11-14T22:13:21.000000000Z', '0192f0a0', '—'],
      ['2', '40PARTFILL', 'order_event', '2023-11-14T22:13:21.500000000Z', '0192f0a0', '0192f0a0'],
      ['3', '80FILLED', 'order_event', '2023-11-14T22:13:23.000000000Z', '0192f0a0', '0192f0a0'],
    ])
    // The last statement is shown until another is chosen; its maps are served
    // as [key, value] pairs and render in that order, an integer-like key included.
    assert.deepEqual(await page.evaluate("pairs('metadata')"), [['31027', 'X'], ['countryofissue', 'US'], ['tech.clientid', 'C-7']])
    assert.deepEqual(await page.evaluate("pairs('securityids')"), [['CUSIP', '037833100'], ['ISIN', 'US0378331005']])
    await page.click('.ygg-ui__lifecycle-chain tbody tr:nth-child(3) .ygg-ui__link')
    const highlighted = await page.evaluate(`(() => {
      const rows = [...lifecycle.el.querySelectorAll('.ygg-ui__lifecycle-chain tbody tr')]
      return [rows.map((row) => row.classList.contains('ygg-ui__highlight')), rows.indexOf(document.activeElement.closest('tr'))]
    })()`)
    assert.deepEqual(highlighted, [[false, true, false], 1], 'the predecessor is highlighted and focused')
    await page.screenshot('lifecycle')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('selecting a statement names it and shows its own tables', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.click('.ygg-ui__lifecycle-chain tbody tr:nth-child(2) .ygg-ui__lifecycle-select')
    await page.frames()
    assert.deepEqual(await page.evaluate('selected'), [{ crosscode: 'ALPHA-O-1', curruuid: '0192f0a0-0000-7000-8000-f00000000002' }])
    assert.deepEqual(await page.evaluate("pairs('metadata').map((pair) => pair[0])"), ['31027', 'countryofissue', 'tech.clientid', 'tech.venue'])
    // Pairs in any order render in that order: nothing is re-keyed or sorted here.
    await page.evaluate("lifecycle.update({ ...chain, rows: chain.rows.map((row) => ({ ...row, securityids: [['ZZ', '1'], ['__proto__', '2'], ['10', '3'], ['AA', '4']] })) })")
    await page.frames()
    assert.deepEqual(await page.evaluate("pairs('securityids')"), [['ZZ', '1'], ['__proto__', '2'], ['10', '3'], ['AA', '4']])
    assert.equal(await page.evaluate("lifecycle.el.querySelector('.ygg-ui__lifecycle-chain tbody tr:nth-child(2)').getAttribute('aria-selected')"), 'true')
    // Keyboard: the select button is reachable and Enter selects.
    await page.evaluate("lifecycle.el.querySelector('.ygg-ui__lifecycle-chain tbody tr:nth-child(1) .ygg-ui__lifecycle-select').focus()")
    await page.press('Enter')
    assert.equal(await page.evaluate('selected.at(-1).curruuid'), '0192f0a0-0000-7000-8000-f00000000001')
    // A chain whose predecessor was not served shows the uuid, not a link; an empty chain says so.
    await page.evaluate("lifecycle.update({ crosscode: 'X', rows: [chain.rows[2]] })")
    await page.frames()
    assert.equal(await page.evaluate("lifecycle.el.querySelectorAll('.ygg-ui__link').length"), 0)
    assert.equal(await page.evaluate("lifecycle.el.querySelector('tbody tr td:last-child').textContent"), '0192f0a0')
    await page.evaluate("lifecycle.update({ crosscode: 'NONE', rows: [] })")
    await page.frames()
    assert.equal(await page.evaluate("lifecycle.el.querySelector('.ygg-ui__lifecycle-empty').textContent"), 'No statements for NONE')
    // Nothing chosen: the two tables of the chosen statement are not shown at all.
    assert.deepEqual(
      await page.evaluate("(() => { const details = lifecycle.el.querySelector('.ygg-ui__lifecycle-details'); return [details.hidden, getComputedStyle(details).display, details.getClientRects().length] })()"),
      [true, 'none', 0],
    )
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
