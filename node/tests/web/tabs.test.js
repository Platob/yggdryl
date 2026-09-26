// `node/web/tabs.js`: WAI-ARIA tabs - roving focus, arrows that wrap, Home
// and End, `aria-selected` and `aria-controls` - emitting the chosen id.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

test('tabs follow the arrows, Home and End, and name their panels', async () => {
  const { page, close } = await openPage(`
import { Tabs } from '/web/tabs.js'
window.Tabs = Tabs
window.chosen = []
document.addEventListener('ygg:tab', (event) => chosen.push(event.detail.id))
window.tabs = new Tabs({ label: 'Views', tabs: [{ id: 'orders', label: 'Orders' }, { id: 'quotes', label: 'Quotes' }, { id: 'executions', label: 'Executions' }] }).mount(document.getElementById('root'))
tabs.panel('orders').textContent = 'the orders view'
window.state = () => [...tabs.el.querySelectorAll('[role=tab]')].map((tab) => [tab.textContent, tab.getAttribute('aria-selected'), tab.tabIndex, document.getElementById(tab.getAttribute('aria-controls')).hidden])
`)
  try {
    assert.deepEqual(await page.evaluate('state()'), [
      ['Orders', 'true', 0, false],
      ['Quotes', 'false', -1, true],
      ['Executions', 'false', -1, true],
    ])
    assert.equal(await page.evaluate("tabs.el.querySelector('[role=tablist]').getAttribute('aria-label')"), 'Views')
    assert.equal(await page.evaluate("document.getElementById(tabs.panel('orders').getAttribute('aria-labelledby')).textContent"), 'Orders')
    await page.evaluate("tabs.el.querySelector('[role=tab]').focus()")
    await page.press('ArrowRight')
    assert.equal(await page.evaluate('document.activeElement.textContent'), 'Quotes')
    await page.press('End')
    await page.press('ArrowRight')
    await page.press('ArrowLeft')
    await page.press('Home')
    assert.deepEqual(await page.evaluate('chosen'), ['quotes', 'executions', 'orders', 'executions', 'orders'])
    await page.click('[role=tab]:nth-child(2)')
    assert.deepEqual(await page.evaluate('state().map((row) => row[1])'), ['false', 'true', 'false'])
    await page.screenshot('tabs')
    // The keys the tabs handle never reach the page's shortcuts.
    const reached = await page.evaluate(`(() => {
      let count = 0
      const count1 = () => { count += 1 }
      document.addEventListener('keydown', count1)
      tabs.el.querySelector('[aria-selected=true]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }))
      document.removeEventListener('keydown', count1)
      return count
    })()`)
    assert.equal(reached, 0)
    // The app may choose a tab, or serve another list, without an event.
    const before = await page.evaluate('chosen.length')
    await page.evaluate("tabs.update({ selected: 'executions' })")
    await page.frames()
    assert.deepEqual(await page.evaluate('state().map((row) => row[1])'), ['false', 'false', 'true'])
    await page.evaluate("tabs.update({ tabs: [{ id: 'books', label: 'Books' }, { id: 'orders', label: 'Orders' }], selected: 'orders' })")
    await page.frames()
    assert.deepEqual(await page.evaluate('state()'), [
      ['Books', 'false', -1, true],
      ['Orders', 'true', 0, false],
    ])
    assert.equal(await page.evaluate("tabs.panel('orders').textContent"), 'the orders view', 'a kept tab keeps its panel')
    assert.equal(await page.evaluate('chosen.length'), before)
    assert.match(await page.evaluate("(() => { try { new Tabs({ tabs: [{ id: 'a', label: 'A' }, { id: 'a', label: 'B' }] }); return 'accepted' } catch (error) { return error.message } })()"), /tab id "a" is listed twice/)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
