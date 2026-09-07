# Browser UI

Build data-heavy browser pages from plain data with the dependency-free UI shipped in the `yggdryl` npm package.

## Contract

| | |
| --- | --- |
| Core | `yggdryl/ui`; DOM components accept plain configuration and plain data |
| Adapters | `yggdryl/ui/yggdryl`; optional views over datatype text, field documents, and generated FIX manifests |
| FIX | `yggdryl/ui/fix`; registry, field, message, projection, decode, encode, and frame views over a generated model or explicit host callbacks |
| Module | Pure ESM: installed MkDocs 1.6.1 accepts `extra_javascript` entries with `type: module`, so there is no classic-script, CommonJS, or global facade |
| Styles | `yggdryl/ui.css`; shipped separately, never injected, and themed only through CSS custom properties |
| Runtime | Browser DOM, Fetch, and platform APIs only; no framework, CDN, polyfill, addon import, or UI runtime dependency |
| Package | The existing `yggdryl` publish target owns the three JavaScript subpath exports and stylesheet |
| Documentation | Generated copies under `docs/assets/ui/` are checked against `node/ui/`; this page lives under Extensions because it is a browser boundary of the Node package, not a new core layer |

## Use

Install the existing package and let the host stylesheet enter the cascade explicitly.

```bash
npm install yggdryl
```

```css
@import "yggdryl/ui.css";
```

```{ .javascript .ignore }
import { cardRow } from 'yggdryl/ui'

const view = cardRow({
  cards: [
    { value: '6,210', label: 'records' },
    { value: '60', label: 'rows per page', note: 'configurable' },
  ],
})
document.body.append(view)
console.assert(view.querySelectorAll('.ygg-ui__card').length === 2)
```

The JavaScript block requires a browser DOM, so the documentation checker reports it as ignored; the live view below runs the same published module.

## Live

<div class="ygg-ui" data-ui-demo markdown="1">
This section loads the generated copy of `yggdryl/ui` and needs JavaScript.
</div>

## Element

`make(tag, className?, text?)` creates one element and assigns text with `textContent`.

| Parameter | Type | Default |
| --- | --- | --- |
| `tag` | HTML tag name | required |
| `className` | string | no class |
| `text` | string, number, or boolean | no text |

```{ .javascript .ignore }
import { make } from 'yggdryl/ui'

const heading = make('h2', 'result', 'Trades')
document.body.append(heading)
console.assert(heading.tagName === 'H2' && heading.textContent === 'Trades')
```

## Code

`code({ text, className? })` displays literal inline code.

| Key | Type | Default |
| --- | --- | --- |
| `text` | string, number, or boolean | required |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { code } from 'yggdryl/ui'

const value = code({ text: 'struct<id: int64>' })
document.body.append(value)
console.assert(value.textContent === 'struct<id: int64>')
```

## Card

`card(config)` renders one value, label, and optional note.

| Key | Type | Default |
| --- | --- | --- |
| `value` | text, node, or array of either | required |
| `label` | text, node, or array of either | required |
| `note` | text, node, or array of either | omitted |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { card } from 'yggdryl/ui'

const view = card({ value: 181, label: 'messages', note: 'indexed once' })
document.body.append(view)
console.assert(view.querySelector('.ygg-ui__card-value').textContent === '181')
```

## Card row

`cardRow(config)` lays cards out with a responsive auto-fit grid.

| Key | Type | Default |
| --- | --- | --- |
| `cards` | array of card configurations | required |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { cardRow } from 'yggdryl/ui'

const view = cardRow({ cards: [{ value: 2, label: 'files' }, { value: 1, label: 'request' }] })
document.body.append(view)
console.assert(view.children.length === 2)
```

## Panel

`panel(config)` returns `{ element, summary, body, fill }`; its native `<details>` body is filled once, on the first open.

| Key | Type | Default |
| --- | --- | --- |
| `title` | text, node, or array | required |
| `aside` | text, node, or array | omitted |
| `open` | boolean | `false` |
| `render` | `(body) => content` | required |
| `className` | string | no extra panel class |
| `summaryClassName` | string | no extra summary class |
| `bodyClassName` | string | no extra body class |

```{ .javascript .ignore }
import { panel } from 'yggdryl/ui'

