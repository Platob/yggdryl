// `node/web/modal.js`: a native modal dialog that traps focus, closes on
// Escape and the backdrop, keeps the page inert, and refocuses its opener.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { Modal } from '/web/modal.js'
const root = document.getElementById('root')
root.innerHTML = '<button id="opener" class="ygg-ui__button">Open</button> <a id="outside" href="#x">outside</a>'
window.modal = new Modal({ title: 'Insert an event' }).mount(root)
modal.body.innerHTML = '<label>Price <input id="price"></label> <button id="save" class="ygg-ui__button">Save</button>'
window.closes = 0
document.addEventListener('ygg:close', () => { window.closes += 1 })
document.getElementById('opener').addEventListener('click', (event) => modal.open(event.currentTarget))
window.inside = () => modal.el.contains(document.activeElement)
`

test('the modal traps focus, is named by its title and leaves the page inert', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.click('#opener')
    assert.equal(await page.evaluate('modal.el.open'), true)
    assert.equal(await page.evaluate("document.getElementById(modal.el.getAttribute('aria-labelledby')).textContent"), 'Insert an event')
    assert.equal(await page.evaluate("document.getElementById('outside').inert && document.getElementById('opener').inert"), true)
    assert.equal(await page.evaluate('inside()'), true)
    const visited = []
    for (let step = 0; step < 6; step += 1) {
      await page.press('Tab')
      visited.push(await page.evaluate("inside() ? (document.activeElement.id || document.activeElement.getAttribute('aria-label')) : 'escaped'"))
    }
    assert.ok(!visited.includes('escaped'), JSON.stringify(visited))
    assert.ok(new Set(visited).size >= 3, 'every control inside is reached')
    await page.evaluate("modal.el.querySelector('button[aria-label=\"Close\"]').focus()")
    await page.press('Tab', { modifiers: 8 })
    assert.equal(await page.evaluate('document.activeElement.id'), 'save', 'Shift+Tab from the first control wraps to the last')
    await page.screenshot('modal')
    await page.press('Escape')
    assert.equal(await page.evaluate('modal.el.open'), false)
    await page.waitFor('closes === 1')
    assert.equal(await page.evaluate('document.activeElement.id'), 'opener', 'the opener has focus again')
    assert.equal(await page.evaluate("document.getElementById('outside').inert || document.getElementById('opener').inert"), false)
    assert.equal(await page.evaluate('closes'), 1)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('the backdrop and the close button close it; each close emits once', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.click('#opener')
    await page.mouse('mouseMoved', 4, 4)
    await page.mouse('mousePressed', 4, 4)
    await page.mouse('mouseReleased', 4, 4)
    assert.equal(await page.evaluate('modal.el.open'), false)
    await page.waitFor('closes === 1')
    await page.click('#opener')
    // A click inside the dialog is not a backdrop click.
    await page.click('#price')
    assert.equal(await page.evaluate('modal.el.open'), true)
    await page.click('button[aria-label="Close"]')
    assert.equal(await page.evaluate('modal.el.open'), false)
    // The dialog's close event is a task after the close itself.
    await page.waitFor('closes === 2')
    assert.equal(await page.evaluate('document.activeElement.id'), 'opener')
    // Closing a closed modal does nothing.
    await page.evaluate('modal.close()')
    assert.equal(await page.evaluate('closes'), 2)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('destroying an open modal gives the page and the opener back at once', async () => {
  const { page, close } = await openPage(`${PAGE}
import { Palette } from '/web/palette.js'
window.palette = new Palette().mount(root)
`)
  try {
    await page.click('#opener')
    assert.equal(await page.evaluate("document.getElementById('outside').inert"), true)
    // Synchronous: \`destroy\` has forgotten the dialog's own \`close\` listener.
    assert.deepEqual(
      await page.evaluate("(() => { modal.destroy(); return [document.getElementById('outside').inert, document.getElementById('opener').inert, document.activeElement.id, closes] })()"),
      [false, false, 'opener', 1],
    )
    // The palette inherits it.
    await page.evaluate("palette.open(document.getElementById('opener'))")
    assert.equal(await page.evaluate("document.getElementById('outside').inert"), true)
    assert.deepEqual(
      await page.evaluate("(() => { palette.destroy(); return [document.getElementById('outside').inert, document.activeElement.id, closes] })()"),
      [false, 'opener', 2],
    )
    await page.frames()
    assert.equal(await page.evaluate('closes'), 2, 'nothing closes twice')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
