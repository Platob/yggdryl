// `node/web/theme-toggle.js`: the preference decides until the person
// chooses; the choice is `data-theme`, remembered across loads.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { ThemeToggle } from '/web/theme-toggle.js'
window.themes = []
document.addEventListener('ygg:theme', (event) => themes.push(event.detail.theme))
window.toggle = new ThemeToggle().mount(document.getElementById('root'))
window.pressed = () => toggle.el.getAttribute('aria-pressed')
window.page = () => getComputedStyle(document.documentElement).getPropertyValue('--ygg-ui-page').trim()
`

test('the toggle follows the preference, then sets and remembers the theme', async () => {
  const { page, url, close } = await openPage(PAGE, { theme: null })
  try {
    await page.emulate({ colorScheme: 'light' })
    await page.frames()
    assert.equal(await page.evaluate('document.documentElement.dataset.theme ?? null'), null, 'nothing chosen: the preference decides')
    assert.equal(await page.evaluate('pressed()'), 'false')
    // Reached by Tab, the toggle shows its focus ring.
    await page.press('Tab')
    assert.equal(await page.evaluate("document.activeElement === toggle.el && getComputedStyle(toggle.el).outlineStyle"), 'solid')
    await page.emulate({ colorScheme: 'dark' })
    await page.waitFor("pressed() === 'true'")
    await page.click('.ygg-ui__theme-toggle')
    assert.deepEqual(await page.evaluate("[document.documentElement.dataset.theme, localStorage.getItem('ygg-ui-theme'), pressed(), page()]"), ['light', 'light', 'false', '#ffffff'])
    await page.settle()
    await page.screenshot('theme-toggle-light')
    await page.evaluate('toggle.toggle()')
    assert.deepEqual(await page.evaluate("[document.documentElement.dataset.theme, localStorage.getItem('ygg-ui-theme'), pressed(), page()]"), ['dark', 'dark', 'true', '#000000'])
    assert.deepEqual(await page.evaluate('themes'), ['light', 'dark'])
    // A later load starts from the remembered choice, whatever the preference.
    await page.evaluate("localStorage.setItem('ygg-ui-theme', 'light')")
    await page.open(url)
    await page.waitFor('window.ready === true')
    assert.deepEqual(await page.evaluate("[document.documentElement.dataset.theme, pressed()]"), ['light', 'false'])
    // Storage that refuses is no failure: the toggle still switches the page.
    await page.evaluate(`(() => {
      Storage.prototype.getItem = () => { throw new Error('blocked') }
      Storage.prototype.setItem = () => { throw new Error('blocked') }
    })()`)
    await page.evaluate("window.second = new (toggle.constructor)().mount(document.getElementById('root')); second.toggle()")
    assert.equal(await page.evaluate('document.documentElement.dataset.theme'), 'dark')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
