// The command palette: a modal with one combobox over a listbox of commands,
// each shown with its chord. Typing filters the labels - substring matches
// first, then subsequence matches, each group in list order - the arrows walk
// the list and wrap, Enter or a click chooses. The chosen command is emitted
// as `ygg:command` `{ id }` once the palette has closed and focus is back on
// its opener, so a command that opens another panel starts from there.

import { Modal } from './modal.js'
import { element } from './component.js'
import { SHORTCUTS, displayChord } from './shortcuts.js'

/** Every shortcut but the palette's own, as `{ id, label, chord }`. */
export const COMMANDS = Object.freeze(
  SHORTCUTS.filter(([, id]) => id !== 'palette.open').map(([chord, id, label]) => Object.freeze({ id, label, chord })),
)

/** Whether every character of `query` appears in `text` in order. */
function subsequence(text, query) {
  let at = 0
  for (const character of text) if (character === query[at]) at += 1
  return at === query.length
}

/** The commands whose label matches `query`, ignoring case: substrings first, then subsequences. */
export function fuzzy(commands, query) {
  const wanted = query.trim().toLowerCase()
  if (!wanted) return [...commands]
  const whole = []
  const scattered = []
  for (const command of commands) {
    const label = command.label.toLowerCase()
    if (label.includes(wanted)) whole.push(command)
    else if (subsequence(label, wanted)) scattered.push(command)
  }
  return [...whole, ...scattered]
}

let serial = 0

export class Palette extends Modal {
  constructor({ commands = COMMANDS, title = 'Commands' } = {}) {
    super({ title })
    this.commands = commands
    this.shown = []
    this.active = 0
    this.pending = null
    this.base = `ygg-ui-palette-${++serial}`
  }

  render() {
    const dialog = super.render()
    dialog.classList.add('ygg-ui__palette')
    this.list = element('ul', 'ygg-ui__palette-list')
    this.list.id = `${this.base}-list`
    this.list.setAttribute('role', 'listbox')
    this.list.setAttribute('aria-label', 'Commands')
    this.input = element('input', 'ygg-ui__palette-input')
    this.input.type = 'text'
    this.input.setAttribute('role', 'combobox')
    this.input.setAttribute('aria-label', 'Search commands')
    this.input.setAttribute('aria-expanded', 'true')
    this.input.setAttribute('aria-autocomplete', 'list')
    this.input.setAttribute('aria-controls', this.list.id)
    this.input.autocomplete = 'off'
    this.input.spellcheck = false
    this.empty = element('p', 'ygg-ui__palette-empty ygg-ui__muted', 'No command matches')
    this.empty.hidden = true
    this.body.append(this.input, this.list, this.empty)
    return dialog
  }

  onMount() {
    super.onMount()
    this.listen(this.input, 'input', () => this.filter())
    this.listen(this.input, 'keydown', (event) => {
      const count = this.shown.length
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        if (count) this.activate((this.active + (event.key === 'ArrowDown' ? 1 : -1) + count) % count)
      } else if (event.key === 'Enter') {
        event.preventDefault()
        this.choose(this.active)
      }
    })
    this.listen(this.list, 'click', (event) => {
      const option = event.target.closest?.('[role="option"]')
      if (option) this.choose(Number(option.dataset.index))
    })
  }

  open(opener = document.activeElement) {
    if (this.isOpen) return
    this.input.value = ''
    this.filter()
    super.open(opener)
    this.input.focus()
  }

  filter() {
    this.shown = fuzzy(this.commands, this.input.value)
    this.list.replaceChildren(
      ...this.shown.map((command, index) => {
        const option = element('li', 'ygg-ui__palette-option')
        option.id = `${this.base}-option-${index}`
        option.dataset.index = String(index)
        option.setAttribute('role', 'option')
        option.append(element('span', 'ygg-ui__palette-label', command.label))
        if (command.chord) option.append(element('kbd', 'ygg-ui__kbd', displayChord(command.chord)))
        return option
      }),
    )
    this.empty.hidden = this.shown.length > 0
    this.activate(0)
  }

  activate(index) {
    this.active = index
    const options = this.list.children
    for (let at = 0; at < options.length; at += 1) options[at].setAttribute('aria-selected', String(at === index))
    const option = options[index]
    if (option) {
      this.input.setAttribute('aria-activedescendant', option.id)
      option.scrollIntoView({ block: 'nearest' })
    } else this.input.removeAttribute('aria-activedescendant')
  }

  choose(index) {
    const command = this.shown[index]
    if (!command) return
    this.pending = command.id
    this.close()
  }

  restore() {
    super.restore()
    const id = this.pending
    this.pending = null
    if (id) this.emit('ygg:command', { id })
  }
}
