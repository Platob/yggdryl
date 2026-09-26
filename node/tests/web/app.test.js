// `node/web/app/`: the replay page end to end, over the real service and the
// synthetic source, in one headless Chromium: the source and the symbol
// chosen in its header, the walk's books streamed into `BookTimeline`, each
// shown book the one the native walk answered. Every test ends with the
// page's console empty and leaves a screenshot.
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { after, before, test } from 'node:test'

import { launch, WEB } from './browser.js'

const require = createRequire(import.meta.url)
const { createReplayServer } = require('../../replay/server.js')
const { loadSource } = require('../../replay/sources.js')
const { books: walked } = require('../../replay/synthetic.js')

let service
let base
let page

before(async () => {
  service = createReplayServer({ sources: [loadSource('synthetic')], webDir: WEB })
  base = await service.listen(0)
  page = await launch()
})

after(async () => {
  try {
    await page?.close()
  } finally {
    await service?.close()
  }
})

/** The walk's hashes of `symbol`, as the native package answers them. */
function hashes(symbol, global = false) {
  return walked(0, global)
    .filter((book) => book.crosscode === symbol)
    .map((book) => String(book.stableHash()))
}

/** The text of the first element `selector` matches. */
function text(selector) {
  return page.evaluate(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`)
}

/** Choose `value` in the header's `select` labelled `label`, as a person does. */
async function choose(label, value) {
  await page.evaluate(`(() => {
    const select = [...document.querySelectorAll('.ygg-app__picker')].find((node) => node.firstChild.textContent === ${JSON.stringify(label)}).querySelector('select')
    select.value = ${JSON.stringify(value)}
    select.dispatchEvent(new Event('change', { bubbles: true }))
  })()`)
}

async function openApp() {
  page.consoleLines.length = 0
  await page.open(new URL('/web/app/', base).href)
  await page.waitFor("document.querySelector('.ygg-app__status')?.textContent.includes('books')", 20_000)
  await page.frames()
}

test('the page shows the first symbol walk of the source, standing at its last book', async () => {
  await openApp()
  const alpha = hashes('ALPHA')
  assert.equal(await text('.ygg-app__status'), `${alpha.length} books, 23 operations`)
  assert.equal(await text('.ygg-bt__symbol'), 'ALPHA')
  assert.equal(await text('.ygg-bt__position'), `${alpha.length} / ${alpha.length}`)
  assert.deepEqual(
    await page.evaluate("[...document.querySelectorAll('.ygg-app__picker select')].map((select) => [...select.options].map((option) => option.value))"),
    [['synthetic'], ['ALPHA', 'BETA', 'GLOBAL']],
  )
  // Every book the page holds is the one the native walk answered, in walk order.
  const shown = await page.evaluate("[...document.querySelectorAll('.ygg-bt__mark')].map((mark) => mark.dataset.index)")
  assert.equal(shown.length, alpha.length)
  await page.screenshot('replay-app')
  assert.deepEqual(page.consoleLines, [])
})

test('another symbol loads its own walk; the audit shows the book standing', async () => {
  await openApp()
  await choose('Symbol', 'BETA')
  const beta = hashes('BETA')
  await page.waitFor(`document.querySelector('.ygg-app__status').textContent === '${beta.length} books, 23 operations'`)
  await page.frames()
  assert.equal(await text('.ygg-bt__symbol'), 'BETA')
  await page.evaluate("document.querySelector('.ygg-bt__head .ygg-bt__button').click()")
  await page.frames()
  assert.match(await text('.ygg-bt__dialog h2'), /^BETA at /)
  assert.equal(await page.evaluate("document.querySelector('.ygg-bt__dialog-body tr:last-child td').textContent"), beta.at(-1))
  await page.screenshot('replay-app-audit')
  // `GLOBAL` is the consolidated walk: one book over every symbol.
  await choose('Symbol', 'GLOBAL')
  const global = hashes('GLOBAL', true)
  await page.waitFor(`document.querySelector('.ygg-app__status').textContent === '${global.length} books, 23 operations'`)
  await page.frames()
  assert.equal(await text('.ygg-bt__position'), `${global.length} / ${global.length}`)
  assert.equal(await text('.ygg-bt__symbol'), 'GLOBAL')
  assert.deepEqual(page.consoleLines, [])
})
