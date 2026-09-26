// `node/web/shortcuts-sheet.js`: every shortcut and its chord as this
// platform spells it, in a modal.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { SHORTCUTS } from '../../web/shortcuts.js'
import { openPage } from './browser.js'

test('the sheet lists every shortcut with its displayed chord', async () => {
  const { page, close } = await openPage(`
import { ShortcutsSheet } from '/web/shortcuts-sheet.js'
const root = document.getElementById('root')
root.innerHTML = '<button id="opener" class="ygg-ui__button">?</button>'
window.sheet = new ShortcutsSheet({ mac: false }).mount(root)
window.mac = new ShortcutsSheet({ mac: true }).mount(root)
document.getElementById('opener').addEventListener('click', (event) => sheet.open(event.currentTarget))
window.rows = (held) => [...held.el.querySelectorAll('tbody tr')].map((row) => [row.querySelector('kbd').textContent, row.cells[1].textContent])
`)
  try {
    await page.click('#opener')
    assert.equal(await page.evaluate('sheet.el.open'), true)
    assert.equal(await page.evaluate("document.getElementById(sheet.el.getAttribute('aria-labelledby')).textContent"), 'Keyboard shortcuts')
    const rows = await page.evaluate('rows(sheet)')
    assert.equal(rows.length, SHORTCUTS.length)
    assert.deepEqual(rows[0], ['Space', 'Play or pause'])
    assert.deepEqual(rows.find(([, label]) => label === 'Open the command palette'), ['Ctrl+k', 'Open the command palette'])
    assert.deepEqual((await page.evaluate('rows(mac)')).find(([, label]) => label === 'Open the command palette'), ['⌘k', 'Open the command palette'])
    await page.screenshot('shortcuts-sheet')
    await page.press('Escape')
    await page.waitFor('!sheet.el.open')
    // Focus returns in the dialog's close task.
    await page.waitFor("document.activeElement.id === 'opener'")
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
