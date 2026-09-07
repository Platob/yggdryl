/// <reference lib="dom" />

export type Content = Node | string | number | boolean
export type ContentValue = Content | readonly ContentValue[] | null | undefined

export function make<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string | null,
  text?: string | number | boolean,
): HTMLElementTagNameMap[K]

export interface CodeConfig {
  text: string | number | boolean
  className?: string
}
export function code(config: CodeConfig): HTMLElement

export interface CardConfig {
  value: ContentValue
  label: ContentValue
  note?: ContentValue
  className?: string
}
export function card(config: CardConfig): HTMLDivElement
export function cardRow(config: {
  cards: readonly CardConfig[]
  className?: string
}): HTMLDivElement

export interface PanelConfig {
  title: ContentValue
  aside?: ContentValue
  open?: boolean
  render?: (body: HTMLDivElement) => ContentValue | void
  className?: string
  summaryClassName?: string
  bodyClassName?: string
}
export interface Panel {
  element: HTMLDetailsElement
  summary: HTMLElement
  body: HTMLDivElement
  fill(): HTMLDivElement
}
export function panel(config: PanelConfig): Panel

export interface FactRow {
  label: string
  value: ContentValue
  format?: FactFormatter
}
export type FactFormatter = (
  value: Exclude<ContentValue, null | undefined>,
  row: FactRow,
  index: number,
) => ContentValue
export function facts(config: {
  rows: readonly FactRow[]
  format?: FactFormatter
  className?: string
}): HTMLTableElement

export type GridFormatter<T> = (
  value: Exclude<ContentValue, null | undefined>,
  row: T,
  rowIndex: number,
  columnIndex: number,
) => ContentValue
export interface GridColumnConfig<T> {
  label: ContentValue
  key?: keyof T
  get?: (row: T, rowIndex: number, columnIndex: number) => ContentValue
  format?: GridFormatter<T>
  rowHeader?: boolean
  className?: string
  headerClassName?: string
}
export type GridColumn<T> = string | GridColumnConfig<T>
export function grid<T>(config: {
  columns: readonly GridColumn<T>[]
  rows: readonly T[]
  empty?: string
  className?: string
}): HTMLDivElement

export type PillKind =
  | 'accent'
  | 'success'
  | 'info'
  | 'warning'
  | 'quiet'
  | 'required'
  | 'group'
  | 'unknown'
  | 'codes'
  | 'branch'
  | 'danger'
export function pill(config: {
  text: ContentValue
  kind?: PillKind
  className?: string
}): HTMLSpanElement

export function control<T extends HTMLElement>(config: {
  id: string
  label: ContentValue
  node: T
  className?: string
}): HTMLDivElement

export interface InputConfig {
  type?: string
  value?: string | number | boolean
  placeholder?: string
  name?: string
  autocomplete?: string
  disabled?: boolean
  ariaLabel?: string
  wide?: boolean
  className?: string
  onInput?: (event: Event) => void
  onChange?: (event: Event) => void
}
export function input(config?: InputConfig): HTMLInputElement
export function search(config?: Omit<InputConfig, 'type'>): HTMLInputElement

export interface SelectOption {
  value: string | number
  label: string | number | boolean
  disabled?: boolean
  selected?: boolean
}
export interface SelectConfig {
  options?: readonly SelectOption[]
  value?: string | number
  name?: string
  disabled?: boolean
  ariaLabel?: string
  wide?: boolean
  className?: string
  onChange?: (event: Event) => void
}
export function select(config?: SelectConfig): HTMLSelectElement

export interface TextareaConfig extends Omit<InputConfig, 'type' | 'wide'> {
  rows?: number
  spellcheck?: boolean
  wrap?: string
}
export function textarea(config?: TextareaConfig): HTMLTextAreaElement

export type ButtonKind = 'primary' | 'danger' | 'chip' | (string & {})
export interface ButtonConfig {
  label: ContentValue
  kind?: ButtonKind
  type?: 'button' | 'submit' | 'reset'
  pressed?: boolean
  disabled?: boolean
  onClick?: (event: MouseEvent) => void
  ariaLabel?: string
  className?: string
}
export function button(config: ButtonConfig): HTMLButtonElement

export type ModalCloseReason =
  | 'button'
  | 'backdrop'
  | 'escape'
  | 'api'
  | 'native'
  | (string & {})
