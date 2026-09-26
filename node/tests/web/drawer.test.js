// `node/web/drawer.js`: a non-modal side panel its toggle expands, focus
// moving in on open and back on Escape.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

test('the drawer opens from its toggle, takes focus, and Escape closes it', async () => {
  const { page, close } = await openPage(`
import { Drawer } from '/web/drawer.js'
const root = document.getElementById('root')
root.innerHTML = '<button id="toggle" class="ygg-ui__button">Scenarios</button> <button id="elsewhere" class="ygg-ui__button">Elsewhere</button>'
window.drawer = new Drawer({ side: 'right', title: 'Scenarios' }).mount(document.body)
drawer.body.innerHTML = '<button id="first" class="ygg-ui__button">New</button>'
drawer.bind(document.getElementById('toggle'))
window.events = []
document.addEventListener('ygg:close', () => events.push('close'))
document.addEventListener('ygg:open', () => events.push('open'))
`)
  try {
    const toggle = "document.getElementById('toggle')"
    assert.equal(await page.evaluate(`${toggle}.getAttribute('aria-expanded')`), 'false')
    assert.equal(await page.evaluate(`${toggle}.getAttribute('aria-controls') === drawer.el.id`), true)
    assert.equal(await page.evaluate('drawer.el.hidden'), true)
    await page.click('#toggle')
    assert.equal(await page.evaluate('drawer.el.hidden'), false)
    assert.equal(await page.evaluate(`${toggle}.getAttribute('aria-expanded')`), 'true')
    assert.equal(await page.evaluate('document.activeElement.id'), 'first', 'focus moves in')
    assert.equal(await page.evaluate("document.getElementById(drawer.el.getAttribute('aria-labelledby')).textContent"), 'Scenarios')
    // Non-modal: the page stays live.
    assert.equal(await page.evaluate("document.getElementById('elsewhere').inert"), false)
    await page.screenshot('drawer')
    await page.press('Escape')
    assert.equal(await page.evaluate('drawer.el.hidden'), true)
    assert.equal(await page.evaluate(`${toggle}.getAttribute('aria-expanded')`), 'false')
    assert.equal(await page.evaluate('document.activeElement.id'), 'toggle', 'focus returns to the toggle')
    // The toggle closes it too; the close button as well.
    await page.click('#toggle')
    await page.click('#toggle')
    assert.equal(await page.evaluate('drawer.el.hidden'), true)
    await page.click('#toggle')
    await page.click('.ygg-ui__drawer button[aria-label="Close"]')
    assert.equal(await page.evaluate('drawer.el.hidden'), true)
    assert.deepEqual(await page.evaluate('events'), ['open', 'close', 'open', 'close', 'open', 'close'])
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
