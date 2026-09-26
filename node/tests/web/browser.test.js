// The harness itself: a Chromium is found, a served module page runs, a
// screenshot lands. Absent browser: the assertion in `launch` names it.
import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

import { launch, openPage, serve } from './browser.js'

const WEB = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'web')

test('a served module page runs in headless Chromium and screenshots', async () => {
  const site = mkdtempSync(path.join(os.tmpdir(), 'yggdryl-web-'))
  writeFileSync(
    path.join(site, 'index.html'),
    `<!doctype html><html data-theme="dark"><head><link rel="stylesheet" href="/theme.css"></head>
<body><main id="root" class="ygg-ui"></main>
<script type="module">
  import { toPixels } from '/instant.js'
  import { formatDecimal } from '/decimal.js'
  import { createStore } from '/store.js'
  const store = createStore({ px: '82.50' })
  document.getElementById('root').textContent = formatDecimal(store.get().px) + ' @ ' + toPixels(1700000000000000005n, 1700000000000000000n, 1n)
  window.ready = true
</script></body></html>`,
  )
  // The pure modules are served from the library folder beside the page.
  for (const name of ['instant.js', 'decimal.js', 'store.js', 'theme.css']) {
    writeFileSync(path.join(site, name), await import('node:fs').then((fs) => fs.readFileSync(path.join(WEB, name))))
  }
  const server = await serve(site)
  let page
  try {
    page = await launch()
    await page.open(server.url)
    assert.equal(await page.evaluate('window.ready'), true)
    assert.equal(await page.evaluate("document.getElementById('root').textContent"), '82.5 @ 5')
    assert.equal(
      await page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--ygg-ui-page').trim()"),
      '#000000',
    )
    const file = await page.screenshot('harness')
    assert.ok(file.endsWith('harness.png'))
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await page?.close()
    await server.close()
    rmSync(site, { recursive: true, force: true })
  }
  assert.equal(existsSync(site), false, 'the page folder is gone')
})

const mine = () => readdirSync(os.tmpdir()).filter((name) => name.startsWith(`yggdryl-chrome-${process.pid}-`))

test('a launch that fails part way leaves no browser, no pipe and no profile behind', async () => {
  const before = new Set(mine())
  // No browser answers in a millisecond: the wait for its endpoint fails, as a hung one would.
  await assert.rejects(launch({ timeoutMs: 1 }), /no DevTools endpoint in 1ms/)
  // A browser left running would write its profile after the failure, and hold this process open.
  await new Promise((resolve) => setTimeout(resolve, 2_000))
  assert.deepEqual(mine().filter((name) => !before.has(name)), [])
})

test('a page that fails before it is ready closes the browser it opened', async () => {
  const before = new Set(mine())
  await assert.rejects(openPage("throw new Error('refused on load')"), /failed before it was ready.*refused on load/)
  assert.deepEqual(mine().filter((name) => !before.has(name)), [])
})
