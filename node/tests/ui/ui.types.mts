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
  type ContentValue,
} from '../../ui/index.mjs'
import {
  dataType,
  fieldDocument,
  fixManifest,
  type FieldDocument,
  type FixIndexDocument,
} from '../../ui/yggdryl.mjs'

const expectType = <T,>(_value: T): void => {}

expectType<HTMLDivElement>(make('div', 'held', 'text'))
expectType<HTMLElement>(code({ text: 42 }))
expectType<HTMLDivElement>(card({ value: 7, label: 'Positions', note: 'live' }))
expectType<HTMLDivElement>(cardRow({ cards: [{ value: 7, label: 'Positions' }] }))

const lazyPanel = panel({ title: 'Book', aside: '12', render: () => make('p') })
expectType<HTMLDetailsElement>(lazyPanel.element)
expectType<HTMLDivElement>(lazyPanel.fill())
expectType<HTMLTableElement>(facts({ rows: [{ label: 'PnL', value: 0 }] }))
expectType<HTMLDivElement>(grid({
  columns: [{ label: 'Symbol', key: 'symbol' }, { label: 'Price', get: (row) => row.price }],
  rows: [{ symbol: 'AAPL', price: 195 }],
}))
expectType<HTMLSpanElement>(pill({ text: 'required', kind: 'required' }))

const amount = input({ type: 'number', value: 10, onInput: (_event) => {} })
expectType<HTMLDivElement>(control({ id: 'amount', label: 'Amount', node: amount }))
expectType<HTMLInputElement>(search({ wide: true }))
expectType<HTMLSelectElement>(select({ options: [{ value: 'D', label: 'Order' }] }))
expectType<HTMLTextAreaElement>(textarea({ rows: 4 }))
expectType<HTMLButtonElement>(button({ label: 'Run', kind: 'primary' }))
expectType<HTMLButtonElement>(button({ label: 'Remove', kind: 'danger' }))

const modalView = modal({
  title: 'Review order',
  content: 'AAPL × 100',
  actions: button({ label: 'Confirm', kind: 'primary' }),
  onClose: (_reason) => {},
})
expectType<HTMLDialogElement>(modalView.element)
expectType<boolean>(modalView.opened)
expectType<void>(modalView.open())
expectType<void>(modalView.close('confirmed'))

const choices = choiceList({ items: [1, 2], render: (item) => item })
expectType<HTMLDivElement>(choices.element)
expectType<void>(choices.select(0))
expectType<Promise<boolean>>(wireBlock({ display: 'shown', copy: 'wire' }).copy())
expectType<HTMLDivElement>(callBlock({ code: 'reader.text(frame)', answer: 'message' }))
expectType<HTMLParagraphElement>(note({ content: 'Browser reading', kind: 'info' }))

const index = createSearchIndex({ items: [{ name: 'AAPL' }], haystack: (item) => item.name })
expectType<{ name: string }[]>(index.search('aapl'))
const listing = searchableList({
  index,
  renderPage: (items): ContentValue => items.map((item) => item.name),
})
expectType<HTMLInputElement>(listing.input)
expectType<number>(listing.refresh().remaining)
expectType<HTMLUListElement>(tree({
  items: [{ id: 1 }],
  key: (item) => item.id,
  describe: (item) => ({ label: item.id }),
}))
const lazy = createLazyIndex({ build: () => new Map<string, number>() })
expectType<Map<string, number>>(lazy.get())
expectType<boolean>(lazy.built)

const assets = assetLoader({
  baseURL: 'https://example.test/app.js',
  assets: { index: 'index.json' },
  regenerate: 'node scripts/build.js',
})
expectType<Promise<{ rows: number }>>(assets.json<{ rows: number }>('index'))
expectType<() => void>(onDocumentReady({ run: (_document) => {} }))

expectType<HTMLElement>(dataType({ value: 'utf8' }))
const field: FieldDocument = { name: 'symbol', dtype: { type: 'utf8' }, nullable: false }
expectType<HTMLUListElement>(fieldDocument({ document: field }))
const fixIndex: FixIndexDocument = {
  wire: {}, fields: [{ t: 55, n: 'Symbol', y: 'utf8' }], messages: [],
  components: [], groups: [], header: [], trailer: [],
}
const model = fixManifest({ index: fixIndex })
expectType<string>(model.field(55)!.n)
