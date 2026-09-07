import {
  composeFixText,
  fixDecodeWorkbench,
  fixEncodeWorkbench,
  fixFieldExplorer,
  fixFrameGallery,
  fixMessageExplorer,
  fixProjectionExplorer,
  fixRegistryEditor,
  fixRegistrySummary,
  fixSourceTable,
  inspectFixText,
  type FixComposition,
  type FixBrowserIndexDocument,
  type FixBrowserManifestModel,
  type FixGeneratedFrame,
  type FixInspection,
  type FixRegistryActions,
} from '../../ui/fix.mjs'
import { fixManifest, type FieldDocument, type FixManifestModel } from '../../ui/yggdryl.mjs'

const expectType = <T,>(_value: T): void => {}

const index: FixBrowserIndexDocument = {
  wire: {},
  fields: [
    { t: 8, n: 'BeginString', y: 'utf8' },
    { t: 35, n: 'MsgType', y: 'msgtype' },
    { t: 55, n: 'Symbol', y: 'utf8' },
  ],
  messages: [{ y: 'D', n: 'NewOrderSingle', i: 1, m: [['f', 55, 1]] }],
  components: [],
  groups: [],
  header: [8, 35],
  trailer: [10],
  spec: {
    version: '5.0SP2',
    ep: 309,
    shards: 1,
    sources: [],
  },
  kpi: {
    fields: 3,
    primitives: 3,
    groups: 0,
    codes: 0,
    codeSets: 0,
    messages: 1,
    components: 0,
    lineage: 0,
    aliases: 0,
    alternates: 0,
    versions: 0,
    columns: 0,
    datatypes: 2,
    branches: 1,
    layoutGroups: 0,
    dtypes: [],
    branchSizes: [['', 3]],
    versionList: [],
  },
  projection: {
    columns: [],
    carried: 0,
    call: '',
  },
  frames: [],
}
const model = fixManifest({ index })
const browserModel = model as FixBrowserManifestModel

const inspection = inspectFixText({ model, text: '8=FIX.4.4|35=D|55=AAPL|' })
expectType<FixInspection>(inspection)
expectType<string>(inspection.tree[0].value)

const composition = composeFixText({
  model,
  messageType: 'D',
  entries: [[55, 'AAPL'], { key: 'Symbol', value: 'MSFT' }],
  separator: '|',
})
expectType<FixComposition>(composition)
expectType<string>(composition.wire)

expectType<HTMLDivElement>(fixRegistrySummary({ model: browserModel }))
expectType<HTMLDivElement>(fixSourceTable({ model: browserModel }))
declare const compactModel: FixManifestModel
// @ts-expect-error Summary requires the generated spec and complete KPI data.
fixRegistrySummary({ model: compactModel })
expectType<HTMLInputElement>(fixFieldExplorer({ model }).input)
expectType<HTMLSelectElement>(fixMessageExplorer({ model }).select)
expectType<HTMLDivElement>(fixProjectionExplorer({ model: browserModel }).element)
expectType<number | null>(
  composeFixText({ model, messageType: 'D', entries: [{ key: 'vendor-key', value: 'x' }] })
    .pairs[3].tag,
)

const frame: FixGeneratedFrame = {
  key: 'order',
  label: 'order',
  line: composition.wire,
  root: 'D',
  mime: 'text/fix',
  msgtype: null,
  direction: null,
  branch: '',
  size: 0,
  columns: [],
  arrivals: [],
  unmapped: [],
  lift: [],
  anomalies: [],
  digest: '',
  ticker: null,
  clock: null,
  partition: null,
  emitted: '',
  call: '',
}
const decoder = fixDecodeWorkbench({
  model,
  frames: [frame],
  decode: async (_text, reading) => reading.messageType,
})
expectType<FixInspection | null>(decoder.reading)
expectType<Promise<import('../../ui/index.mjs').ContentValue | null>>(decoder.decode())
expectType<FixGeneratedFrame | null>(fixFrameGallery({ model, frames: [frame] }).select(0))

const encoder = fixEncodeWorkbench({
  model,
  values: new Map([[55, 'AAPL']]),
  onDecode: (wire, draft) => expectType<string>(wire + draft.checksum),
  validate: (_wire, draft) => draft.display,
})
expectType<FixComposition | null>(encoder.draft)
expectType<Promise<import('../../ui/index.mjs').ContentValue | null>>(encoder.validate())

const field: FieldDocument = {
  name: 'symbol',
  dtype: { type: 'utf8' },
  nullable: true,
  metadata: { 'fix:tag': '55' },
}
const actions: FixRegistryActions = {
  list: () => [field],
  insert: (_field) => null,
  update: async (_field) => {},
  removeById: (_id) => null,
}
const registry = fixRegistryEditor({ fields: [field], actions })
expectType<HTMLTextAreaElement | null>(registry.editor)
expectType<Promise<boolean>>(registry.insert())
expectType<boolean>(registry.stale)
expectType<readonly FieldDocument[] | null>(registry.fields)
expectType<boolean>(registry.stale)
expectType<readonly FieldDocument[] | null>(fixRegistryEditor({ model }).fields)
// @ts-expect-error A registry editor must choose compact-model or full-document mode.
fixRegistryEditor({})
// @ts-expect-error Connected mode requires canonical Field documents.
fixRegistryEditor({ actions })
// @ts-expect-error Compact and canonical registry inputs are mutually exclusive.
fixRegistryEditor({ model, fields: [field] })
