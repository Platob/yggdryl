/// <reference lib="dom" />

import type { ChoiceList, ContentValue, SearchableList } from './index.mjs'
import type {
  FieldDocument,
  FixDetailDocument,
  FixFieldRecord,
  FixIndexDocument,
  FixManifestModel,
  FixMessageRecord,
} from './yggdryl.mjs'

export interface FixInspectionCheck {
  name: string
  stated: string
  computed: string
  ok: boolean
}

export interface FixInspectionEntry {
  key: string
  value: string
  record: FixFieldRecord | null
  tag: number | null
  occurrences?: FixOccurrence[]
}

export interface FixOccurrence {
  members: FixInspectionEntry[]
}

export interface FixInspection {
  direction: 'SENT' | 'RECV' | null
  separator: string
  body: string
  pairs: FixInspectionEntry[]
  tree: FixInspectionEntry[]
  unknown: number
  messageType: string | null
  version: string | null
  checks: FixInspectionCheck[]
}

export function inspectFixText(config: {
  model: FixManifestModel
  text: string
}): FixInspection

export type FixComposeEntry =
  | readonly [key: string | number, value: string]
  | { key: string | number; value: string }

export interface FixComposedPair {
  key: string
  value: string
  tag: number | null
}

export interface FixComposition {
  /** The exact frame with SOH delimiters, suitable for copying or transport. */
  wire: string
  /** The same frame rendered with the requested visible separator. */
  display: string
  separator: string
  bodyLength: number
  checksum: string
  byteLength: number
  pairs: FixComposedPair[]
  conflicts: string[]
  message: FixMessageRecord | null
}

export function composeFixText(config: {
  model: FixManifestModel
  messageType: string
  beginString?: string
  entries: Iterable<FixComposeEntry>
  separator?: string
}): FixComposition

export type FixDetailLoader =
  | FixDetailDocument
  | null
  | (() => FixDetailDocument | null | PromiseLike<FixDetailDocument | null>)

export interface FixManifestKpi {
  fields: number
  primitives: number
  groups: number
  codes: number
  codeSets: number
  messages: number
  components: number
  lineage: number
  aliases: number
  alternates: number
  versions: number
  columns: number
  datatypes: number
  branches: number
  layoutGroups: number
  dtypes: readonly (readonly [name: string, count: number])[]
  branchSizes: readonly (readonly [branch: string, count: number])[]
  versionList: readonly string[]
}

export interface FixManifestSource {
  id: string
  url: string
  format: string
  version: string
  sha256: string
  license: string
}

export interface FixManifestSpecification {
  version: string
  ep: number
  shards: number
  sources: readonly FixManifestSource[]
}

export interface FixManifestProjection {
  columns: readonly FixProjectionColumn[]
  carried: number
  call: string
}

/** The complete generated documentation manifest, beyond the compact lookup core. */
export interface FixBrowserIndexDocument extends FixIndexDocument {
  spec: FixManifestSpecification
  kpi: FixManifestKpi
  projection: FixManifestProjection
  frames: readonly FixGeneratedFrame[]
}

export type FixBrowserManifestModel = Omit<FixManifestModel, 'data'> & {
  data: FixBrowserIndexDocument
}

export function fixRegistrySummary(config: { model: FixBrowserManifestModel }): HTMLDivElement
export function fixSourceTable(config: { model: FixBrowserManifestModel }): HTMLDivElement

export interface FixFieldExplorer extends SearchableList<FixFieldRecord> {
  kind: HTMLSelectElement
  dtype: HTMLSelectElement
}

export function fixFieldExplorer(config: {
  model: FixManifestModel
  loadDetails?: FixDetailLoader
  renderDetailsError?: (error: unknown) => ContentValue
  pageSize?: number
}): FixFieldExplorer

export interface FixMessageExplorer {
  element: HTMLDivElement
  input: HTMLInputElement
  select: HTMLSelectElement
  summary: HTMLDivElement
  view: HTMLDivElement
  refresh(): FixMessageRecord | null
  show(): FixMessageRecord | null
}

export function fixMessageExplorer(config: {
  model: FixManifestModel
}): FixMessageExplorer

export interface FixProjectionColumn {
  c: string
  t: number
  n: string
  y: string
  x: string
}

export function fixProjectionExplorer(config: {
  model: FixBrowserManifestModel
  pageSize?: number
}): SearchableList<FixProjectionColumn>

export interface FixFrameColumn {
  t: number
  n: string
  y: string
  v: string
}

