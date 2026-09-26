// The keyboard sheet the `?` shortcut opens: a modal table of every chord in
// `shortcuts.js` as this platform spells it, and what it does.

import { element } from './component.js'
import { Modal } from './modal.js'
import { SHORTCUTS, displayChord } from './shortcuts.js'

export class ShortcutsSheet extends Modal {
  /** `mac` overrides the platform the chords are spelled for; left out, the browser's own. */
  constructor({ title = 'Keyboard shortcuts', shortcuts = SHORTCUTS, mac } = {}) {
    super({ title })
    this.shortcuts = shortcuts
    this.mac = mac
  }

  render() {
    const dialog = super.render()
    dialog.classList.add('ygg-ui__sheet')
    const table = element('table', 'ygg-ui__table')
    table.append(element('caption', 'ygg-ui__sr-only', 'Shortcuts and what they do'))
    const head = element('thead')
    const headRow = element('tr')
    for (const name of ['Keys', 'Action']) {
      const cell = element('th', undefined, name)
      cell.scope = 'col'
      headRow.append(cell)
    }
    head.append(headRow)
    const body = element('tbody')
    for (const [chord, , label] of this.shortcuts) {
      const row = element('tr')
      const keys = element('td')
      keys.append(element('kbd', 'ygg-ui__kbd', displayChord(chord, this.mac)))
      row.append(keys, element('td', undefined, label))
      body.append(row)
    }
    table.append(head, body)
    this.body.append(table)
    return dialog
  }
}
