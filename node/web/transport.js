// The replay's controls: play and pause, a step one book either way, a step
// to the next or previous source instant (skipping grid ticks - the app
// decides which books those are), first and last, a scrubber over the book
// index, the speed, the snapshot grid, and a jump to an instant typed as
// decimal nanoseconds. Each control emits `ygg:transport` with the command id
// `shortcuts.js` names; the scrubber and the speed add `transport.seek` and
// `transport.speed`, which have a value and no chord. The instant is shown as
// text with every digit, never as a number.

import { Component, element } from './component.js'
import { formatInstant, parseInstant } from './instant.js'

export const SPEEDS = Object.freeze([0.25, 0.5, 1, 2, 4, 8, 16])

const STEPS = [
  ['transport.first', '⏮', 'First book'],
  ['transport.backSource', '⇤', 'Previous source instant'],
  ['transport.back', '‹', 'Previous book'],
  ['transport.toggle', '▶', 'Play'],
  ['transport.forward', '›', 'Next book'],
  ['transport.forwardSource', '⇥', 'Next source instant'],
  ['transport.last', '⏭', 'Last book'],
]

let serial = 0

export class Transport extends Component {
  render() {
    const id = `ygg-ui-transport-${++serial}`
    const root = element('section', 'ygg-ui__transport')
    root.setAttribute('aria-label', 'Replay controls')
    const buttons = element('div', 'ygg-ui__transport-buttons')
    buttons.setAttribute('role', 'group')
    buttons.setAttribute('aria-label', 'Step')
    this.buttons = new Map()
    for (const [command, glyph, label] of STEPS) {
      const button = element('button', 'ygg-ui__button ygg-ui__transport-step', glyph)
      button.type = 'button'
      button.dataset.command = command
      button.setAttribute('aria-label', label)
      button.title = label
      buttons.append(button)
      this.buttons.set(command, button)
    }
    // Play and pause is one toggle: pressed while playing.
    this.buttons.get('transport.toggle').setAttribute('aria-pressed', 'false')

    const scrub = element('div', 'ygg-ui__transport-scrub')
    this.range = element('input', 'ygg-ui__transport-range')
    this.range.type = 'range'
    this.range.min = '0'
    this.range.step = '1'
    this.range.id = `${id}-range`
    this.range.setAttribute('aria-label', 'Book')
    this.first = element('span', 'ygg-ui__transport-edge ygg-ui__muted ygg-ui__num')
    this.last = element('span', 'ygg-ui__transport-edge ygg-ui__muted ygg-ui__num')
    const where = element('div', 'ygg-ui__transport-where')
    this.at = element('output', 'ygg-ui__transport-at ygg-ui__num')
    this.at.htmlFor = this.range.id
    this.position = element('span', 'ygg-ui__transport-position ygg-ui__muted')
    this.tick = element('span', 'ygg-ui__transport-tick', 'Grid tick')
    this.tick.hidden = true
    where.append(this.at, this.position, this.tick)
    scrub.append(this.first, this.range, this.last)

    const options = element('div', 'ygg-ui__transport-options')
    const speedLabel = element('label', 'ygg-ui__transport-label', 'Speed ')
    this.speed = element('select', 'ygg-ui__transport-speed')
    for (const speed of SPEEDS) this.speed.append(new Option(`${speed}x`, String(speed)))
    speedLabel.append(this.speed)
    this.grid = element('button', 'ygg-ui__button ygg-ui__transport-grid', 'Grid')
    this.grid.type = 'button'
    this.grid.dataset.command = 'transport.grid'
    this.grid.setAttribute('aria-label', 'Snapshot grid')
    this.grid.setAttribute('aria-pressed', 'false')
    this.buttons.set('transport.grid', this.grid)

    this.jump = element('form', 'ygg-ui__transport-jump')
    this.jump.noValidate = true
    const jumpLabel = element('label', 'ygg-ui__transport-label', 'Jump to ')
    this.jumpInput = element('input', 'ygg-ui__transport-input ygg-ui__num')
    this.jumpInput.type = 'text'
    this.jumpInput.inputMode = 'numeric'
    this.jumpInput.placeholder = 'nanoseconds since the epoch'
    this.jumpInput.setAttribute('aria-invalid', 'false')
    this.error = element('span', 'ygg-ui__transport-error')
    this.error.id = `${id}-error`
    this.error.setAttribute('role', 'alert')
    this.jumpInput.setAttribute('aria-describedby', this.error.id)
    jumpLabel.append(this.jumpInput)
    const go = element('button', 'ygg-ui__button', 'Go')
    go.type = 'submit'
    go.setAttribute('aria-label', 'Jump to the instant')
    this.jump.append(jumpLabel, go, this.error)
    options.append(speedLabel, this.grid, this.jump)

    root.append(buttons, where, scrub, options)
    return root
  }