let fills = 0
const view = panel({ title: 'Fields', render: () => `filled ${++fills}` })
document.body.append(view.element)
console.assert(view.body.childNodes.length === 0)
view.element.open = true
view.element.dispatchEvent(new Event('toggle'))
view.fill()
console.assert(fills === 1 && view.body.textContent === 'filled 1')
```

## Facts table

`facts(config)` skips only null, undefined, and empty-string values; values and formatter results may be text, nodes, or arrays of either.

| Key | Type | Default |
| --- | --- | --- |
| `rows` | `{ label, value, format? }[]` | required |
| `format` | `(value, row, index) => content` | inline code |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { facts } from 'yggdryl/ui'

const view = facts({ rows: [{ label: 'tag', value: 55 }, { label: 'absent', value: '' }] })
document.body.append(view)
console.assert(view.rows.length === 1 && view.rows[0].cells[1].textContent === '55')
```

## Grid

`grid(config)` returns a bounded scrolling container with a sticky header and per-column formatters. A string column reads the positional array value; an object resolves `get`, then `key`, then the positional value at that column index.

| Key | Type | Default |
| --- | --- | --- |
| `columns` | label strings or `{ label, get?, key?, format?, rowHeader?, className?, headerClassName? }` objects | required |
| `rows` | plain data array | required |
| `empty` | text | `—` |
| `className` | string | no extra table class |

```{ .javascript .ignore }
import { grid } from 'yggdryl/ui'

const view = grid({
  columns: [{ label: 'Name', get: (row) => row.name, rowHeader: true }, { label: 'Value', get: (row) => row.value }],
  rows: [{ name: 'price', value: null }],
})
document.body.append(view)
console.assert(view.querySelector('tbody td').textContent === '—')
```

## Pill

`pill(config)` maps semantic kinds to theme tokens rather than fixed component colours.

| Key | Type | Default |
| --- | --- | --- |
| `text` | text, node, or array | required |
| `kind` | `accent`, `success`, `info`, `warning`, `quiet`, `required`, `group`, `unknown`, `codes`, `branch`, or `danger` | neutral |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { pill } from 'yggdryl/ui'

const tag = pill({ text: 'required', kind: 'required' })
document.body.append(tag)
console.assert(tag.classList.contains('ygg-ui__pill--required'))
```

## Labelled control

`control({ id, label, node, className? })` binds one visible label to an existing form control and returns the wrapper `<div>`.

| Key | Type | Default |
| --- | --- | --- |
| `id` | string | required |
| `label` | text, node, or array | required |
| `node` | input, select, or textarea element | required |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { control, input } from 'yggdryl/ui'

const node = input()
const view = control({ id: 'symbol', label: 'Symbol', node })
document.body.append(view)
console.assert(view.querySelector('label').htmlFor === node.id)
```

## Input

`input(config)` creates a native text input and attaches handlers directly.

| Key | Type | Default |
| --- | --- | --- |
| `type` | input type | `text` |
| `value` | string, number, or boolean | unset |
| `placeholder`, `name`, `autocomplete`, `ariaLabel` | string | unset |
| `disabled`, `wide` | boolean | `false` |
| `onInput`, `onChange` | event listener | unset |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { input } from 'yggdryl/ui'

const node = input({ value: 'AAPL', name: 'symbol', wide: true })
document.body.append(node)
console.assert(node.value === 'AAPL' && node.type === 'text')
```

## Search box

`search(config)` accepts the input configuration and fixes its native type to `search`.

| Key | Type | Default |
| --- | --- | --- |
| `value`, `placeholder`, `name`, `autocomplete`, `ariaLabel` | string | unset |
| `disabled`, `wide` | boolean | `false` |
| `onInput`, `onChange` | event listener | unset |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { search } from 'yggdryl/ui'

const node = search({ placeholder: 'Tag, name, datatype…' })
document.body.append(node)
console.assert(node.type === 'search')
```

