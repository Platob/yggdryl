'use strict'

// Option properties given by name: the plain object every options-taking door
// takes beside an options value, and the warning a name no property owns
// raises.
//
// Each property is set on a copy of the options by that property's own setter,
// so a name is validated exactly as an assignment is. A name no setter owns is
// not an error: it is skipped with an `UnknownPropertyWarning` - the process
// warning Python's `yggdryl.UnknownPropertyWarning` is - naming it and the
// closest property there is, so a typo is heard without failing the call.

const WARNING = 'UnknownPropertyWarning'
const CODE = 'YGGDRYL_UNKNOWN_PROPERTY'

// The settable accessors a native options class declares, read off its own
// prototype once: the list is the class's, never a second copy of it.
const settable = new WeakMap()

function settableProperties(OptionsClass) {
  let names = settable.get(OptionsClass)
  if (names === undefined) {
    names = new Set()
    for (let prototype = OptionsClass.prototype; prototype && prototype !== Object.prototype; ) {
      for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(prototype))) {
        if (typeof descriptor.set === 'function') names.add(name)
      }
      prototype = Object.getPrototypeOf(prototype)
    }
    settable.set(OptionsClass, names)
  }
  return names
}

// Levenshtein distance over characters, one row at a time.
function editDistance(left, right) {
  let previous = Array.from({ length: right.length + 1 }, (_, index) => index)
  for (let row = 0; row < left.length; row += 1) {
    const current = [row + 1]
    for (let column = 0; column < right.length; column += 1) {
      current.push(
        Math.min(
          previous[column] + (left[row] === right[column] ? 0 : 1),
          previous[column + 1] + 1,
          current[column] + 1,
        ),
      )
    }
    previous = current
  }
  return previous[right.length]
}

// The candidate closest to `name` by edit distance, when one is close enough
// to be the name meant: within a third of its length, and at least one edit.
function closest(name, candidates) {
  const limit = Math.max(1, Math.floor(name.length / 3))
  const folded = name.toLowerCase()
  let best
  for (const candidate of candidates) {
    const distance = editDistance(folded, candidate.toLowerCase())
    if (distance > limit) continue
    if (
      best === undefined ||
      distance < best.distance ||
      (distance === best.distance && candidate < best.candidate)
    ) {
      best = { candidate, distance }
    }
  }
  return best?.candidate
}

// Each name is warned about once per process, as a warning Node raises itself
// is: a loop repeating one typo is heard once, not once per iteration.
const warned = new Set()

// Warn that `name` names no settable property of `owner`, suggesting the
// closest of `candidates`.
function warnUnknownProperty(owner, name, candidates) {
  let message = `${owner} has no settable property '${name}'; it is ignored`
  const suggestion = closest(name, candidates)
  if (suggestion !== undefined) message += `; did you mean '${suggestion}'?`
  if (warned.has(message)) return
  warned.add(message)
  process.emitWarning(message, { type: WARNING, code: CODE })
}

// Whether `name` is a settable property of `OptionsClass`, warning when it is
// not: the one check every property bag landing on a native options class
// runs, before any value is set.
function checkProperty(OptionsClass, owner, name) {
  const names = settableProperties(OptionsClass)
  if (names.has(name)) return true
  warnUnknownProperty(owner, name, names)
  return false
}

// A plain object is a set of option properties rather than an options value.
function isPropertyBag(value) {
  return value !== null && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype
}

// `settings` with each of `properties` set by its own setter on a copy - the
// same object when nothing is set. An `undefined` value is skipped, the
// project's spelling for an argument that was not given; a name no setter of
// `OptionsClass` owns is skipped with a warning, whatever its value.
function withProperties(settings, OptionsClass, owner, properties) {
  const entries = settableEntries(OptionsClass, owner, properties)
  if (entries.length === 0) return settings
  const copy = settings.clone()
  for (const [name, value] of entries) copy[name] = value
  return copy
}

// `target` itself with each of `properties` set by its own setter: what a
// constructor taking its properties by name does to the value it just built.
function setProperties(target, OptionsClass, owner, properties) {
  for (const [name, value] of settableEntries(OptionsClass, owner, properties)) target[name] = value
  return target
}

// The `[name, value]` pairs of a bag a setter takes, warned about and skipped
// where none does.
function settableEntries(OptionsClass, owner, properties) {
  if (!isPropertyBag(properties)) {
    throw new TypeError(
      'expected option properties as a plain object beside one options value, got ' +
        (properties === null ? 'null' : typeof properties),
    )
  }
  const entries = []
  for (const [name, value] of Object.entries(properties)) {
    if (checkProperty(OptionsClass, owner, name) && value !== undefined) entries.push([name, value])
  }
  return entries
}

module.exports = {
  checkProperty,
  closest,
  isPropertyBag,
  setProperties,
  settableProperties,
  warnUnknownProperty,
  withProperties,
}
