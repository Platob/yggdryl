// `node/web/theme.css`: every colour is a token on `:root`, and every text
// the timeline draws keeps WCAG AA contrast over whatever lies behind it - a
// changed row under its depth bar included - in both themes.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { test } from 'node:test'

import { WEB, openPage } from './browser.js'

const COLOUR = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\(/i

test('a colour is written once, as a token on :root, and every rule reads the token', () => {
  // Comments keep their lines so a finding names the right one.
  const css = readFileSync(path.join(WEB, 'theme.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ''))
  const literal = css
    .split('\n')
    .map((line, index) => [index + 1, line.trim()])
    .filter(([, line]) => COLOUR.test(line) && !/^--ygg-ui-[\w-]+:/.test(line))
  assert.deepEqual(literal, [])
})

const PAGE = `
import { BookTimeline } from '/web/book-timeline.js'
const books = await (await fetch('/fixtures/books.json')).json()
window.timeline = new BookTimeline({ books, index: 1 }).mount(document.getElementById('root'))
// The worst case: every row changed, every text over the deepest bar.
for (const row of document.querySelectorAll('.ygg-bt__limit')) row.classList.add('ygg-bt__limit--changed')
const channels = (text) => text.match(/[0-9.]+/g).map(Number)
const luminance = ([r, g, b]) => [r, g, b].map((v) => v / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)).reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0)
// A translucent layer is laid over what is behind it, as the eye sees it.
const over = (top, under) => { const alpha = top[3] ?? 1; return [0, 1, 2].map((i) => top[i] * alpha + under[i] * (1 - alpha)) }
const behind = (node) => {
  const layers = []
  for (let at = node; at; at = at.parentElement) layers.push(channels(getComputedStyle(at).backgroundColor))
  const bar = node.closest('.ygg-bt__limit')?.querySelector('.ygg-bt__bar')
  let colour = [255, 255, 255]
  for (const layer of layers.reverse()) colour = over(layer, colour)
  return bar ? over(channels(getComputedStyle(bar).backgroundColor), colour) : colour
}
window.contrasts = (selector) => [...document.querySelectorAll(selector)].map((node) => {
  const [hi, lo] = [luminance(channels(getComputedStyle(node).color)), luminance(behind(node))].sort((a, b) => b - a)
  return [node.className || node.tagName, node.textContent, Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100]
})
`

const TEXTS = ['.ygg-bt__symbol', '.ygg-bt__instant', '.ygg-bt__stats > span', '.ygg-bt__position', '.ygg-bt__caption', '.ygg-bt__columns > span', '.ygg-bt__button', '.ygg-bt__limit > span:not(.ygg-bt__bar)']

test('every text of the timeline keeps AA contrast in dark and light', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    for (const theme of ['dark', 'light']) {
      await page.evaluate(`document.documentElement.dataset.theme = '${theme}'`)
      await page.frames()
      const measured = (await page.evaluate(`[${TEXTS.map((selector) => `...contrasts(${JSON.stringify(selector)})`).join(', ')}]`))
      assert.ok(measured.length > 12, `${theme}: ${measured.length} texts`)
      const low = measured.filter(([, , ratio]) => ratio < 4.5)
      assert.deepEqual(low, [], `${theme}: below 4.5`)
    }
    // The audit's backdrop reads its token too: it inherits from the dialog it is behind.
    await page.evaluate('timeline.openAudit()')
    assert.equal(await page.evaluate("getComputedStyle(timeline.dialog, '::backdrop').backgroundColor"), 'rgba(0, 0, 0, 0.55)')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