## Select

`select(config)` creates a native select from plain option records.

| Key | Type | Default |
| --- | --- | --- |
| `options` | `{ label, value, disabled?, selected? }[]` | `[]` |
| `value`, `name`, `ariaLabel` | string | unset |
| `disabled`, `wide` | boolean | `false` |
| `onChange` | event listener | unset |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { select } from 'yggdryl/ui'

const node = select({ options: [{ label: 'Nasdaq', value: 'XNAS' }], value: 'XNAS' })
document.body.append(node)
console.assert(node.value === 'XNAS' && node.options.length === 1)
```

## Textarea

`textarea(config)` accepts the shared text-control keys plus native multiline settings.

| Key | Type | Default |
| --- | --- | --- |
| `value`, `placeholder`, `name`, `autocomplete`, `ariaLabel`, `wrap` | string | unset |
| `rows` | positive integer | browser default |
| `spellcheck`, `disabled` | boolean | browser default / `false` |
| `onInput`, `onChange` | event listener | unset |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { textarea } from 'yggdryl/ui'

const node = textarea({ value: '8=FIX.4.4|35=D|', rows: 2, spellcheck: false })
document.body.append(node)
console.assert(node.rows === 2 && node.value.includes('35=D'))
```

## Button

`button(config)` creates a native button with semantic visual kinds and optional pressed state.

| Key | Type | Default |
| --- | --- | --- |
| `label` | text, node, or array | required |
| `kind` | `primary`, `danger`, `chip`, or host kind | neutral |
| `type` | button type | `button` |
| `pressed`, `disabled` | boolean | unset / `false` |
| `onClick` | event listener | unset |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { button } from 'yggdryl/ui'

let clicks = 0
const node = button({ label: 'Apply', kind: 'primary', onClick: () => { clicks += 1 } })
document.body.append(node)
node.click()
console.assert(clicks === 1 && node.type === 'button')
```

`danger` is the solid red action reserved for destructive operations; `primary` uses the coral accent.

## Modal

`modal(config)` creates an accessible dialog. Browsers with `HTMLDialogElement.showModal()` use the native top layer and focus containment; other browsers receive a scoped fixed overlay with dialog semantics, trapped keyboard focus, Escape and backdrop dismissal, and focus restoration.

| Key | Type | Default |
| --- | --- | --- |
| `title` | text, node, or array | required |
| `content`, `actions` | text, node, or array | empty |
| `closeLabel` | text, node, or array | `Close` |
| `closeOnEscape`, `closeOnBackdrop` | boolean | `true` |
| `initialFocus` | element inside the modal | close button |
| `onOpen` | callback | unset |
| `onClose` | callback receiving `button`, `backdrop`, `escape`, `api`, `native`, or the caller's reason | unset |
| `className`, `surfaceClassName`, `bodyClassName` | string | no extra class |
| Return | `{ element, surface, body, closeButton, opened, open, close }` | one controller |

```{ .javascript .ignore }
import { button, modal } from 'yggdryl/ui'

const trigger = button({ label: 'Review order' })
const confirm = button({ label: 'Confirm', kind: 'primary' })
const view = modal({
  title: 'Review order',
  content: 'Buy 100 AAPL',
  actions: confirm,
})
trigger.addEventListener('click', view.open)
confirm.addEventListener('click', () => view.close('confirmed'))
document.body.append(trigger, view.element)
trigger.focus()
trigger.click()
console.assert(view.opened && view.element.getAttribute('aria-modal') === 'true')
view.close()
console.assert(!view.opened && document.activeElement === trigger)
```

Append `element` before calling `open()`: native dialogs can only enter the top layer after they are connected. The modal restores the element focused at `open()` whenever it closes.

## Choice list

`choiceList(config)` returns `{ element, buttons, select, selectedIndex }` over a fixed item list.

| Key | Type | Default |
| --- | --- | --- |
| `items` | iterable | required |
| `render` | `(item, index) => content` | required |
| `onSelect` | `(item, index) => void` | unset |
| `selected` | integer index | `-1` |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { choiceList } from 'yggdryl/ui'

const view = choiceList({ items: ['Wire', 'Fields'], render: (item) => item, selected: 0 })
document.body.append(view.element)
view.select(1)
console.assert(view.selectedIndex === 1 && view.buttons[1].getAttribute('aria-pressed') === 'true')
```