export interface FixGeneratedFrame {
  key: string
  label: string
  line: string
  root: string
  mime: string
  msgtype: string | null
  direction: string | null
  branch: string
  size: number
  columns: readonly FixFrameColumn[]
  arrivals: readonly (readonly [tag: string, key: string, value: string])[]
  unmapped: readonly string[]
  lift: readonly (readonly [facet: string, value: string, source: string])[]
  anomalies: readonly string[]
  digest: string
  ticker: string | null
  clock: string | null
  partition: string | null
  emitted: string
  call: string
  [key: string]: unknown
}

export type FixDecodeCallback = (
  text: string,
  inspection: FixInspection,
) => ContentValue | PromiseLike<ContentValue>

export interface FixDecodeWorkbench {
  element: HTMLDivElement
  input: HTMLTextAreaElement
  summary: HTMLDivElement
  view: HTMLDivElement
  host: HTMLDivElement
  error: HTMLParagraphElement
  readonly reading: FixInspection | null
  setValue(text: string): FixInspection | null
  inspect(): FixInspection | null
  decode(): Promise<ContentValue | null>
}

export function fixDecodeWorkbench(config: {
  model: FixManifestModel
  loadDetails?: FixDetailLoader
  renderDetailsError?: (error: unknown) => ContentValue
  frames?: readonly FixGeneratedFrame[]
  value?: string
  decode?: FixDecodeCallback
  pageSize?: number
}): FixDecodeWorkbench

export interface FixFrameGallery {
  element: HTMLDivElement
  choices: ChoiceList<FixGeneratedFrame> | null
  view: HTMLDivElement
  select(index: number): FixGeneratedFrame | null
}

export function fixFrameGallery(config: {
  model: FixManifestModel
  frames?: readonly FixGeneratedFrame[]
}): FixFrameGallery

export type FixDraftValues =
  | Readonly<Record<string, string>>
  | Iterable<readonly [tag: number, value: string]>

export type FixValidateCallback = (
  wire: string,
  draft: FixComposition,
) => ContentValue | PromiseLike<ContentValue>

export interface FixEncodeWorkbench {
  element: HTMLDivElement
  messageSelect: HTMLSelectElement
  scopeSelect: HTMLSelectElement
  separatorSelect: HTMLSelectElement
  form: HTMLFormElement
  output: HTMLDivElement
  status: HTMLDivElement
  error: HTMLParagraphElement
  readonly draft: FixComposition | null
  setMessageType(messageType: string): FixComposition | null
  setValue(key: string | number, value: string): FixComposition | null
  compose(): FixComposition
  validate(): Promise<ContentValue | null>
}

export function fixEncodeWorkbench(config: {
  model: FixManifestModel
  loadDetails?: FixDetailLoader
  renderDetailsError?: (error: unknown) => ContentValue
  messageType?: string
  beginString?: string
  values?: FixDraftValues
  onDecode?: (wire: string, draft: FixComposition) => void
  validate?: FixValidateCallback
}): FixEncodeWorkbench

export interface FixRegistryActions {
  list(): Iterable<FieldDocument> | PromiseLike<Iterable<FieldDocument>>
  insert(field: FieldDocument): unknown | PromiseLike<unknown>
  update(field: FieldDocument): unknown | PromiseLike<unknown>
  removeById(id: string): unknown | PromiseLike<unknown>
}

export interface FixRegistryEditor {
  element: HTMLDivElement
  readonly listing: SearchableList<FieldDocument | FixFieldRecord>
  editor: HTMLTextAreaElement | null
  status: HTMLParagraphElement | null
  error: HTMLParagraphElement | null
  readonly fields: readonly FieldDocument[] | null
  readonly selectedId: string | null
  /** True when a mutation committed but the authoritative reload failed. */
  readonly stale: boolean
  select(id: string): boolean
  refresh(): Promise<readonly FieldDocument[] | null>
  /** True once the host mutation resolves, even when `stale` reports a reload failure. */
  insert(): Promise<boolean>
  /** True once the host mutation resolves, even when `stale` reports a reload failure. */
  update(): Promise<boolean>
  /** True once the host mutation resolves, even when `stale` reports a reload failure. */
  remove(): Promise<boolean>
}

export type FixRegistryEditorConfig =
  | {
      /** Compact generated manifest used only for static, read-only exploration. */
      model: FixManifestModel
      fields?: never
      actions?: never
      loadDetails?: FixDetailLoader
      renderDetailsError?: (error: unknown) => ContentValue
      pageSize?: number
    }
  | {
      model?: never
      fields: Iterable<FieldDocument>
      actions?: undefined
      loadDetails?: never
      renderDetailsError?: never
      pageSize?: number
    }
  | {
      model?: never
      fields: Iterable<FieldDocument>
      actions: FixRegistryActions
      loadDetails?: never
      renderDetailsError?: never
      pageSize?: number
    }

export function fixRegistryEditor(config: FixRegistryEditorConfig): FixRegistryEditor
