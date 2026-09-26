// The browser smoke harness: a Chromium found on this machine, driven over
// the DevTools protocol with Node's own WebSocket, with no dependency. A page
// is served from a directory over `http` (ES modules refuse `file://`),
// evaluated, and screenshotted into the scratch directory. Absent browser: a
// loud failure naming what to set, never a silent pass.

import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
/** The component library the smoke pages import, and the fixtures they render. */
export const WEB = path.resolve(HERE, '..', '..', 'web')
export const FIXTURES = path.join(HERE, 'fixtures')

const CANDIDATES = {
  win32: [
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
  ],
  darwin: [
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
  ],
  linux: [
    '/usr/bin/google-chrome',
    '/usr/bin/google-chrome-stable',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
    '/snap/bin/chromium',
  ],
}

/** The Chromium executable: `CHROME_PATH`, else the platform's usual places. */
export function findChromium() {
  const stated = process.env.CHROME_PATH
  if (stated) return existsSync(stated) ? stated : undefined
  return (CANDIDATES[process.platform] ?? []).find((candidate) => existsSync(candidate))
}

/** Where screenshots land: `YGGDRYL_SCREENSHOT_DIR`, else the OS temp dir. */
export function screenshotDir() {
  const dir = process.env.YGGDRYL_SCREENSHOT_DIR ?? path.join(os.tmpdir(), 'yggdryl-web')
  mkdirSync(dir, { recursive: true })
  return dir
}

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
}

/** Serve `root` over http on an ephemeral port; answers `{ url, close }`. */
export async function serve(root) {
  const server = http.createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
    // A page states no icon; the browser asks anyway, and an empty answer is not a failure.
    if (pathname === '/favicon.ico') {
      response.writeHead(204).end()
      return
    }
    const file = path.join(root, pathname === '/' ? 'index.html' : pathname)
    if (!file.startsWith(path.resolve(root)) || !existsSync(file) || !statSync(file).isFile()) {
      response.writeHead(404).end('not found')
      return
    }
    response.writeHead(200, { 'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream' })
    response.end(readFileSync(file))
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const { port } = server.address()
  return {
    url: `http://127.0.0.1:${port}/`,
    close: () => new Promise((resolve) => server.close(resolve)),
  }
}

const KEYS = {
  Enter: { code: 'Enter', keyCode: 13, text: '\r' },
  Tab: { code: 'Tab', keyCode: 9 },
  Escape: { code: 'Escape', keyCode: 27 },
  Backspace: { code: 'Backspace', keyCode: 8 },
  Delete: { code: 'Delete', keyCode: 46 },
  ' ': { code: 'Space', keyCode: 32, text: ' ' },
  PageUp: { code: 'PageUp', keyCode: 33 },
  PageDown: { code: 'PageDown', keyCode: 34 },
  End: { code: 'End', keyCode: 35 },
  Home: { code: 'Home', keyCode: 36 },
  ArrowLeft: { code: 'ArrowLeft', keyCode: 37 },
  ArrowUp: { code: 'ArrowUp', keyCode: 38 },
  ArrowRight: { code: 'ArrowRight', keyCode: 39 },
  ArrowDown: { code: 'ArrowDown', keyCode: 40 },
}

function codeOf(key) {
  if (/^[a-z]$/i.test(key)) return `Key${key.toUpperCase()}`
  if (/^\d$/.test(key)) return `Digit${key}`
  return { '?': 'Slash', '/': 'Slash', '[': 'BracketLeft', ']': 'BracketRight' }[key] ?? ''
}