## Wire block

`wireBlock(config)` returns `{ element, body, button, copy }`; display text and copied text are independent.

| Key | Type | Default |
| --- | --- | --- |
| `display` | string | required |
| `copy` | string | `display` |
| `duration` | milliseconds | `1200` |
| `labels` | `{ copy?, copied?, unavailable? }` | English labels |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { wireBlock } from 'yggdryl/ui'

const view = wireBlock({ display: '8=FIX.4.4␁', copy: '8=FIX.4.4\u0001' })
document.body.append(view.element)
console.assert(view.body.textContent.endsWith('␁') && view.body.textContent !== '8=FIX.4.4\u0001')
```

## Call block

`callBlock(config)` keeps a code snippet beside the answer it produced.

| Key | Type | Default |
| --- | --- | --- |
| `code` | string | required |
| `answer` | text, node, or array | omitted |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { callBlock } from 'yggdryl/ui'

const view = callBlock({ code: 'lookup(55)', answer: 'Symbol' })
document.body.append(view)
console.assert(view.textContent.includes('lookup(55)') && view.textContent.includes('Symbol'))
```

## Note

`note(config)` renders a short accented aside.

| Key | Type | Default |
| --- | --- | --- |
| `content` | text, node, or array | required |
| `kind` | `accent`, `success`, `info`, `warning`, or `danger` | accent |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { note } from 'yggdryl/ui'

const view = note({ content: 'Index loaded.', kind: 'success' })
document.body.append(view)
console.assert(view.textContent === 'Index loaded.')
```

## Search index

`createSearchIndex(config)` returns `{ entries, size, search(query, filter?) }` and computes one lowercase haystack per item at construction. The FIX explorer indexes all 6,210 records once so each keystroke only scans prepared strings.

| Key | Type | Default |
| --- | --- | --- |
| `items` | iterable | required |
| `haystack` | `(item, index) => string` | required |

```{ .javascript .ignore }
import { createSearchIndex } from 'yggdryl/ui'

const index = createSearchIndex({ items: [{ tag: 55, name: 'Symbol' }], haystack: (row) => `${row.tag} ${row.name}` })
const output = document.createElement('output')
output.textContent = index.search('symbol')[0].name
document.body.append(output)
console.assert(index.size === 1 && output.textContent === 'Symbol')
```

## Searchable list

`searchableList(config)` caps the DOM, reports the true match count, and states the exact undrawn count; silent truncation is never used. It returns `{ element, controls, input, count, list, more, refresh({ reset? }?), reset }`.

| Key | Type | Default |
| --- | --- | --- |
| `index` | result of `createSearchIndex` | required |
| `renderPage` | `(visible, counts) => content` | required |
| `pageSize` | positive integer | `60` |
| `noun` | string | `items` |
| `filter` | item predicate | unset |
| `label` | content | `Search` |
| `controls` | content placed beside the search control | omitted |
| `id`, `placeholder`, `query`, `className` | string | generated / unset / empty / unset |

```{ .javascript .ignore }
import { createSearchIndex, searchableList } from 'yggdryl/ui'

