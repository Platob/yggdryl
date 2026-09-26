// `node/web/theme.css`: every colour is a token on `:root`, and a filled
// button keeps its text readable while it is hovered, in both themes.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { test } from 'node:test'

import { WEB, openPage } from './browser.js'

const COLOUR = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\(/i

test('a colour is written once, as a token on :root, and every rule reads the token', () => {
  // Comments state the measured pairs; they keep their lines so a finding names the right one.
  const css = readFileSync(path.join(WEB, 'theme.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ''))
  const literal = css
    .split('\n')
    .map((line, index) => [index + 1, line.trim()])
    .filter(([, line]) => COLOUR.test(line) && !/^--ygg-ui-[\w-]+:/.test(line))
  assert.deepEqual(literal, [])
})

const PAGE = `
const root = document.getElementById('root')
root.innerHTML = '<p class="ygg-ui"><button id="primary" class="ygg-ui__button ygg-ui__button--primary">Insert</button> <button id="danger" class="ygg-ui__button ygg-ui__button--danger">Delete</button> <button id="plain" class="ygg-ui__button">Cancel</button></p>'
const channels = (text) => text.match(/[0-9.]+/g).map(Number)
const luminance = ([r, g, b]) => [r, g, b].map((v) => v / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)).reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0)
// A translucent background is laid over what is behind it, as the eye sees it.
const over = (top, under) => { const alpha = top[3] ?? 1; return [0, 1, 2].map((i) => top[i] * alpha + under[i] * (1 - alpha)) }
window.contrast = (id) => {
  const node = document.getElementById(id)
  const style = getComputedStyle(node)
  const behind = channels(getComputedStyle(node.parentElement).backgroundColor)
  const back = over(channels(style.backgroundColor), behind)
  const [hi, lo] = [luminance(channels(style.color)), luminance(back)].sort((a, b) => b - a)
  return Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100
}
`

test('a hovered primary or danger button keeps AA contrast in dark and light', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.emulate({ reducedMotion: 'reduce' })
    for (const theme of ['dark', 'light']) {
      await page.evaluate(`document.documentElement.dataset.theme = '${theme}'`)
      for (const id of ['primary', 'danger', 'plain']) {
        await page.hover(`#${id}`)
        await page.frames()
        assert.equal(await page.evaluate(`document.getElementById('${id}').matches(':hover')`), true)
        const ratio = await page.evaluate(`contrast('${id}')`)
        assert.ok(ratio >= 4.5, `${theme} ${id} hovered: ${ratio}`)
      }
    }
    // A backdrop reads the token too: it inherits from the dialog it is behind.
    const backdrop = await page.evaluate(`(() => {
      const dialog = document.createElement('dialog')
      dialog.className = 'ygg-ui__modal'
      document.body.append(dialog)
      dialog.showModal()
      return getComputedStyle(dialog, '::backdrop').backgroundColor
    })()`)
    assert.equal(backdrop, 'rgba(0, 0, 0, 0.55)')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
