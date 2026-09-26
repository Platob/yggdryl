// One small store: the whole application state as one frozen object, changed
// by `set`, observed by `subscribe`. Components read what they are given and
// emit intents; only the app writes here.

/** Create a store over `initial`. */
export function createStore(initial = {}) {
  let state = Object.freeze({ ...initial })
  const listeners = new Set()

  function get() {
    return state
  }

  /** Apply a patch object or a function of the current state; notify once. */
  function set(patch) {
    const next = typeof patch === 'function' ? patch(state) : patch
    if (next === null || typeof next !== 'object') {
      throw new TypeError('expected a patch object or a function answering one')
    }
    const previous = state
    let changed = false
    for (const key of Object.keys(next)) {
      if (!Object.is(previous[key], next[key])) {
        changed = true
        break
      }
    }
    if (!changed) return state
    state = Object.freeze({ ...previous, ...next })
    for (const listener of [...listeners]) listener(state, previous)
    return state
  }

  /** Observe every change; answers the unsubscribe function. */
  function subscribe(listener) {
    if (typeof listener !== 'function') throw new TypeError('expected a listener function')
    listeners.add(listener)
    return () => listeners.delete(listener)
  }

  /** Observe one derived value, called only when it changes (`Object.is`). */
  function select(selector, listener) {
    let held = selector(state)
    return subscribe((next) => {
      const value = selector(next)
      if (!Object.is(value, held)) {
        held = value
        listener(value, next)
      }
    })
  }

  return { get, set, subscribe, select }
}