const index = createSearchIndex({ items: ['one', 'two', 'three'], haystack: (item) => item })
const view = searchableList({
  index,
  pageSize: 2,
  renderPage: (items) => items.join(', '),
})
document.body.append(view.element)
console.assert(view.count.textContent.includes('2 drawn, 1 not drawn'))
```

## Tree

`tree(config)` recursively renders leaf descriptions and lazy panel branches while protecting the active path from cycles.

| Key | Type | Default |
| --- | --- | --- |
| `items` | root iterable | required |
| `key` | `(item) => stable key` | required |
| `describe` | `(item) => { label, aside?, branch?, children?, beforeChildren?, open?, className? }` | required |
| `className` | string | no extra root class |

```{ .javascript .ignore }
import { tree } from 'yggdryl/ui'

const root = { id: 'root', children: [] }
root.children.push(root)
const view = tree({
  items: [root],
  key: (item) => item.id,
  describe: (item) => ({ label: item.id, branch: true, open: true, children: () => item.children }),
})
document.body.append(view)
view.querySelector('details').open = true
view.querySelector('details').dispatchEvent(new Event('toggle'))
console.assert(view.textContent.includes('cycle'))
```

## Lazy index

`createLazyIndex(config)` returns `{ get(), built }`; every caller shares the first result or failure, and recursive construction is refused. The FIX carrier cross-index walks 181 layouts with thousands of tag visits: rebuilding it per field panel previously cost about 300 ms per click, while the single lazy page build is imperceptible.

| Key | Type | Default |
| --- | --- | --- |
| `build` | `() => value` | required |

```{ .javascript .ignore }
import { createLazyIndex } from 'yggdryl/ui'

let builds = 0
const lazy = createLazyIndex({ build: () => ({ count: ++builds }) })
const output = document.createElement('output')
output.textContent = String(lazy.get().count)
document.body.append(output)
console.assert(lazy.get().count === 1 && lazy.built && builds === 1)
```

## Asset loader

`assetLoader(config)` returns `{ address, json, prefetch, failure }`. Load the small index for first paint, then prefetch detail data through the default two-animation-frame scheduler; each resolved URL is fetched once per page load and failures name the regeneration command.

| Key | Type | Default |
| --- | --- | --- |
| `baseURL` | URL or URL string | required |
| `assets` | name-to-string-or-URL object | required |
| `regenerate` | command string | required |
| `fetch` | Fetch-compatible function | global `fetch` |
| `schedule` | `(run) => void` | after first paint |

```{ .javascript .ignore }
import { assetLoader } from 'yggdryl/ui'

let requests = 0
const loader = assetLoader({
  baseURL: location.href,
  assets: { index: 'index.json', details: 'details.json' },
  regenerate: 'node scripts/build_docs_data.js',
  fetch: async () => ({ ok: true, json: async () => ({ value: ++requests }) }),
  schedule: (run) => run(),
})
const first = loader.json('index')
console.assert(first === loader.json('index'))
const value = await first
document.body.append(String(value.value))
console.assert(requests === 1)
```

## Document readiness

`onDocumentReady(config)` runs now, on `DOMContentLoaded`, or through an instant-navigation observable and returns cleanup.

| Key | Type | Default |
| --- | --- | --- |
| `run` | `(document) => void` | required |
| `document` | document-like object | global document |
| `navigation` | subscribable observable | Material `document$` when present |

```{ .javascript .ignore }
import { onDocumentReady } from 'yggdryl/ui'

let runs = 0
const cleanup = onDocumentReady({ run: (document) => { runs += 1; document.body.dataset.ready = 'yes' } })
console.assert(typeof cleanup === 'function')
addEventListener('load', () => console.assert(runs >= 1 && document.body.dataset.ready === 'yes'), { once: true })
```

## Datatype adapter

`dataType(config)` displays a package-produced datatype string without parsing or normalizing it.

| Key | Type | Default |
| --- | --- | --- |
| `value` | string | required |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { dataType } from 'yggdryl/ui/yggdryl'

const view = dataType({ value: 'decimal128(18, 4)' })
document.body.append(view)
console.assert(view.textContent === 'decimal128(18, 4)')
```

## Field-document adapter

`fieldDocument(config)` renders the parsed structural object returned by `Field.toJSON()` as lazy field panels; it does not validate schema semantics.

