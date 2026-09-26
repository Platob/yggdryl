// `node/web/toast.js`: stacked notices; information leaves by itself, a
// refusal stays until dismissed and is announced as an alert.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

test('toasts stack, information expires, refusals stay verbatim until dismissed', async () => {
  const { page, close } = await openPage(`
import { Toasts } from '/web/toast.js'
window.toasts = new Toasts({ timeoutMs: 300 }).mount(document.body)
window.texts = () => [...toasts.el.querySelectorAll('.ygg-ui__toast')].map((toast) => [toast.closest('[role]').getAttribute('role'), toast.querySelector('.ygg-ui__toast-text').textContent])
`)
  try {
    await page.evaluate("toasts.push({ text: 'Scenario spike saved' })")
    await page.evaluate("toasts.push({ text: '$.operations[1].kind: expected an operation event, got book_side', kind: 'refusal' })")
    await page.evaluate("toasts.push({ text: 'Replay loaded', kind: 'info' })")
    assert.deepEqual(await page.evaluate('texts()'), [
      ['alert', '$.operations[1].kind: expected an operation event, got book_side'],
      ['status', 'Scenario spike saved'],
      ['status', 'Replay loaded'],
    ])
    // Both regions exist before anything is announced into them.
    assert.equal(await page.evaluate("toasts.el.querySelector('[role=status]').getAttribute('aria-live')"), 'polite')
    await page.screenshot('toast')
    await page.evaluate('new Promise((resolve) => setTimeout(resolve, 600))')
    assert.deepEqual(await page.evaluate('texts()'), [['alert', '$.operations[1].kind: expected an operation event, got book_side']])
    await page.click('.ygg-ui__toast button[aria-label="Dismiss"]')
    assert.deepEqual(await page.evaluate('texts()'), [])
    assert.match(await page.evaluate("(() => { try { toasts.push({ text: 'x', kind: 'warning' }); return 'accepted' } catch (error) { return error.message } })()"), /expected kind 'info' or 'refusal'/)
    // A pushed toast answers its own dismissal.
    await page.evaluate("window.held = toasts.push({ text: 'held', kind: 'refusal' }); held.dismiss()")
    assert.deepEqual(await page.evaluate('texts()'), [])
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