/** Launch a headless Chromium and attach one page; answers the page driver. */
export async function launch({ timeoutMs = 20_000 } = {}) {
  const executable = findChromium()
  assert.ok(executable, 'no Chromium found: set CHROME_PATH to a Chrome, Chromium or Edge executable')
  const profile = path.join(os.tmpdir(), `yggdryl-chrome-${process.pid}-${Date.now()}`)
  const child = spawn(
    executable,
    [
      '--headless=new',
      '--disable-gpu',
      '--no-sandbox',
      '--no-first-run',
      '--disable-extensions',
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      '--window-size=1280,800',
      'about:blank',
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] },
  )
  const exited = new Promise((resolve) => child.once('exit', resolve))
  let socket
  // The one way down, on success or failure: the browser, its pipe and its profile.
  const stop = async () => {
    socket?.close()
    child.kill()
    child.stderr.destroy()
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))])
    await discard(profile)
  }
  try {
    return await attach()
  } catch (error) {
    await stop()
    throw error
  }

  async function attach() {
    const endpoint = await new Promise((resolve, reject) => {
      let held = ''
      const fail = (error) => {
        clearTimeout(timer)
        reject(error)
      }
      const timer = setTimeout(() => fail(new Error(`Chromium gave no DevTools endpoint in ${timeoutMs}ms:\n${held}`)), timeoutMs)
      child.stderr.on('data', (chunk) => {
        held += chunk
        const match = /DevTools listening on (ws:\/\/\S+)/.exec(held)
        if (match) {
          clearTimeout(timer)
          resolve(match[1])
        }
      })
      child.once('exit', (code) => fail(new Error(`Chromium exited with ${code} before listening:\n${held}`)))
      child.once('error', fail)
    })
    socket = new WebSocket(endpoint)
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true })
      socket.addEventListener('error', () => reject(new Error(`could not connect to ${endpoint}`)), { once: true })
    })
    let next = 1
    const pending = new Map()
    const events = new Set()
    socket.addEventListener('message', (message) => {
      const data = JSON.parse(message.data)
      if (data.id !== undefined) {
        const waiter = pending.get(data.id)
        pending.delete(data.id)
        if (data.error) waiter.reject(new Error(`${waiter.method}: ${data.error.message}`))
        else waiter.resolve(data.result)
      } else if (data.method) {
        for (const listener of events) listener(data)
      }
    })
    // A call that has no answer - the browser gone, the socket closed, a page
    // that never settles - fails naming its method rather than hanging the run.
    socket.addEventListener('close', () => {
      for (const [id, waiter] of pending) {
        pending.delete(id)
        waiter.reject(new Error(`${waiter.method}: the DevTools socket closed`))
      }
    })
    const send = (method, params = {}, sessionId, timeoutMs = 60_000) =>
      new Promise((resolve, reject) => {
        const id = next++
        const timer = setTimeout(() => {
          pending.delete(id)
          reject(new Error(`${method}: no answer in ${timeoutMs}ms`))
        }, timeoutMs)
        const settle = (fn) => (value) => {
          clearTimeout(timer)
          fn(value)
        }
        pending.set(id, { resolve: settle(resolve), reject: settle(reject), method })
        if (socket.readyState !== WebSocket.OPEN) {
          pending.get(id).reject(new Error(`${method}: the DevTools socket is closed`))
          pending.delete(id)
          return
        }
        socket.send(JSON.stringify({ id, method, params, sessionId }))
      })
    const waitFor = (name, sessionId, timeoutMs = 60_000) =>
      new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          events.delete(listener)
          reject(new Error(`${name}: not heard in ${timeoutMs}ms`))
        }, timeoutMs)
        const listener = (data) => {
          if (data.method === name && data.sessionId === sessionId) {
            clearTimeout(timer)
            events.delete(listener)
            resolve(data.params)
          }
        }
        events.add(listener)
      })
    const { targetId } = await send('Target.createTarget', { url: 'about:blank' })
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true })
    await send('Page.enable', {}, sessionId)
    await send('Runtime.enable', {}, sessionId)
    await send('Log.enable', {}, sessionId)
    const consoleLines = []
    let held = 0
    events.add((data) => {
      if (data.sessionId === sessionId && data.method === 'Runtime.consoleAPICalled') {
        consoleLines.push(data.params.args.map((arg) => arg.value ?? arg.description).join(' '))
      }
      if (data.sessionId === sessionId && data.method === 'Runtime.exceptionThrown') {
        consoleLines.push(`exception: ${data.params.exceptionDetails.text} ${data.params.exceptionDetails.exception?.description ?? ''}`)
      }
      // A module that fails to load, or a request the page makes that fails, is a line too.
      if (data.sessionId === sessionId && data.method === 'Log.entryAdded' && data.params.entry.level === 'error') {
        consoleLines.push(`log: ${data.params.entry.text} ${data.params.entry.url ?? ''}`.trim())
      }
    })
    return {
      consoleLines,
      /** Navigate and wait for the load event. */
      async open(url) {
        const loaded = waitFor('Page.loadEventFired', sessionId)
        await send('Page.navigate', { url }, sessionId)
        await loaded
      },
      /** Evaluate an expression (a promise is awaited) and answer its value. */
      async evaluate(expression) {
        const result = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId)
        if (result.exceptionDetails) {
          throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text)
        }
        return result.result.value
      },
      /**
       * Press a key as a person would: the key, its code and virtual key code,
       * so the browser's own default actions (Tab moving focus, Escape closing a
       * dialog, Enter clicking a button) run. `modifiers`: Alt 1, Ctrl 2, Meta 4, Shift 8.
       */
      async press(key, { text, modifiers = 0 } = {}) {
        const known =
          KEYS[key] ?? (key.length === 1 ? { code: codeOf(key), keyCode: key.toUpperCase().charCodeAt(0), text: key } : { code: key, keyCode: 0 })
        const typed = text ?? (modifiers & 6 ? undefined : known.text)
        const common = { key, code: known.code, windowsVirtualKeyCode: known.keyCode, nativeVirtualKeyCode: known.keyCode, modifiers }
        await send('Input.dispatchKeyEvent', { ...common, type: typed ? 'keyDown' : 'rawKeyDown', text: typed, unmodifiedText: typed }, sessionId)
        await send('Input.dispatchKeyEvent', { ...common, type: 'keyUp' }, sessionId)
      },
      /** Type text into the focused field. */
      async type(text) {
        await send('Input.insertText', { text }, sessionId)
      },
      /**
       * A mouse event at viewport coordinates: `mouseMoved`, `mousePressed` or
       * `mouseReleased`. A move while a button is held says so, as a drag does.
       */
      async mouse(type, x, y, { button = 'left', clickCount = 1 } = {}) {
        if (type === 'mousePressed') held |= 1
        if (type === 'mouseReleased') held &= ~1
        const moving = type === 'mouseMoved'
        await send(
          'Input.dispatchMouseEvent',
          { type, x, y, button: moving ? (held ? 'left' : 'none') : button, buttons: held, clickCount: moving ? 0 : clickCount },
          sessionId,
        )
      },
      /** The centre of the first element `selector` matches, scrolled into view. */
      async centre(selector) {
        const point = await this.evaluate(`(() => {
          const node = document.querySelector(${JSON.stringify(selector)})
          if (!node) return null
          node.scrollIntoView({ block: 'nearest', inline: 'nearest' })
          const rect = node.getBoundingClientRect()
          return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
        })()`)
        assert.ok(point, `no element matches ${selector}`)
        return point
      },
      /** Move the pointer over an element. */
      async hover(selector) {
        const { x, y } = await this.centre(selector)
        await this.mouse('mouseMoved', x, y)
      },
      /** Click an element with the mouse. */
      async click(selector) {
        const { x, y } = await this.centre(selector)
        await this.mouse('mouseMoved', x, y)
        await this.mouse('mousePressed', x, y)
        await this.mouse('mouseReleased', x, y)
      },
      /** Wait `count` animation frames, so a scheduled draw has run. */
      async frames(count = 2) {
        await this.evaluate(
          `new Promise((resolve) => { let left = ${count}; const step = () => (--left <= 0 ? resolve(true) : requestAnimationFrame(step)); requestAnimationFrame(step) })`,
        )
      },
      /** Wait out the theme's transitions (120ms) so a screenshot shows the settled colours. */
      async settle(ms = 250) {
        await this.evaluate(`new Promise((resolve) => setTimeout(resolve, ${ms}))`)
        await this.frames()
      },
      /** Wait until `expression` is truthy in the page; answers its value. */
      async waitFor(expression, timeoutMs = 5_000) {
        const value = await this.evaluate(`new Promise((resolve) => {
          const until = performance.now() + ${timeoutMs}
          const poll = () => {
            let held
            try { held = (${expression}) } catch { held = undefined }
            if (held || performance.now() > until) resolve(held ?? null)
            else setTimeout(poll, 10)
          }
          poll()
        })`)
        assert.ok(value, `timed out waiting for ${expression}; console: ${JSON.stringify(consoleLines)}`)
        return value
      },
      /** Emulate `prefers-color-scheme` and `prefers-reduced-motion`. */
      async emulate({ colorScheme, reducedMotion } = {}) {
        const features = []
        if (colorScheme) features.push({ name: 'prefers-color-scheme', value: colorScheme })
        if (reducedMotion) features.push({ name: 'prefers-reduced-motion', value: reducedMotion })
        await send('Emulation.setEmulatedMedia', { features }, sessionId)
      },
      /** Emulate a viewport: its CSS size and its device pixel ratio. */
      async viewport({ width = 1280, height = 800, deviceScaleFactor = 1 } = {}) {
        await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor, mobile: false }, sessionId)
      },
      /** Write a PNG screenshot into the screenshot directory; answers its path. */
      async screenshot(name) {
        const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId)
        const file = path.join(screenshotDir(), `${name}.png`)
        writeFileSync(file, Buffer.from(data, 'base64'))
        return file
      },
      async close() {
        // The browser may close its socket before it answers: wait briefly, never forever.
        await send('Browser.close', {}, undefined, 5_000).catch(() => {})
        await stop()
      },
    }
  }
}

