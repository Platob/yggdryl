// `node/web/virtual.js`: a long list keeps only its window in the DOM, keys
// its rows, and moves an active row with the keyboard.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

test('only the window is rendered, rows are keyed, and the keyboard moves the active row', async () => {
  const { page, close } = await openPage(`
import { VirtualRows } from '/web/virtual.js'
const grid = document.createElement('div')
grid.setAttribute('role', 'grid')
grid.tabIndex = 0
document.getElementById('root').append(grid)
window.list = new VirtualRows(grid, {
  rows: 10,
  rowHeight: 20,
  key: (item) => item.id,
  create: () => document.createElement('div'),
  patch: (row, item) => { row.textContent = item.label },
})
window.items = Array.from({ length: 10000 }, (_, at) => ({ id: 'k' + at, label: 'row ' + at }))
list.setItems(items)
list.render()
grid.addEventListener('keydown', (event) => { if (list.navigate(event)) list.render() })
window.texts = () => [...list.layer.children].map((row) => row.textContent)
`)
  try {
    assert.equal(await page.evaluate("list.grid.getAttribute('aria-rowcount')"), '10001')
    const first = await page.evaluate('texts()')
    assert.equal(first[0], 'row 0')
    assert.ok(first.length <= 10 + 12, `${first.length} rows rendered`)
    assert.equal(await page.evaluate("list.layer.children[0].getAttribute('aria-rowindex')"), '2', 'after one header row')
    await page.evaluate("list.layer.children[3].marker = 'kept'; list.setItems([{ id: 'new', label: 'new' }, ...items]); list.render()")
    assert.equal(await page.evaluate("list.rowAt(4).marker"), 'kept', 'the keyed row moved down one, not rebuilt')
    await page.evaluate('list.grid.focus()')
    await page.press('End')
    assert.equal(await page.evaluate('list.active'), 10000)
    assert.equal(await page.evaluate('texts().at(-1)'), 'row 9999')
    assert.equal(await page.evaluate("document.getElementById(list.grid.getAttribute('aria-activedescendant')).textContent"), 'row 9999')
    await page.press('PageUp')
    assert.equal(await page.evaluate('list.active'), 9990)
    await page.press('Home')
    await page.press('ArrowDown')
    assert.equal(await page.evaluate('list.active'), 1)
    assert.equal(await page.evaluate("list.rowAt(1).getAttribute('aria-selected')"), 'true')
    assert.equal(await page.evaluate("list.indexOf(list.rowAt(1).firstChild ?? list.rowAt(1))"), 1)
    assert.ok((await page.evaluate('list.layer.children.length')) <= 22)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('the active row follows its item when an update inserts above it or removes it', async () => {
  const { page, close } = await openPage(`
import { VirtualRows } from '/web/virtual.js'
const grid = document.createElement('div')
grid.setAttribute('role', 'grid')
document.getElementById('root').append(grid)
window.list = new VirtualRows(grid, {
  rows: 10,
  rowHeight: 20,
  key: (item) => item.id,
  create: () => document.createElement('div'),
  patch: (row, item) => { row.textContent = item.label },
})
window.item = (id) => ({ id, label: 'row ' + id })
window.show = (ids) => { list.setItems(ids.map(item)); list.render() }
window.activeText = () => document.getElementById(list.grid.getAttribute('aria-activedescendant'))?.textContent ?? null
show(['a', 'b', 'c', 'd'])
list.active = 2
list.render()
`)
  try {
    assert.equal(await page.evaluate('activeText()'), 'row c')
    // A print prepended to the tape, a better price inserted into the ladder: the same item stays active.
    await page.evaluate("show(['new', 'a', 'b', 'c', 'd'])")
    assert.deepEqual(await page.evaluate('[list.active, activeText()]'), [3, 'row c'])
    await page.evaluate("show(['x', 'y', 'new', 'a', 'b', 'c', 'd'])")
    assert.deepEqual(await page.evaluate('[list.active, activeText()]'), [5, 'row c'])
    // Two items under one key stay two: the second one keeps its place among them.
    await page.evaluate("show(['a', 'c', 'c']); list.active = 2; list.render(); show(['c', 'a', 'c', 'c'])")
    assert.deepEqual(await page.evaluate('list.active'), 2)
    // Gone: the row that took its place is active, and it is followed from then on.
    await page.evaluate("show(['a', 'b', 'c', 'd']); list.active = 2; list.render(); show(['a', 'b', 'd'])")
    assert.deepEqual(await page.evaluate('[list.active, activeText()]'), [2, 'row d'])
    await page.evaluate("show(['z', 'a', 'b', 'd'])")
    assert.deepEqual(await page.evaluate('[list.active, activeText()]'), [3, 'row d'])
    await page.evaluate('show([])')
    assert.equal(await page.evaluate('list.active'), -1)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
