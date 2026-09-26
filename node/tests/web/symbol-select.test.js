// `node/web/symbol-select.js`: the served symbols in served order, the
// consolidated book named as such, a choice emitted with its mode.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

test('the symbols are served order, GLOBAL is the consolidated book, a choice names its mode', async () => {
  const { page, close } = await openPage(`
import { SymbolSelect } from '/web/symbol-select.js'
window.chosen = []
document.addEventListener('ygg:symbol', (event) => chosen.push(event.detail))
window.symbols = new SymbolSelect().mount(document.getElementById('root'))
symbols.update({ symbols: ['BETA', 'ALPHA', 'GLOBAL'], symbol: 'ALPHA', global: false })
window.select = () => symbols.el.querySelector('select')
window.pick = (value) => { select().value = value; select().dispatchEvent(new Event('change', { bubbles: true })) }
`)
  try {
    assert.deepEqual(await page.evaluate("[...select().options].map((option) => [option.value, option.textContent])"), [
      ['BETA', 'BETA'],
      ['ALPHA', 'ALPHA'],
      ['GLOBAL', 'GLOBAL (consolidated)'],
    ])
    assert.equal(await page.evaluate('select().value'), 'ALPHA')
    assert.equal(await page.evaluate('select().labels[0].textContent.trim().startsWith("Symbol")'), true)
    await page.evaluate("pick('GLOBAL')")
    await page.evaluate("pick('BETA')")
    assert.deepEqual(await page.evaluate('chosen'), [
      { symbol: 'GLOBAL', global: true },
      { symbol: 'BETA', global: false },
    ])
    await page.evaluate("symbols.update({ symbols: ['BETA', 'ALPHA', 'GLOBAL'], symbol: 'ALPHA', global: true })")
    await page.frames()
    assert.equal(await page.evaluate('select().value'), 'GLOBAL', 'global mode shows the consolidated book')
    await page.screenshot('symbol-select')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