export interface ModalConfig {
  title: ContentValue
  content?: ContentValue
  actions?: ContentValue
  closeLabel?: ContentValue
  closeOnEscape?: boolean
  closeOnBackdrop?: boolean
  initialFocus?: HTMLElement
  onOpen?: () => void
  onClose?: (reason: ModalCloseReason) => void
  className?: string
  surfaceClassName?: string
  bodyClassName?: string
}
export interface Modal {
  element: HTMLDialogElement
  surface: HTMLDivElement
  body: HTMLDivElement
  closeButton: HTMLButtonElement
  readonly opened: boolean
  open(): void
  close(reason?: ModalCloseReason): void
}
export function modal(config: ModalConfig): Modal

export interface ChoiceList<T> {
  element: HTMLDivElement
  buttons: HTMLButtonElement[]
  readonly selectedIndex: number
  select(index: number): void
}
export function choiceList<T>(config: {
  items: Iterable<T>
  render: (item: T, index: number) => ContentValue
  onSelect?: (item: T, index: number) => void
  selected?: number
  className?: string
}): ChoiceList<T>

export interface WireBlock {
  element: HTMLDivElement
  body: HTMLPreElement
  button: HTMLButtonElement
  copy(): Promise<boolean>
}
export function wireBlock(config: {
  display: string
  copy?: string
  duration?: number
  labels?: {
    copy?: string
    copied?: string
    unavailable?: string
  }
  className?: string
}): WireBlock

export function callBlock(config: {
  code: string
  answer?: ContentValue
  className?: string
}): HTMLDivElement
export type NoteKind = 'accent' | 'success' | 'info' | 'warning' | 'danger'
export function note(config: {
  content: ContentValue
  kind?: NoteKind
  className?: string
}): HTMLParagraphElement

export interface SearchIndexEntry<T> {
  item: T
  haystack: string
}
export interface SearchIndex<T> {
  entries: SearchIndexEntry<T>[]
  size: number
  search(query: string, filter?: (item: T) => boolean): T[]
}
export function createSearchIndex<T>(config: {
  items: Iterable<T>
  haystack: (item: T, index: number) => string
}): SearchIndex<T>

export interface SearchPageState {
  matches: number
  remaining: number
}
export interface SearchableList<T> {
  element: HTMLDivElement
  controls: HTMLDivElement
  input: HTMLInputElement
  count: HTMLParagraphElement
  list: HTMLDivElement
  more: HTMLButtonElement
  refresh(config?: { reset?: boolean }): { matches: number; drawn: number; remaining: number }
  reset(): { matches: number; drawn: number; remaining: number }
}
export function searchableList<T>(config: {
  index: SearchIndex<T>
  renderPage: (items: readonly T[], state: SearchPageState) => ContentValue
  pageSize?: number
  noun?: string
  filter?: (item: T) => boolean
  label?: ContentValue
  id?: string
  placeholder?: string
  query?: string
  controls?: ContentValue
  className?: string
}): SearchableList<T>

export interface TreeDescription<T> {
  label: ContentValue
  aside?: ContentValue
  branch?: boolean
  children?: (item: T) => Iterable<T>
  beforeChildren?: ContentValue | ((item: T) => ContentValue)
  open?: boolean
  className?: string
}
export function tree<T>(config: {
  items: Iterable<T>
  key: (item: T) => string | number
  describe: (item: T) => TreeDescription<T> | null | undefined
  className?: string
}): HTMLUListElement

export interface LazyIndex<T> {
  readonly built: boolean
  get(): T
}
export function createLazyIndex<T>(config: { build: () => T }): LazyIndex<T>

export interface AssetLoader {
  address(name: string): string
  json<T = unknown>(name: string): Promise<T>
  prefetch<T = unknown>(name: string): Promise<T>
  failure(config: { asset: string; error: unknown }): HTMLDivElement
}
export function assetLoader(config: {
  baseURL: string | URL
  assets: Readonly<Record<string, string | URL>>
  regenerate: string
  fetch?: typeof fetch
  schedule?: (run: () => void) => void
}): AssetLoader

export interface DocumentObservable {
  subscribe(run: () => void): void | { unsubscribe?(): void }
}
export function onDocumentReady(config: {
  run: (document: Document) => void
  document?: Document
  navigation?: DocumentObservable
}): () => void
