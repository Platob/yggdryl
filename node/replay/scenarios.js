'use strict'

// Named scenarios, kept as files: `<state>/<name>.json` holds what the page's
// `scenario.js` serializes - the name and the inserted events in instant
// order, each event its leaf's JSON beside `native`, the leaf's own `toJSON`
// text. The native text is the event: a save re-renders every event from it,
// so the facts a file shows are the native leaf's, and admits each leaf alone
// through the native walk, so what a file holds is what a re-run can fold; a
// load rebuilds each leaf through `MarketData.fromJSON`, which refuses a text
// that is not one.

const fs = require('node:fs')
const path = require('node:path')

const { eventJson, eventsJson } = require('./json.js')
const { admit, eventsOf } = require('./walk.js')
const web = require('./web.js')

/**
 * A scenario name: its own file name, so nothing that could leave the folder,
 * and lowercase, so two names are two files where the file system folds case.
 */
const NAME = /^[a-z0-9][a-z0-9._-]{0,127}$/

/** `name`, when it names a scenario file; a `TypeError` otherwise. */
function checkName(name) {
  if (typeof name !== 'string' || !NAME.test(name)) {
    throw new TypeError(
      "expected a scenario name of at most 128 lowercase letters, digits, '.', '_' or '-', " +
        `starting with a lowercase letter or a digit, got ${JSON.stringify(name)}`,
    )
  }
  return name
}

/**
 * The scenarios under `stateDir`:
 *
 * - `list()` their names;
 * - `read(name)` one as its file holds it, or `null` when absent - its shape
 *   and name checked, no event decoded;
 * - `load(name)` one as `{ scenario, operations }`, or `null` - every event
 *   rebuilt from its native text, which refuses a text that is no leaf;
 * - `save(scenario, operations?)` one - every event's leaf, from
 *   `operations` (one per event, in order) when the caller holds them and
 *   rebuilt from its native text otherwise, admitted alone by the native walk
 *   and the event re-rendered from it - answering what it stored;
 * - `insert(scenario, leaf)` one leaf into a scenario as `read` answered it
 *   (or a new one) - the leaf admitted alone by the native walk and rendered
 *   once, the events already held kept as they are - answering the stored
 *   event;
 * - `remove(name)` one, answering whether it was there.
 *
 * The folder is created on the first write, and a file is replaced whole.
 */
async function openScenarios(stateDir) {
  const { createScenario, insertEvent, parseScenario, serializeScenario } = await web()
  const fileOf = (name) => path.join(stateDir, `${checkName(name)}.json`)

  function list() {
    let entries
    try {
      entries = fs.readdirSync(stateDir)
    } catch (error) {
      if (error.code === 'ENOENT') return []
      throw error
    }
    return entries
      .filter((entry) => entry.endsWith('.json'))
      .map((entry) => entry.slice(0, -'.json'.length))
      .filter((name) => NAME.test(name))
      .sort()
  }

  function read(name) {
    let text
    try {
      text = fs.readFileSync(fileOf(name), 'utf8')
    } catch (error) {
      if (error.code === 'ENOENT') return null
      throw error
    }
    const scenario = parseScenario(text)
    if (scenario.name !== name) {
      throw new Error(`scenario file ${name}.json holds the scenario ${JSON.stringify(scenario.name)}`)
    }
    return scenario
  }

  function load(name) {
    const scenario = read(name)
    return scenario === null ? null : { scenario, operations: eventsOf(scenario) }
  }

  function write(scenario) {
    fs.mkdirSync(stateDir, { recursive: true })
    const file = fileOf(scenario.name)
    const staged = `${file}.${process.pid}.tmp`
    fs.writeFileSync(staged, serializeScenario(scenario))
    fs.renameSync(staged, file)
    return scenario
  }

  function save(scenario, operations) {
    let stored = createScenario(checkName(scenario.name))
    const leaves = eventsOf(scenario, operations).map(admit)
    for (const event of eventsJson(leaves)) stored = insertEvent(stored, event)
    return write(stored)
  }

  function insert(scenario, leaf) {
    checkName(scenario.name)
    const event = eventJson(admit(leaf))
    const stored = write(insertEvent(scenario, event))
    return stored.events.find((held) => held.curruuid === event.curruuid)
  }

  function remove(name) {
    try {
      fs.unlinkSync(fileOf(name))
      return true
    } catch (error) {
      if (error.code === 'ENOENT') return false
      throw error
    }
  }

  return Object.freeze({ dir: stateDir, list, read, load, save, insert, remove })
}

module.exports = { openScenarios, checkName }
