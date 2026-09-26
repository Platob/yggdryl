// The keyboard map: one chord, one command id. A component reads the command
// and emits the intent; the app decides what it does. `Mod` is Ctrl on
// Windows and Linux and Cmd on macOS, spelled once here.

/** Every chord and the command it names, in the order the shortcut sheet lists them. */
export const SHORTCUTS = Object.freeze([
  ['Space', 'transport.toggle', 'Play or pause'],
  ['ArrowRight', 'transport.forward', 'Step one book forward'],
  ['ArrowLeft', 'transport.back', 'Step one book back'],
  ['Shift+ArrowRight', 'transport.forwardSource', 'Step to the next source instant, skipping grid ticks'],
  ['Shift+ArrowLeft', 'transport.backSource', 'Step to the previous source instant, skipping grid ticks'],
  ['Home', 'transport.first', 'Jump to the first book'],
  ['End', 'transport.last', 'Jump to the last book'],
  [']', 'transport.faster', 'Speed up'],
  ['[', 'transport.slower', 'Slow down'],
  ['g', 'transport.grid', 'Snapshot grid on or off'],
  ['j', 'transport.jump', 'Jump to an instant'],
  ['i', 'scenario.insert', 'Insert an event'],
  ['s', 'scenario.drawer', 'Open the scenarios'],
  ['d', 'scenario.diff', 'Show the diff against the base replay'],
  ['t', 'theme.toggle', 'Switch the theme'],
  ['Mod+k', 'palette.open', 'Open the command palette'],
  ['?', 'help.shortcuts', 'Show this sheet'],
  ['Escape', 'ui.close', 'Close the open panel'],
].map((row) => Object.freeze(row)))

const MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform ?? '')

/** The chord text of a keyboard event: modifiers in a fixed order, then the key. */
export function chordOf(event, mac = MAC) {
  const parts = []
  const mod = mac ? event.metaKey : event.ctrlKey
  if (mod) parts.push('Mod')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey && event.key.length > 1) parts.push('Shift')
  let key = event.key
  if (key === ' ') key = 'Space'
  else if (key.length === 1 && !event.shiftKey) key = key.toLowerCase()
  parts.push(key)
  return parts.join('+')
}

const BY_CHORD = new Map(SHORTCUTS.map(([chord, command]) => [chord, command]))

/** What acts on Space and Enter itself: a button presses, a link follows, a summary opens, a role has its own keys. */
const ACTS = 'button, a[href], summary, [role]'

/**
 * The command a keyboard event names, or `undefined`. Typing in a field names
 * none but Escape and the palette; Space and Enter inside a control that acts
 * on them name none.
 */
export function commandFor(event, mac = MAC) {
  const target = event.target
  const typing =
    target &&
    (target.isContentEditable ||
      /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName ?? '') ||
      target.closest?.('[contenteditable="true"]'))
  const chord = chordOf(event, mac)
  if (typing && chord !== 'Escape' && chord !== 'Mod+k') return undefined
  if ((chord === 'Space' || chord === 'Enter') && target?.closest?.(ACTS)) return undefined
  return BY_CHORD.get(chord)
}

/** The chord shown for a command on this platform: `Ctrl+K` or `⌘K`. */
export function displayChord(chord, mac = MAC) {
  return chord
    .split('+')
    .map((part) => (part === 'Mod' ? (mac ? '⌘' : 'Ctrl') : part))
    .join(mac ? '' : '+')
}
