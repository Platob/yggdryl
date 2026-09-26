// `node/web/tooltip.js`: one shared tooltip on hover and focus, described by
// its target while it shows, hidden on blur and Escape.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { Tooltip } from '/web/tooltip.js'
const root = document.getElementById('root')
root.innerHTML = '<button id="a" class="ygg-ui__button">Alpha</button> <button id="b" class="ygg-ui__button" aria-describedby="note">Beta</button><p id="note">kept</p>'
window.tip = new Tooltip().mount(document.body)
window.detach = tip.attach(document.getElementById('a'), 'the first')
tip.attach(document.getElementById('b'), () => 'the second, built on demand')
`

test('the tooltip shows on focus and hover, describes its target and hides on blur and Escape', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.equal(await page.evaluate('tip.el.getAttribute("role")'), 'tooltip')
    assert.equal(await page.evaluate('tip.el.hidden'), true)
    await page.evaluate("document.getElementById('a').focus()")
    assert.deepEqual(await page.evaluate('[tip.el.hidden, tip.el.textContent, document.getElementById("a").getAttribute("aria-describedby") === tip.el.id]'), [false, 'the first', true])
    // Placed below its target, inside the viewport.
    const placed = await page.evaluate(`(() => {
      const target = document.getElementById('a').getBoundingClientRect()
      const box = tip.el.getBoundingClientRect()
      return box.top >= target.bottom && box.left >= 0 && box.right <= innerWidth
    })()`)
    assert.equal(placed, true)
    await page.screenshot('tooltip')
    await page.press('Escape')
    assert.equal(await page.evaluate('tip.el.hidden'), true)
    assert.equal(await page.evaluate("document.getElementById('a').hasAttribute('aria-describedby')"), false)
    await page.hover('#b')
    assert.equal(await page.evaluate('tip.el.textContent'), 'the second, built on demand')
    // A target's own description is kept beside the tooltip's.
    assert.equal(await page.evaluate("document.getElementById('b').getAttribute('aria-describedby')"), await page.evaluate("'note ' + tip.el.id"))
    await page.mouse('mouseMoved', 5, 700)
    assert.equal(await page.evaluate('tip.el.hidden'), true)
    assert.equal(await page.evaluate("document.getElementById('b').getAttribute('aria-describedby')"), 'note')
    await page.evaluate("document.getElementById('a').focus(); document.getElementById('a').blur()")
    assert.equal(await page.evaluate('tip.el.hidden'), true)
    await page.evaluate('detach()')
    await page.evaluate("document.getElementById('a').focus()")
    assert.equal(await page.evaluate('tip.el.hidden'), true, 'a detached target shows nothing')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
