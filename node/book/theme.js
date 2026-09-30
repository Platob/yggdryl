// The colour scheme of the book display: system, light or dark.
//
// The scheme is one word stored under `STORAGE_KEY` in `localStorage` and
// stamped on `<html>` as `data-theme`; `theme.css` reads the stamp, and the
// system scheme is the absence of one. Nothing here touches the document at
// import time, so the module loads under Node and every function takes the
// root and the storage it works on as an argument with a browser default.

export const THEMES = Object.freeze(['system', 'light', 'dark'])

export const STORAGE_KEY = 'yggdryl-book-theme'

/** The `localStorage` of the page, or `null` where there is none or it throws. */
function defaultStorage() {
  try {
    return globalThis.localStorage ?? null
  } catch {
    return null
  }
}

/** Whether the operating system asks for a dark scheme right now. */
export function systemPrefersDark(window = globalThis.window) {
  try {
    return window?.matchMedia?.('(prefers-color-scheme: dark)')?.matches === true
  } catch {
    return false
  }
}

/** The scheme name after `name` in the cycle system, light, dark. */
export function nextTheme(name) {
  const index = THEMES.indexOf(name)
  return THEMES[(index + 1) % THEMES.length]
}

/** The stored scheme name, `system` when nothing valid is stored. */
export function readTheme(storage = defaultStorage()) {
  try {
    const stored = storage?.getItem(STORAGE_KEY)
    return THEMES.includes(stored) ? stored : 'system'
  } catch {
    return 'system'
  }
}

/** Store a scheme name; `system` clears the entry. Storage failures are ignored. */
export function writeTheme(name, storage = defaultStorage()) {
  try {
    if (name === 'system') storage?.removeItem(STORAGE_KEY)
    else storage?.setItem(STORAGE_KEY, name)
  } catch {
    // A private window or a blocked origin keeps the choice for this page only.
  }
}

/**
 * The scheme a name resolves to, `light` or `dark`: a stated one is itself,
 * `system` is what the operating system prefers.
 */
export function resolveTheme(name = 'system', prefersDark = systemPrefersDark()) {
  if (name === 'light' || name === 'dark') return name
  return prefersDark ? 'dark' : 'light'
}

/**
 * Stamp `name` on the root element and store it. Returns the resolved scheme.
 * An unknown name is `system`.
 */
export function applyTheme(name, { root = globalThis.document?.documentElement, storage = defaultStorage(), prefersDark } = {}) {
  const theme = THEMES.includes(name) ? name : 'system'
  if (root) {
    if (theme === 'system') delete root.dataset.theme
    else root.dataset.theme = theme
  }
  writeTheme(theme, storage)
  return resolveTheme(theme, prefersDark)
}

/** Move to the next scheme in the cycle, apply it, and answer its name. */
export function toggleTheme(current = readTheme(), options = {}) {
  const next = nextTheme(current)
  applyTheme(next, options)
  return next
}

/** What a toggle button says for a scheme name. */
export function themeLabel(name) {
  switch (name) {
    case 'light':
      return 'Light theme'
    case 'dark':
      return 'Dark theme'
    default:
      return 'System theme'
  }
}
