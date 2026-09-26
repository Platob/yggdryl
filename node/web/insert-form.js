// A form to insert one event into a scenario. The columns it offers are the
// ones the service served from `MarketData.field()` - never a list of its own
// - each chosen one an input of its own; the kind is one of the served kinds
// and the instant is decimal nanoseconds, checked with `parseInstant` before
// anything is sent. Submitting emits `ygg:insert` `{ kind, currunix, facts }`
// with the facts as typed - a map, a serie or a struct typed as the JSON of
// the value, which is what is sent; every other fact its text - and an empty
// input states nothing. The native package decides the rest, and
// `showRefusal(text)` shows what it refused verbatim, beside the field its
// path names when it names one the form shows.

import { Component, element } from './component.js'
import { parseInstant } from './instant.js'

/** The two columns the form asks for itself rather than as facts. */
const OWN = new Set(['kind', 'currunix'])

/**
 * The datatypes whose value is not one text, by the word the native display
 * opens with: a map (sorted or not, its `keys_sorted` says), a struct, and the
 * five serie layouts. Their facts are typed as JSON.
 */
const STRUCTURED = /^(?:map|struct|serie|serie_view|large_serie|large_serie_view|fixed_size_serie)\(/

/** What a structured fact's input shows before anything is typed. */
function placeholderOf(dtype) {
  if (dtype.startsWith('map(')) return '[["key", "value"]]'
  if (dtype.startsWith('struct(')) return '{"name": "value"}'
  return '["value"]'
}

let serial = 0

export class InsertForm extends Component {
  constructor(props = {}) {
    super(props)
    this.base = `ygg-ui-insert-${++serial}`
    this.fields = new Map()
    this.columns = undefined
    this.kinds = undefined
    this.drawnAt = undefined
    this.atEdited = false
  }

  render() {
    const form = element('form', 'ygg-ui__insert')
    form.noValidate = true
    form.setAttribute('aria-label', 'Insert an event')
    this.kind = element('select', 'ygg-ui__insert-kind')
    this.at = element('input', 'ygg-ui__insert-at ygg-ui__mono')
    this.at.type = 'text'
    this.at.inputMode = 'numeric'
    this.at.autocomplete = 'off'
    this.column = element('select', 'ygg-ui__insert-column')
    this.add = element('button', 'ygg-ui__button ygg-ui__insert-add', 'Add')
    this.add.type = 'button'
    this.add.setAttribute('aria-label', 'Add the chosen column as a fact')
    this.facts = element('fieldset', 'ygg-ui__insert-facts')
    this.facts.append(element('legend', undefined, 'Facts'))
    this.none = element('p', 'ygg-ui__muted ygg-ui__insert-none', 'No fact chosen: add a column above.')
    this.facts.append(this.none)
    this.refusal = element('p', 'ygg-ui__error ygg-ui__insert-refusal')
    this.refusal.setAttribute('role', 'alert')
    this.refusal.hidden = true
    const submit = element('button', 'ygg-ui__button ygg-ui__button--primary ygg-ui__insert-submit', 'Insert')
    submit.type = 'submit'
    const chooser = element('div', 'ygg-ui__insert-row')
    chooser.append(this.labelled('Column', this.column, 'column'), this.add)
    form.append(
      this.labelled('Kind', this.kind, 'kind', true),
      this.labelled('Instant (ns since the epoch)', this.at, 'currunix', true),
      chooser,
      this.facts,
      this.refusal,
      submit,
    )
    return form
  }

  /** A label over its control and the error line that describes it. */
  labelled(text, control, name, owned = false) {
    const id = `${this.base}-${name}`
    control.id = id
    const row = element('div', 'ygg-ui__insert-field')
    const label = element('label', 'ygg-ui__insert-label', text)
    label.htmlFor = id
    row.append(label, control)
    if (owned) {
      const error = element('span', 'ygg-ui__error ygg-ui__field-error')
      error.id = `${id}-error`
      control.setAttribute('aria-describedby', error.id)
      control.setAttribute('aria-invalid', 'false')
      row.append(error)
      this.fields.set(name, { control, error })
    }
    return row
  }

  onMount() {
    this.listen(this.add, 'click', () => this.choose(this.column.value))
    this.listen(this.at, 'input', () => {
      this.atEdited = true
    })
    this.listen(this.facts, 'click', (event) => {
      const remove = event.target.closest?.('[data-remove]')
      if (remove) this.unchoose(remove.dataset.remove)
    })
    this.listen(this.el, 'submit', (event) => {
      event.preventDefault()
      this.submit()
    })
  }

  draw(state) {
    const columns = state?.columns ?? []
    if (columns !== this.columns) {
      this.columns = columns
      const offered = columns.filter((column) => !OWN.has(column.name))
      this.column.replaceChildren(...offered.map((column) => new Option(`${column.name} (${column.dtype})`, column.name)))
      const served = new Set(offered.map((column) => column.name))
      for (const name of [...this.fields.keys()]) if (!OWN.has(name) && !served.has(name)) this.unchoose(name)
      this.syncChooser()
    }
    const kinds = state?.kinds ?? []
    if (kinds !== this.kinds) {
      this.kinds = kinds
      const held = this.kind.value
      this.kind.replaceChildren(...kinds.map((kind) => new Option(kind, kind)))
      if (kinds.includes(held)) this.kind.value = held
      this.kind.disabled = kinds.length === 0
    }
    if (state?.at !== this.drawnAt) {
      this.drawnAt = state?.at
      if (!this.atEdited) this.at.value = state?.at === null || state?.at === undefined ? '' : String(state.at)
    }
  }

  /** Start a new insert: the instant is the one shown again, and follows it until one is typed. */
  reset() {
    this.atEdited = false
    const at = this.state?.at
    if (this.at) this.at.value = at === null || at === undefined ? '' : String(at)
  }

  /** Add an input for column `name`, once. */
  choose(name) {
    const column = (this.columns ?? []).find((held) => held.name === name)
    if (!column || OWN.has(name) || this.fields.has(name)) return
    const id = `${this.base}-fact-${name}`
    const row = element('div', 'ygg-ui__insert-field ygg-ui__insert-fact')
    row.dataset.fact = name
    const label = element('label', 'ygg-ui__insert-label')
    label.htmlFor = id
    label.append(element('span', 'ygg-ui__mono', name), ' ', element('span', 'ygg-ui__muted ygg-ui__insert-dtype', column.dtype))
    if (column.nullable === false) label.append(' ', element('span', 'ygg-ui__muted', 'required'))
    const input = element('input', 'ygg-ui__mono')
    input.type = 'text'
    input.id = id
    input.autocomplete = 'off'
    if (STRUCTURED.test(column.dtype)) {
      input.dataset.json = ''
      input.spellcheck = false
      input.placeholder = placeholderOf(column.dtype)
    }
    const error = element('span', 'ygg-ui__error ygg-ui__field-error')
    error.id = `${id}-error`
    input.setAttribute('aria-describedby', error.id)
    input.setAttribute('aria-invalid', 'false')
    const remove = element('button', 'ygg-ui__button', '×')
    remove.type = 'button'
    remove.dataset.remove = name
    remove.setAttribute('aria-label', `Remove ${name}`)
    row.append(label, input, remove, error)
    this.facts.append(row)
    this.fields.set(name, { control: input, error, row })
    this.syncChooser()
    input.focus()
  }

  unchoose(name) {
    const field = this.fields.get(name)
    if (!field || OWN.has(name)) return
    field.row.remove()
    this.fields.delete(name)
    this.syncChooser()
  }

  /** A chosen column is offered no second time; the chooser moves to the next one free. */
  syncChooser() {
    for (const option of this.column.options) option.disabled = this.fields.has(option.value)
    if (this.column.selectedOptions[0]?.disabled) {
      const free = [...this.column.options].find((option) => !option.disabled)
      if (free) this.column.value = free.value
    }
    this.add.disabled = ![...this.column.options].some((option) => !option.disabled)
    this.none.hidden = [...this.fields.keys()].some((name) => !OWN.has(name))
  }

  submit() {
    this.showRefusal(null)
    const currunix = this.at.value.trim()
    try {
      parseInstant(currunix)
    } catch (error) {
      this.mark(this.fields.get('currunix'), error.message)
      return
    }
    const facts = {}
    for (const [name, { control }] of this.fields) {
      if (OWN.has(name) || control.value === '') continue
      if (!('json' in control.dataset)) {
        facts[name] = control.value
        continue
      }
      try {
        facts[name] = JSON.parse(control.value)
      } catch (error) {
        const dtype = this.columns.find((column) => column.name === name)?.dtype ?? ''
        const shape = dtype.startsWith('map(') ? 'a map' : dtype.startsWith('struct(') ? 'a struct' : 'a serie'
        this.showRefusal(`${name}: expected JSON for ${shape}, as ${placeholderOf(dtype)}: ${error.message}`, { beside: false })
        return
      }
    }
    this.emit('ygg:insert', { kind: this.kind.value, currunix, facts })
  }

  /**
   * Show a refusal verbatim: beside the field its path names - `$.price`,
   * `$.operation.price` or `$.operations[1].kind` for an operation, and any
   * path below one of them - else on the form's own line, where `beside:
   * false` puts it too. `null` clears it.
   */
  showRefusal(text, { beside = true } = {}) {
    for (const field of this.fields.values()) {
      field.control.setAttribute('aria-invalid', 'false')
      field.error.textContent = ''
    }
    this.refusal.textContent = ''
    this.refusal.hidden = true
    if (text === null || text === undefined || text === '') return
    const field = beside ? this.fieldNamed(String(text)) : null
    if (field) this.mark(field, String(text))
    else {
      this.refusal.textContent = String(text)
      this.refusal.hidden = false
    }
  }

  /** The field a refusal's path names: its first column, after the operation it is on when it names one. */
  fieldNamed(text) {
    for (const [, path] of text.matchAll(/\$((?:\.[A-Za-z_]\w*|\[\d+\])*)/g)) {
      const name = /^(?:\.operations\[\d+\]|\.operation)?\.([A-Za-z_]\w*)/.exec(path)?.[1]
      if (name && this.fields.has(name)) return this.fields.get(name)
    }
    return null
  }

  /** Mark a field refused and put focus on it, so the refusal describing it is read once, with it. */
  mark(field, text) {
    field.control.setAttribute('aria-invalid', 'true')
    field.error.textContent = text
    field.control.focus()
  }
}
