// The theme switch: until the person chooses, `prefers-color-scheme` decides
// and `<html>` carries no `data-theme`; a choice sets `data-theme` and is
// remembered in `localStorage['ygg-ui-theme']` for the next load, when
// storage allows. The button is pressed while the page is dark. Emits
// `ygg:theme` `{ theme }`.

import { Component, element } from './component.js'

export const STORAGE_KEY = 'ygg-ui-theme'
const THEMES = new Set(['dark', 'light'])

function stored() {
  try {
    const value = globalThis.localStorage?.getItem(STORAGE_KEY)
    return THEMES.has(value) ? value : null
  } catch {
    return null
  }
}

function remember(theme) {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, theme)
  } catch {
    // Storage refused (a private window, blocked site data): the choice lasts this page.
  }
}

export class ThemeToggle extends Component {
  constructor({ label = 'Dark theme' } = {}) {
    super({ label })
  }

  render() {
    const button = element('button', 'ygg-ui__button ygg-ui__theme-toggle', this.props.label)
    button.type = 'button'
    button.setAttribute('aria-pressed', 'false')
    return button
  }

  onMount() {
    this.media = matchMedia('(prefers-color-scheme: dark)')
    const remembered = stored()
    if (remembered) document.documentElement.dataset.theme = remembered
    this.listen(this.el, 'click', () => this.toggle())
    this.listen(this.media, 'change', () => this.reflect())
    this.reflect()
  }

  /** The theme the page shows: the chosen one, else the preference's. */
  get theme() {
    const chosen = document.documentElement.dataset.theme
    if (THEMES.has(chosen)) return chosen
    return (this.media?.matches ?? true) ? 'dark' : 'light'
  }

  /** Switch to the other theme, remember it, and say so. */
  toggle() {
    const theme = this.theme === 'dark' ? 'light' : 'dark'
    document.documentElement.dataset.theme = theme
    remember(theme)
    this.reflect()
    this.emit('ygg:theme', { theme })
  }

  reflect() {
    if (this.el) this.el.setAttribute('aria-pressed', String(this.theme === 'dark'))
  }
}
