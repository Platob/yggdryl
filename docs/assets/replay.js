/*
 * Render docs/assets/replay.json: the order book replay, recorded.
 *
 * The replay service is Node over a native addon, so this page cannot run it.
 * scripts/build_docs_replay.js ran the package over two sources and recorded
 * what the service answers; this module mounts the package's own page
 * (docs/assets/web/app/app.js, copied from node/web/app/) over an `api` that
 * answers from that record. Nothing is computed here - no book folded, no
 * reading taken: a walk the record does not hold is refused, and says so.
 *
 * The page runs in a frame of its own, so its full-height layout and its
 * keyboard meet no documentation around them. This one module is both sides:
 * on the documentation page it builds the frame; in the frame, which names
 * it again, it mounts the page.
 *
 * The script is loaded on every page and does nothing where no container asks
 * for it.
 */

const MANIFEST = new URL('replay.json', import.meta.url)
const WEB = new URL('web/', import.meta.url)
const COMMAND = 'node scripts/build_docs_replay.js'
const FRAME = 'data-replay-frame'

let pending = null

/** Fetch the manifest once per document, whatever asks for it first. */
function manifest() {
  pending ??= fetch(MANIFEST).then((answer) => {
    if (!answer.ok) throw new Error(`${answer.status} ${answer.statusText}`)
    return answer.json()
  })
  return pending
}

function make(tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

/** The frame the page runs in: this module again, told by `FRAME` which side it is. */
function renderFrame(root) {
  root.textContent = ''
  const frame = make('iframe', 'ygg-rp__frame')
  frame.title = 'The order book replay over the recorded books'
  frame.srcdoc = [
    '<!doctype html>',
    `<html lang="en" ${FRAME}>`,
    '<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">',
    '<title>Recorded replay</title><link rel="icon" href="data:,">',
    `<link rel="stylesheet" href="${new URL('theme.css', WEB)}">`,
    `<link rel="stylesheet" href="${new URL('app/app.css', WEB)}">`,
    '<style>html, body { height: 100%; margin: 0; } body { background: var(--ygg-ui-page); }</style>',
    `<script type="module" src="${import.meta.url}"></script></head>`,
    '<body><div class="ygg-app" data-replay-root><p>Loading the recorded replay.</p></div></body></html>',
  ].join('')
  root.append(frame)
}

function start() {
  for (const root of document.querySelectorAll('[data-replay="app"]')) renderFrame(root)
}

/** The service's two methods, answered from the record. */
function recordedApi(data) {
  return Object.freeze({
    sources: async () => data.listing,
    books: async (id, query = {}, { onBook } = {}) => {
      const books = data.sources[id]?.books[query.symbol]
      if (books === undefined) {
        return { refusal: `the recorded replay holds no walk of ${id} for ${query.symbol}; ${COMMAND} records it`, status: 404 }
      }
      for (const book of books) onBook?.(book)
      return { count: books.length }
    },
  })
}

/** Follow the documentation's colour scheme when the page opens. */
function followScheme() {
  try {
    const scheme = window.parent.document.body.getAttribute('data-md-color-scheme')
    if (scheme) document.documentElement.dataset.theme = scheme === 'slate' ? 'dark' : 'light'
  } catch {
    // A frame the page cannot read keeps the preference's theme.
  }
}

async function mountRecorded() {
  const root = document.querySelector('[data-replay-root]')
  try {
    const [{ mountApp }, data] = await Promise.all([import(new URL('app/app.js', WEB)), manifest()])
    followScheme()
    await mountApp(root, { api: recordedApi(data) })
  } catch (error) {
    root.textContent = `The recorded replay could not be mounted (${error.message}). A local build writes it with: ${COMMAND}`
  }
}

if (document.documentElement.hasAttribute(FRAME)) {
  mountRecorded()
} else if (typeof document$ !== 'undefined' && document$ && typeof document$.subscribe === 'function') {
  // Material's instant navigation swaps the document without reloading this
  // module, so the render is driven by its document observable where it exists.
  document$.subscribe(start)
} else if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', start)
} else {
  start()
}
