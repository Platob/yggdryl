// A scenario is a named, saved, reloadable list of inserted events. An event
// is what the server validated and answered: its native `toJSON()` text
// (base64 Arrow IPC, the exact leaf), beside the facts the browser shows and
// the `curruuid` that keys it. The list stays ordered by instant, stable for
// ties, so a replay merges it with the base stream without re-sorting.

import { compareInstant, parseInstant } from './instant.js'

/** A fresh scenario. */
export function createScenario(name) {
  if (typeof name !== 'string' || !name.trim()) throw new TypeError('expected a scenario name')
  return { name: name.trim(), events: [] }
}

/** The instant an inserted event lands at: the instant the walk reads it at, its grid `snapunix`, else its `currunix`. */
export function eventInstant(event) {
  return parseInstant(event.snapunix ?? event.currunix)
}

/** A new scenario with `event` inserted at its instant (after any equal one). */
export function insertEvent(scenario, event) {
  if (!event || typeof event.curruuid !== 'string' || !event.curruuid) {
    throw new TypeError('expected an event the server identified by curruuid')
  }
  if (typeof event.native !== 'string' || !event.native) {
    throw new TypeError('expected the native leaf text the server answered')
  }
  if (scenario.events.some((held) => held.curruuid === event.curruuid)) {
    throw new Error(`event ${event.curruuid} is already in scenario ${scenario.name}`)
  }
  const at = eventInstant(event)
  let index = scenario.events.length
  while (index > 0 && compareInstant(eventInstant(scenario.events[index - 1]), at) > 0) index -= 1
  const events = [...scenario.events]
  events.splice(index, 0, { ...event, currunix: String(event.currunix) })
  return { ...scenario, events }
}

/** A new scenario without the event `curruuid` names; the same one when absent. */
export function removeEvent(scenario, curruuid) {
  const events = scenario.events.filter((event) => event.curruuid !== curruuid)
  return events.length === scenario.events.length ? scenario : { ...scenario, events }
}

/**
 * The earliest instant a change between two scenarios affects - an event
 * inserted, removed or moved, each at `eventInstant`, where the walk reads it:
 * the replay re-runs from here. `undefined` when nothing moved.
 */
export function earliestAffected(before, after) {
  const held = new Map()
  for (const event of before.events) held.set(event.curruuid, eventInstant(event))
  let earliest
  const consider = (instant) => {
    if (earliest === undefined || instant < earliest) earliest = instant
  }
  for (const event of after.events) {
    const at = eventInstant(event)
    const was = held.get(event.curruuid)
    if (was === undefined) consider(at)
    else if (was !== at) {
      consider(was)
      consider(at)
    }
    held.delete(event.curruuid)
  }
  for (const instant of held.values()) consider(instant)
  return earliest
}

/** JSON text of a scenario: instants and hashes as strings, nothing lost. */
export function serializeScenario(scenario) {
  return JSON.stringify(scenario, (key, value) => (typeof value === 'bigint' ? value.toString() : value), 2)
}

/** A scenario read back from `serializeScenario`'s text, its shape checked. */
export function parseScenario(text) {
  const held = JSON.parse(text)
  if (!held || typeof held.name !== 'string' || !Array.isArray(held.events)) {
    throw new TypeError('expected a scenario: a name and a list of events')
  }
  let scenario = createScenario(held.name)
  for (const event of held.events) scenario = insertEvent(scenario, event)
  return scenario
}
