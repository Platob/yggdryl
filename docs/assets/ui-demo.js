import {
  assetLoader,
  button,
  callBlock,
  card,
  cardRow,
  choiceList,
  code,
  control,
  createLazyIndex,
  createSearchIndex,
  facts,
  grid,
  input,
  make,
  modal,
  note,
  onDocumentReady,
  panel,
  pill,
  search,
  searchableList,
  select,
  textarea,
  tree,
  wireBlock,
} from './ui/index.js'
import { dataType, fieldDocument, fixManifest } from './ui/yggdryl.js'

let renderId = 0

const heading = (text) => make('h3', null, text)

function renderDemo(root) {
  root.textContent = ''
  root.classList.add('ygg-ui')

  const standalone = card({ value: '$2.42M', label: 'Net asset value', note: 'plain DOM' })
  console.assert(standalone.classList.contains('ygg-ui__card'))
  root.append(
    heading('Cards and semantic tags'),
    standalone,
    cardRow({
      cards: [
        { value: '+$18,420', label: 'Day P&L', note: '+0.76% at market close' },
        { value: '$184K', label: 'Available cash', note: '7.6% of portfolio' },
        { value: '-1.2%', label: 'Drawdown', note: 'from the 30-day high' },
      ],
    }),
    pill({ text: '+4.8%', kind: 'success' }),
    document.createTextNode(' '),
    pill({ text: 'market open', kind: 'group' }),
    document.createTextNode(' '),
    pill({ text: '-0.3%', kind: 'danger' }),
  )

  let panelFills = 0
  const lazyPanel = panel({
    title: 'Lazy facts panel',
    aside: 'open, close, and open again',
    className: 'ygg-ui-demo__panel',
    render: () => {
      panelFills += 1
      const table = facts({
        rows: [
          { label: 'filled', value: `${panelFills} time` },
          { label: 'empty row', value: '' },
          { label: 'node value', value: code({ text: 'int64' }), format: (value) => value },
        ],
      })
      table.dataset.uiDemoPanelFacts = ''
      return table
    },
  })
  console.assert(lazyPanel.body.childElementCount === 0)
  lazyPanel.element.dataset.uiDemoPanel = ''
  root.append(heading('Panel and facts'), lazyPanel.element)

  const table = grid({
    columns: [
      { label: 'Tag', get: (row) => row.tag, rowHeader: true },
      { label: 'Name', get: (row) => row.name, format: (value) => String(value) },
      { label: 'Market value', get: (row) => row.value, className: 'ygg-ui__cell--number' },
    ],
    rows: [
      { tag: 35, name: 'AAPL', value: '$28,410' },
      { tag: 55, name: 'MSFT', value: '$21,005' },
      { tag: 44, name: 'NVDA', value: null },
    ],
  })
  table.dataset.uiDemoGrid = ''
  root.append(heading('Scrolling grid'), table)

  const controls = make('div', 'ygg-ui__controls')
  const name = input({ value: 'AAPL', wide: true })
  const filter = search({ placeholder: 'Search controls' })
  const venue = select({
    value: 'XNAS',
    options: [
      { label: 'Nasdaq', value: 'XNAS' },
      { label: 'Euronext Paris', value: 'XPAR' },
    ],
  })
  const message = textarea({ value: '8=FIX.4.4|35=D|', rows: 2, spellcheck: false })
  controls.append(
    control({ id: 'ygg-ui-demo-name', label: 'Symbol', node: name }),
    control({ id: 'ygg-ui-demo-filter', label: 'Filter', node: filter }),
    control({ id: 'ygg-ui-demo-venue', label: 'Venue', node: venue }),
    control({ id: 'ygg-ui-demo-message', label: 'Message', node: message }),
  )
  const controlStatus = note({ content: 'Controls are native inputs.', kind: 'info' })
  const apply = button({
    label: 'Apply',
    kind: 'primary',
    onClick: () => {
      controlStatus.textContent = `${name.value} routes to ${venue.value}.`
    },
  })
  apply.dataset.uiDemoButton = ''
  const choices = choiceList({
    items: ['Summary', 'Fields', 'Wire'],
    selected: 0,
    render: (item) => item,
    onSelect: (item) => {
      controlStatus.textContent = `${item} selected.`
    },
  })
  choices.select(0)
  console.assert(choices.selectedIndex === 0 && choices.buttons.length === 3)
  root.append(heading('Controls and choices'), controls, apply, choices.element, controlStatus)

  const modalStatus = note({ content: 'No order review opened.', kind: 'info' })
  let orderModal
  const confirmOrder = button({
    label: 'Confirm buy',
    kind: 'primary',
    onClick: () => orderModal.close('confirmed'),
  })
  const deleteDraft = button({
    label: 'Delete draft',
    kind: 'danger',
    onClick: () => orderModal.close('deleted'),
  })
  orderModal = modal({
    title: 'Review order',
    content: facts({
      rows: [
        { label: 'symbol', value: 'AAPL' },
        { label: 'side', value: 'Buy' },
        { label: 'quantity', value: 100 },
      ],
    }),
    actions: [deleteDraft, confirmOrder],
    onClose: (reason) => {
      modalStatus.textContent = `Order review closed: ${reason}.`
    },
  })
  orderModal.element.dataset.uiDemoModal = ''
  const reviewOrder = button({
    label: 'Review order',
    onClick: orderModal.open,
  })
  reviewOrder.dataset.uiDemoModalOpen = ''
  root.append(heading('Modal and destructive action'), reviewOrder, orderModal.element, modalStatus)

  const wire = wireBlock({
    display: '8=FIX.4.4␁35=D␁',
    copy: '8=FIX.4.4\u000135=D\u0001',
    labels: { copy: 'Copy wire', copied: 'Copied byte', unavailable: 'Clipboard unavailable' },
    className: 'ygg-ui-demo__wire',
  })
  wire.element.dataset.uiDemoWire = ''
  console.assert(wire.body.textContent.includes('␁'))
  root.append(
    heading('Displayed and copied values'),
    wire.element,
    callBlock({
      code: "reader.bytes(Buffer.from(frame, 'binary'))",
      answer: note({ content: 'The answer stays beside the call that produced it.', kind: 'success' }),
    }),
  )

  const records = Array.from({ length: 84 }, (_, index) => ({
    tag: index + 1,
    name: `Field ${index + 1}`,
    kind: index % 7 === 0 ? 'group' : 'field',
  }))
  let indexed = 0
  const searchIndex = createSearchIndex({
    items: records,
    haystack: (record) => {
      indexed += 1
      return `${record.tag} ${record.name} ${record.kind}`
    },
  })
  console.assert(indexed === records.length)
  const searchable = searchableList({
    index: searchIndex,
    pageSize: 8,
    noun: 'fields',
    placeholder: 'Field 42 or group',
    renderPage: (rows) =>
      grid({
        columns: [
          { label: 'Tag', get: (row) => row.tag, rowHeader: true },
          { label: 'Name', get: (row) => row.name, format: (value) => String(value) },
          {
            label: 'Kind',
            get: (row) => row.kind,
            format: (value) => pill({ text: value, kind: value === 'group' ? 'group' : 'quiet' }),
          },
        ],
        rows,
      }),
  })
  searchable.element.dataset.uiDemoSearchable = ''
  root.append(heading('Capped searchable list'), searchable.element)

  const layout = { id: 'order', label: 'Order', children: [] }
  const parties = { id: 'parties', label: 'Parties', children: [] }
  const symbol = { id: 'symbol', label: 'Symbol', children: [] }
  layout.children.push(symbol, parties)
  parties.children.push(layout)
  const layoutTree = tree({
    items: [layout],
    key: (item) => item.id,
    describe: (item) => ({
      label: item.label,
      aside: `${item.children.length} children`,
      branch: item.children.length > 0,
      children: () => item.children,
    }),
  })
  layoutTree.dataset.uiDemoTree = ''
  root.append(heading('Lazy recursive tree'), layoutTree)

  let indexBuilds = 0
  const lazy = createLazyIndex({
    build: () => {
      indexBuilds += 1
      return new Map([[55, ['NewOrderSingle']]])
    },
  })
  console.assert(!lazy.built)
  const first = lazy.get()
  console.assert(first === lazy.get() && lazy.built && indexBuilds === 1)

  const run = ++renderId
  const requests = []
  const assets = assetLoader({
    baseURL: import.meta.url,
    assets: {
      index: `demo-index.json?run=${run}`,
      details: `demo-details.json?run=${run}`,
    },
    regenerate: 'node scripts/build_docs_ui.js',
    fetch: async (url) => {
      requests.push(url)
      return {
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => ({ asset: url.includes('details') ? 'details' : 'index' }),
      }
    },
    schedule: (load) => load(),
  })
  const indexRequest = assets.json('index')
  console.assert(indexRequest === assets.json('index'))
  const detailsRequest = assets.prefetch('details')
  const infrastructure = panel({
    title: 'Lazy indexes and split assets',
    aside: 'open to inspect the states',
    render: () => [
      facts({
        rows: [
          { label: 'lazy builds', value: indexBuilds },
          { label: 'index address', value: assets.address('index') },
        ],
      }),
      assets.failure({ asset: 'details', error: new Error('failure rendering example') }),
    ],
  })
  infrastructure.element.dataset.uiDemoInfrastructure = ''
  root.append(heading('Infrastructure states'), infrastructure.element)
  Promise.all([indexRequest, detailsRequest]).then(([indexData, detailData]) => {
    console.assert(indexData.asset === 'index' && detailData.asset === 'details')
    console.assert(requests.length === 2)
    infrastructure.summary.append(pill({ text: 'index first', kind: 'success' }))
  })

  const field = {
    name: 'trade',
    nullable: false,
    metadata: { comment: 'Adapter input is plain structural data.' },
    dtype: {
      type: 'struct',
      fields: [
        { field: { name: 'id', nullable: false, metadata: {}, dtype: { type: 'int64' } } },
        { field: { name: 'venue', nullable: true, metadata: {}, dtype: { type: 'utf8' } } },
      ],
    },
  }
  const manifest = fixManifest({
    index: {
      wire: {},
      fields: [{ t: 55, n: 'Symbol', y: 'ascii' }],
      messages: [{ y: 'D', n: 'NewOrderSingle', i: 1, m: [['f', 55, 1]] }],
      components: [],
      groups: [],
      header: [],
      trailer: [],
    },
    details: { '55:': { c: [['AAPL', 'Apple', '', '', 0, '']] } },
  })
  const symbolRecord = manifest.field('symbol')
  console.assert(symbolRecord.t === 55 && manifest.carriers(55).length === 1)
  root.append(
    heading('Optional yggdryl adapters'),
    facts({
      rows: [
        { label: 'datatype', value: dataType({ value: 'struct<id: int64>' }), format: (value) => value },
        { label: 'FIX lookup', value: manifest.titleOf(symbolRecord) },
        { label: 'FIX detail', value: JSON.stringify(manifest.detail(symbolRecord)) },
      ],
    }),
    fieldDocument({ document: field, open: true }),
  )

  const plain = document.createElement('details')
  plain.dataset.uiDemoPlainDetails = ''
  const plainSummary = document.createElement('summary')
  plainSummary.textContent = 'Plain Material details marker fixture'
  const plainBody = document.createElement('p')
  plainBody.textContent = 'This is deliberately not a kit panel; its native theme marker must remain.'
  plain.append(plainSummary, plainBody)
  console.assert(!plain.classList.contains('ygg-ui__panel'))
  root.append(heading('Marker regression fixture'), plain)
}

onDocumentReady({
  run: (document) => {
    for (const root of document.querySelectorAll('[data-ui-demo]')) renderDemo(root)
  },
})

