/// <reference lib="dom" />

import type { SearchIndex } from './index.mjs'

export interface DataTypeDocument {
  type: string
  fields?: readonly (FieldDocument | { field: FieldDocument; type_id?: number })[]
  field?: FieldDocument
  entries?: FieldDocument
  run_ends?: FieldDocument
  values?: FieldDocument
  [key: string]: unknown
}

export interface FieldDocument {
  name: string
  dtype: DataTypeDocument | string
  nullable: boolean
  dictionary_id?: string | number
  dictionary_is_ordered?: boolean
  metadata?: Readonly<Record<string, string>>
  [key: string]: unknown
}

export function dataType(config: { value: string; className?: string }): HTMLElement
export function fieldDocument(config: {
  document: FieldDocument
  open?: boolean
  className?: string
}): HTMLUListElement

export type FixMemberKind = 'f' | 'c' | 'g'
export type FixMember = readonly [FixMemberKind, number, 0 | 1]

export interface FixFieldRecord {
  t: number
  n: string
  y: string
  d?: string
  b?: string
  x?: string
  a?: readonly string[]
  g?: readonly number[]
  k?: string
  m?: readonly number[]
  c?: number
  s?: string
  e?: string
}

export interface FixMessageRecord {
  y: string
  n: string
  i: number
  m: readonly FixMember[]
}

export interface FixComponentRecord {
  n: string
  i: number
  m: readonly FixMember[]
}

export interface FixGroupRecord extends FixComponentRecord {
  t: number
}

export interface FixWireGroup {
  n: string
  m: readonly number[]
}

export interface FixIndexDocument {
  wire: Readonly<Record<string, FixWireGroup>>
  fields: readonly FixFieldRecord[]
  messages: readonly FixMessageRecord[]
  components: readonly FixComponentRecord[]
  groups: readonly FixGroupRecord[]
  header: readonly number[]
  trailer: readonly number[]
  [key: string]: unknown
}

export type FixCode = readonly [string, string, string, string, number, string]
export type FixLineage = readonly [string, string, string, string]
export interface FixFieldDetail {
  c?: readonly FixCode[]
  l?: readonly FixLineage[]
}
export type FixDetailDocument = Readonly<Record<string, FixFieldDetail>>

export interface FixLayoutTag {
  tag: number
  required: boolean
  depth: number
  group?: FixGroupRecord
}

export interface FixManifestModel {
  data: FixIndexDocument
  details: FixDetailDocument | null
  byTag: Map<number, FixFieldRecord>
  byName: Map<string, FixFieldRecord>
  groupsByTag: Map<number, FixWireGroup>
  messages: Map<string, FixMessageRecord>
  components: Map<number, FixComponentRecord>
  layoutGroups: Map<number, FixGroupRecord>
  header: Set<number>
  trailer: Set<number>
  searchIndex: SearchIndex<FixFieldRecord>
  field(key: string | number): FixFieldRecord | null
  groupsOf(msgtype: string | null): Map<number, FixWireGroup>
  carriers(tag: number): FixMessageRecord[]
  layoutTags(
    message: string | FixMessageRecord,
    seen?: Set<string>,
  ): FixLayoutTag[]
  layoutTree(members: Iterable<FixMember>): HTMLUListElement
  detail(
    record: FixFieldRecord | string | number,
    source?: FixDetailDocument | null,
  ): FixFieldDetail | null
  titleOf(record: FixFieldRecord): string
}

export function fixManifest(config: {
  index: FixIndexDocument
  details?: FixDetailDocument | null
}): FixManifestModel