| Key | Type | Default |
| --- | --- | --- |
| `document` | field document object | required |
| `open` | boolean | `false` |
| `className` | string | no extra class |

```{ .javascript .ignore }
import { fieldDocument } from 'yggdryl/ui/yggdryl'

const field = { name: 'id', nullable: false, metadata: {}, dtype: { type: 'int64' } }
const view = fieldDocument({ document: field, open: true })
document.body.append(view)
console.assert(view.textContent.includes('id') && view.textContent.includes('int64'))
```

## FIX-manifest adapter

`fixManifest(config)` indexes the generated FIX index without copying its records; split details can arrive later through `detail(record, lateManifest)`.

| Key | Type | Default |
| --- | --- | --- |
| `index` | generated FIX index object | required |
| `details` | generated detail object | `null` |
| Return | lookups, layout maps/tree, search index, lazy carriers, and detail resolver | one indexed view |

```{ .javascript .ignore }
import { fixManifest } from 'yggdryl/ui/yggdryl'

const view = fixManifest({
  index: {
    wire: {},
    fields: [{ t: 55, n: 'Symbol', y: 'ascii' }],
    messages: [], components: [], groups: [], header: [], trailer: [],
  },
})
document.body.append(view.layoutTree([]))
console.assert(view.field('symbol').t === 55 && view.searchIndex.size === 1)
```

The FIX components below consume that indexed model. A manifest is a
package-generated, read-only snapshot. Optional `decode`, `validate`, and
registry `actions` callbacks are the boundary to a host running the native
package; the browser components never imitate their validation or mutation
rules.

## FIX text inspection

`inspectFixText(config)` reads visible FIX text against a manifest without
typing values. It reports arrival-order pairs, groups, unknowns, direction,
message type, version, BodyLength, and CheckSum.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `text` | string | required |
| Return | `FixInspection` | one browser reading |

```{ .javascript .ignore }
import { inspectFixText } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const model = fixManifest({ index })
const reading = inspectFixText({ model, text: '8=FIX.4.4|35=D|55=AAPL|10=000|' })
console.assert(reading.messageType === 'D' && reading.pairs.length === 4)
```

## FIX text composition

`composeFixText(config)` creates a manifest-guided browser draft. `wire`
always contains real SOH delimiters; `display` uses the requested visible
separator. This computes BodyLength and CheckSum in the browser and is not the
package encoder.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `messageType` | string | required |
| `entries` | iterable of `[key, value]` or `{ key, value }` | required |
| `beginString` | string | `FIX.4.4` |
| `separator` | string or `soh` | `|` |
| Return | `{ wire, display, pairs, bodyLength, checksum, byteLength, conflicts, message }` | one draft |

```{ .javascript .ignore }
import { composeFixText } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const model = fixManifest({ index })
const draft = composeFixText({ model, messageType: 'D', entries: [[55, 'AAPL']] })
console.assert(draft.wire.includes('\u0001') && draft.display.includes('|'))
```

## FIX registry summary

`fixRegistrySummary(config)` displays package-generated registry, datatype,
branch, lineage, projection, and specification counts.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| Return | `HTMLDivElement` | summary cards and lazy detail panels |

```{ .javascript .ignore }
import { fixRegistrySummary } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixRegistrySummary({ model: fixManifest({ index }) })
document.body.append(view)
console.assert(view.classList.contains('ygg-ui__fix-summary'))
```

## FIX source table

`fixSourceTable(config)` renders the pinned dictionary inputs and their hashes,
formats, versions, and licences.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| Return | `HTMLDivElement` | source grid |

```{ .javascript .ignore }
import { fixSourceTable } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixSourceTable({ model: fixManifest({ index }) })
document.body.append(view)
console.assert(view.classList.contains('ygg-ui__fix-source'))
```

## FIX field explorer

