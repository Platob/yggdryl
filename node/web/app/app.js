// The replay page: a source and a symbol chosen in its header, the walk's
// books loaded through the service's wire (`api.js`), and the one component,
// `BookTimeline`, over them. Every fact on the page is what the service
// answered, and a refusal is shown as the service worded it.
// `mountApp(root, { api })` is the whole composition, so a page holding a
// recorded replay mounts it over a recorded `api` answering the same two
// methods.

import { BookTimeline } from '../book-timeline.js'
import { createApi } from './api.js'

/** The symbol the listing names the consolidated walk by. */
const GLOBAL = 'GLOBAL'

function node(tag, className, text) {
  const created = document.createElement(tag)
  if (className) created.className = className
  if (text !== undefined) created.textContent = text
  return created
}

/** A labelled `<select>`: `[label, select]`. */
function picker(text) {
  const label = node('label', 'ygg-app__picker', text)
  const select = node('select')
  label.append(select)
  return [label, select]
}

function options(select, values, chosen) {
  select.replaceChildren(...values.map((value) => new Option(value, value, false, value === chosen)))
}

/**
 * Mount the replay under `root` over `api`; answers `{ timeline, load }`,
 * `load()` answering once the chosen walk's books are shown.
 */
export async function mountApp(root, { api = createApi() } = {}) {
  const bar = node('header', 'ygg-app__bar')
  const [sourceLabel, sourceSelect] = picker('Source')
  const [symbolLabel, symbolSelect] = picker('Symbol')
  const status = node('output', 'ygg-app__status')
  status.setAttribute('aria-live', 'polite')
  bar.append(node('strong', 'ygg-app__title', 'yggdryl replay'), sourceLabel, symbolLabel, status)
  const main = node('main', 'ygg-app__main')
  root.replaceChildren(bar, main)
  const timeline = new BookTimeline().mount(main)

  const listing = await api.sources()
  if (listing.refusal) {
    status.textContent = listing.refusal
    return { timeline, load: async () => {} }
  }
  const sources = new Map(listing.sources.map((source) => [source.id, source]))
  options(sourceSelect, [...sources.keys()])

  let loading = 0
  async function load() {
    const ticket = ++loading
    const source = sources.get(sourceSelect.value)
    if (!source) return
    const symbol = symbolSelect.value
    status.textContent = `loading ${symbol}`
    const books = []
    // `GLOBAL` names the consolidated walk, one book over every symbol.
    const query = { symbol, global: symbol === GLOBAL }
    const answer = await api.books(source.id, query, { onBook: (book) => books.push(book) })
    if (ticket !== loading) return
    timeline.update({ books })
    const refused = source.refusals.length ? `, ${source.refusals.length} message(s) refused` : ''
    status.textContent = answer.refusal ?? `${books.length} books, ${source.operations} operations${refused}`
    // Each refused message, as the package worded it.
    status.title = source.refusals.join('\n')
  }

  function showSymbols() {
    const source = sources.get(sourceSelect.value)
    options(symbolSelect, source?.symbols ?? [], source?.symbols[0])
  }

  sourceSelect.addEventListener('change', () => {
    showSymbols()
    load()
  })
  symbolSelect.addEventListener('change', () => load())
  showSymbols()
  await load()
  return { timeline, load }
}

// The served page mounts itself; a page importing this module mounts it where it wants.
const host = globalThis.document?.querySelector('[data-ygg-app]')
if (host) mountApp(host)
