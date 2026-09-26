// `node/web/diff-view.js`: the diff route's instants - two hashes and a count
// per instant - each expanding into what `diff.js` found, keyed as it keyed.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { DiffView } from '/web/diff-view.js'
import { diffStreams } from '/web/diff.js'
const base = await (await fetch('/fixtures/books.json')).json()
const other = structuredClone(base)
// The scenario: a bid limit added and a hash moved at the third instant, an
// execution gone at the last, and one more book of its own.
other[2].stableHash = '2003'
other[2].bid.limits.push({ price: '99.0', quantity: '1', uuids: ['0192f0a0-0000-7000-8000-900000000001'] })
other[2].spread = '0.5'
other[4].stableHash = '2005'
other[4].executions = other[4].executions.slice(0, 1)
other.push({ ...structuredClone(base[4]), currunix: '1700000003000000001', stableHash: '2006' })
window.rows = diffStreams(base, other)
window.seeks = []
document.addEventListener('ygg:seek', (event) => seeks.push(event.detail.at))
const root = document.getElementById('root')
root.style.width = '900px'
window.view = new DiffView().mount(root)
view.update({ rows })
window.shown = () => [...view.el.querySelectorAll('tbody tr.ygg-ui__diff-row')].map((row) => [...row.cells].slice(0, 4).map((cell) => cell.textContent))
`

test('the diff lists the changed instants with both hashes and a count, and seeks', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.deepEqual(await page.evaluate('shown()'), [
      ['2023-11-14T22:13:21.500000000Z', '1003', '2003', '3'],
      ['2023-11-14T22:13:23.000000000Z', '1005', '2005', '2'],
      ['2023-11-14T22:13:23.000000001Z', '—', '2006', 'added'],
    ])
    assert.equal(await page.evaluate("view.el.querySelector('.ygg-ui__diff-summary').textContent"), '3 of 6 instants differ')
    await page.click('.ygg-ui__diff-unchanged input')
    await page.frames()
    assert.equal(await page.evaluate('shown().length'), 6)
    assert.deepEqual(await page.evaluate('shown()[0]'), ['2023-11-14T22:13:21.000000000Z', '1001', '1001', 'same'])
    await page.click('.ygg-ui__diff-unchanged input')
    await page.frames()
    await page.click('tbody tr.ygg-ui__diff-row .ygg-ui__diff-seek')
    assert.deepEqual(await page.evaluate('seeks'), ['1700000001500000000'])
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('an expanded instant shows the facts, limits, entries and executions by their keys', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    const toggle = 'tbody tr.ygg-ui__diff-row:nth-of-type(1) button[aria-expanded]'
    assert.equal(await page.evaluate(`document.querySelector('${toggle}').getAttribute('aria-expanded')`), 'false')
    await page.click(toggle)
    await page.frames()
    assert.equal(await page.evaluate(`document.querySelector('${toggle}').getAttribute('aria-expanded')`), 'true')
    const detail = await page.evaluate(`(() => {
      const region = document.getElementById(document.querySelector('${toggle}').getAttribute('aria-controls'))
      return [region.hidden, [...region.querySelectorAll('li')].map((item) => item.textContent)]
    })()`)
    assert.equal(detail[0], false)
    assert.deepEqual(detail[1], [
      'fact spread: — → 0.5',
      'fact stableHash: 1003 → 2003',
      'bid limit added 99.0: 1 (1 entry)',
    ], 'the limit is keyed by its served text: 99.0 stays 99.0')
    await page.click('tbody tr.ygg-ui__diff-row:nth-of-type(3) button[aria-expanded]')
    await page.frames()
    const second = await page.evaluate(`[...document.getElementById(document.querySelector('tbody tr.ygg-ui__diff-row:nth-of-type(3) button[aria-expanded]').getAttribute('aria-controls')).querySelectorAll('li')].map((item) => item.textContent)`)
    assert.deepEqual(second, ['fact stableHash: 1005 → 2005', 'execution removed 0192f0a0-0000-7000-8000-e00000000013'])
    await page.screenshot('diff-view')
    await page.evaluate('view.update({ rows: [] })')
    await page.frames()
    assert.equal(await page.evaluate("view.el.querySelector('.ygg-ui__diff-summary').textContent"), 'No instants to compare')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