`fixFieldExplorer(config)` searches the precomputed field haystacks, caps the
DOM, filters by kind and datatype, and fetches code sets and lineages only when
a field opens.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `loadDetails` | detail document or function returning one, optionally asynchronously | model details |
| `renderDetailsError` | `(error) => content` | compact error note |
| `pageSize` | positive integer | `60` |
| Return | searchable list plus `kind` and `dtype` selects | one explorer |

```{ .javascript .ignore }
import { fixFieldExplorer } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const model = fixManifest({ index })
const view = fixFieldExplorer({
  model,
  loadDetails: () => fetch('./fix-codes.json').then((response) => response.json()),
})
document.body.append(view.element)
console.assert(view.input.type === 'search' && view.kind.tagName === 'SELECT')
```

## FIX message explorer

`fixMessageExplorer(config)` filters message types and lazily expands the
selected layout, including components, groups, and required fields.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| Return | `{ element, input, select, summary, view, refresh, show }` | one explorer |

```{ .javascript .ignore }
import { fixMessageExplorer } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixMessageExplorer({ model: fixManifest({ index }) })
document.body.append(view.element)
console.assert(view.input.type === 'search' && typeof view.show === 'function')
```

## FIX projection explorer

`fixProjectionExplorer(config)` searches the fixed capture projection generated
by `fix.FixProjection`; it does not resolve columns again in the browser.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `pageSize` | positive integer | `60` |
| Return | searchable list | one projection view |

```{ .javascript .ignore }
import { fixProjectionExplorer } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixProjectionExplorer({ model: fixManifest({ index }) })
document.body.append(view.element)
console.assert(view.count.getAttribute('aria-live') === 'polite')
```

## FIX decode workbench

`fixDecodeWorkbench(config)` gives immediate manifest inspection while typing.
When `decode` is supplied, its explicit button awaits the authoritative host
answer and displays either that content or the unmodified error message.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `loadDetails` | detail loader | model details |
| `frames` | generated frame array | model frames |
| `value` | initial text | first generated frame |
| `pageSize` | positive integer | `60` |
| `renderDetailsError` | `(error) => content` | compact error note |
| `decode` | `(text, inspection) => content or promise` | no host action |
| Return | workbench with `setValue`, `inspect`, `decode`, and current `reading` | one decoder |

```{ .javascript .ignore }
import { fixDecodeWorkbench } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixDecodeWorkbench({
  model: fixManifest({ index }),
  value: '8=FIX.4.4|35=D|55=AAPL|10=000|',
})
document.body.append(view.element)
console.assert(view.reading.messageType === 'D')
```

## FIX frame gallery

`fixFrameGallery(config)` shows generated corpus frames beside the package
answers captured when the manifests were built.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `frames` | generated frame array | model frames |
| Return | `{ element, choices, view, select }` | one gallery |

```{ .javascript .ignore }
import { fixFrameGallery } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixFrameGallery({ model: fixManifest({ index }) })
document.body.append(view.element)
console.assert(typeof view.select === 'function')
```

## FIX encode workbench

`fixEncodeWorkbench(config)` builds a bounded form from one generated message
layout. Its browser draft is explicit; `validate` is the optional authority
boundary and `onDecode` hands the exact SOH wire to another view.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | required |
| `loadDetails` | detail loader | model details |
| `messageType` | string | `D` when present |
| `beginString` | string | `FIX.4.4` |
| `values` | tag/value object or iterable | built-in demonstration values |
| `renderDetailsError` | `(error) => content` | compact error note |
| `onDecode` | `(wire, draft) => void` | omitted |
| `validate` | `(wire, draft) => content or promise` | no host action |
| Return | workbench with setters, `compose`, `validate`, and current `draft` | one encoder |

```{ .javascript .ignore }
import { fixEncodeWorkbench } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixEncodeWorkbench({
  model: fixManifest({ index }),
  messageType: 'D',
  values: { 55: 'AAPL', 38: '100' },
})
document.body.append(view.element)
console.assert(view.draft.wire.includes('\u0001'))
```

## FIX registry editor