/**
 * A smoke page: the component library under `/web/`, the fixtures under
 * `/fixtures/`, and `script` as the page's one module script - it imports
 * `/web/<name>.js` and reads `await fetch('/fixtures/<name>.json')`. Served,
 * opened in a fresh headless Chromium and awaited until the script has run
 * and two frames have drawn. `theme` sets `data-theme` on `<html>`; `null`
 * leaves `prefers-color-scheme` to decide. Answers `{ page, url, close }`.
 */
export async function openPage(script, { theme = 'dark' } = {}) {
  const site = mkdtempSync(path.join(os.tmpdir(), 'yggdryl-web-'))
  let server
  let page
  // Closes what was opened, however far the opening got: the browser, the server, the folder.
  const close = async () => {
    try {
      if (page) await page.close()
    } finally {
      if (server) await server.close()
      await discard(site)
    }
  }
  try {
    await start()
  } catch (error) {
    await close()
    throw error
  }
  return { page, url: server.url, close }

  async function start() {
    cpSync(WEB, path.join(site, 'web'), { recursive: true })
    cpSync(FIXTURES, path.join(site, 'fixtures'), { recursive: true })
    const themed = theme ? ` data-theme="${theme}"` : ''
    writeFileSync(
      path.join(site, 'index.html'),
      `<!doctype html>
<html lang="en"${themed}>
<head><meta charset="utf-8"><title>yggdryl web smoke</title><link rel="stylesheet" href="/web/theme.css">
<style>body { margin: 0; padding: 16px; background: var(--ygg-ui-page); color: var(--ygg-ui-text); font-family: var(--ygg-ui-font); }</style></head>
<body><main id="root"></main>
<script type="module">
${script}
window.ready = true
</script>
</body>
</html>
`,
    )
    server = await serve(site)
    page = await launch()
    await page.open(server.url)
    // A loaded machine can take seconds to fetch and run the modules; a page
    // that logged a failure before it was ready is reported at once.
    const until = Date.now() + 30_000
    while (!(await page.evaluate('window.ready === true'))) {
      if (page.consoleLines.length) throw new Error(`the page failed before it was ready: ${JSON.stringify(page.consoleLines)}`)
      if (Date.now() > until) throw new Error('the page was not ready in 30s')
      await new Promise((resolve) => setTimeout(resolve, 20))
    }
    await page.frames()
  }
}

/**
 * Remove a scratch folder, best effort: a browser's helper processes can hold
 * a profile's files for a moment after the browser itself has exited, and a
 * folder left in the temp directory is no failure of what the test checked.
 */
async function discard(folder) {
  await rm(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }).catch(() => {})
}