  onMount() {
    this.listen(this.el, 'click', (event) => {
      const button = event.target.closest?.('button[data-command]')
      if (!button || button.getAttribute('aria-disabled') === 'true') return
      const command = button.dataset.command
      const state = this.state ?? {}
      if (command === 'transport.toggle') this.intent(command, !state.playing)
      else if (command === 'transport.grid') this.intent(command, !state.grid)
      else this.intent(command)
    })
    this.listen(this.range, 'input', () => this.intent('transport.seek', Number(this.range.value)))
    this.listen(this.speed, 'change', () => this.intent('transport.speed', Number(this.speed.value)))
    this.listen(this.jump, 'submit', (event) => {
      event.preventDefault()
      const text = this.jumpInput.value.trim()
      try {
        parseInstant(text)
      } catch (error) {
        this.jumpInput.setAttribute('aria-invalid', 'true')
        this.error.textContent = error.message
        return
      }
      this.jumpInput.setAttribute('aria-invalid', 'false')
      this.error.textContent = ''
      this.intent('transport.jump', text)
    })
  }

  /** Put the caret in the jump field: what the `j` shortcut does. */
  focusJump() {
    this.jumpInput?.focus()
    this.jumpInput?.select()
  }

  intent(command, value) {
    this.emit('ygg:transport', value === undefined ? { command } : { command, value })
  }

  draw(state) {
    const count = Math.max(0, state?.count ?? 0)
    const index = Math.min(Math.max(0, state?.index ?? 0), Math.max(0, count - 1))
    const label = state?.at === null || state?.at === undefined ? 'no book' : formatInstant(state.at)
    this.range.max = String(Math.max(0, count - 1))
    if (this.range.value !== String(index)) this.range.value = String(index)
    this.range.disabled = count === 0
    this.range.setAttribute('aria-valuetext', label)
    this.at.textContent = label
    this.position.textContent = count ? `${index + 1} of ${count}` : 'no books'
    this.first.textContent = state?.first ? formatInstant(state.first) : ''
    this.last.textContent = state?.last ? formatInstant(state.last) : ''
    this.tick.hidden = !state?.isTick
    const atStart = count === 0 || index === 0
    const atEnd = count === 0 || index === count - 1
    for (const command of ['transport.first', 'transport.back', 'transport.backSource']) this.disable(command, atStart)
    for (const command of ['transport.last', 'transport.forward', 'transport.forwardSource']) this.disable(command, atEnd)
    const toggle = this.buttons.get('transport.toggle')
    toggle.textContent = state?.playing ? '⏸' : '▶'
    toggle.setAttribute('aria-label', state?.playing ? 'Pause' : 'Play')
    toggle.title = state?.playing ? 'Pause' : 'Play'
    toggle.setAttribute('aria-pressed', String(Boolean(state?.playing)))
    this.grid.setAttribute('aria-pressed', String(Boolean(state?.grid)))
    const speed = String(state?.speed ?? 1)
    if (SPEEDS.map(String).includes(speed) && this.speed.value !== speed) this.speed.value = speed
  }

  /** A step that cannot move stays focusable and says so, rather than dropping focus. */
  disable(command, disabled) {
    this.buttons.get(command).setAttribute('aria-disabled', String(disabled))
  }
}