`fixRegistryEditor(config)` reads either a compact manifest or full canonical
`Field.toJSON()` documents. Mutations are enabled only when the documents and
all four host actions are supplied; it reloads `actions.list()` after every
successful write.

| Key | Type | Default |
| --- | --- | --- |
| `model` | result of `fixManifest` | omitted |
| `fields` | iterable of canonical field documents | omitted; required with actions |
| `actions` | `{ list, insert, update, removeById }`, sync or async | read-only |
| `loadDetails` | detail loader for manifest mode | model details |
| `renderDetailsError` | `(error) => content` in manifest mode | compact error note |
| `pageSize` | positive integer | `60` |
| Return | editor with selection, refresh, insert, update, remove, and authoritative fields | one workbench |

```{ .javascript .ignore }
import { fixRegistryEditor } from 'yggdryl/ui/fix'
import { fixManifest } from 'yggdryl/ui/yggdryl'

const index = await fetch('./fix.json').then((response) => response.json())
const view = fixRegistryEditor({ model: fixManifest({ index }) })
document.body.append(view.element)
console.assert(view.editor === null && view.fields === null)
```

## Theme tokens

The light defaults use Material variables when present, `[data-md-color-scheme="slate"]` supplies the dark set, and standalone hosts may select `[data-ygg-ui-theme="light|dark"]` or replace tokens before loading the stylesheet. The standalone selectors also set the matching native `color-scheme` for form controls and scrollbars.

| Tokens | Control |
| --- | --- |
| `--ygg-ui-surface`, `--ygg-ui-raised`, `--ygg-ui-line` | surfaces and borders |
| `--ygg-ui-text`, `--ygg-ui-muted`, `--ygg-ui-quiet` | text hierarchy |
| `--ygg-ui-code-fg`, `--ygg-ui-code-bg` | literal and block code |
| `--ygg-ui-accent`, `--ygg-ui-accent-soft` | focus and primary accent |
| `--ygg-ui-info`, `--ygg-ui-info-soft` | informational pills and notes |
| `--ygg-ui-warning`, `--ygg-ui-warning-soft` | warnings and cycles |
| `--ygg-ui-success`, `--ygg-ui-success-soft` | success and branch states |
| `--ygg-ui-danger`, `--ygg-ui-danger-fg`, `--ygg-ui-danger-soft` | destructive actions and danger states |
| `--ygg-ui-font`, `--ygg-ui-code-font` | text and monospace families |
| `--ygg-ui-radius` | component corners |
| `--ygg-ui-shadow`, `--ygg-ui-modal-shadow`, `--ygg-ui-backdrop` | card elevation and modal depth |
| `--ygg-ui-grid-max-block-size`, `--ygg-ui-grid-sticky-top` | scrolling-grid bound and sticky offset |
| `--ygg-ui-focus-width` | visible focus outline |
| `--ygg-ui-details-icon` | panel chevron mask |

## Edges

- DOM constructors fail explicitly when no browser document exists; the package root and native addon are never imported.
- Content is text or a real node, never HTML interpretation; caller values are not trimmed or stringified as markup.
- Count UTF-8 bytes with `new TextEncoder().encode(value).length`; JavaScript string length counts UTF-16 code units.
- Clipboard permission and API failures fall back to the document copy command and report unavailable when neither works.
- Modal and panel selectors are class-scoped; host dialogs and Material details keep their native styling. Append a modal before opening it so native `showModal()` can enter the top layer.
- The grid's horizontal overflow container is also its bounded vertical scroll container; change `--ygg-ui-grid-max-block-size` deliberately.
- Panel marker suppression is scoped to `details.ygg-ui__panel`; the plain Material fixture in the live view must retain its marker.
- `tree` stops only cycles on the active path, so the same acyclic object may appear in separate branches.
- `localStorage` is not read or written by the kit.

## Commands

```bash
npm test --prefix node
npm run --prefix node test:package:files
node scripts/build_docs_ui.js --check
python scripts/check_docs_examples.py --lang javascript
python -m mkdocs build --strict
```
