#!/usr/bin/env node
'use strict'

// `yggdryl/replay`: the trading replay service over the native package, and
// its command line.
//
//   node node/replay.js <source> [--port 0] [--snapshot-millis 0] [--global]
//     [--rowheader <regex>] [--sending-time <ns|ISO 8601>] [--registry <dir>]
//     [--state <dir>] [--web <dir>]
//
// `<source>` is a `.arrow`/`.ipc`/`.feather`/`.parquet` file of `marketdata`
// rows, the word `synthetic`, or any other file as a FIX capture read under
// the row header (the ULBridge one by default), dated by the sending clock
// where a line states none (2024-01-02T10:15:30Z by default) and resolved
// against the dictionary in `--registry` (the process default otherwise).
// Prints the URL it serves at and serves until SIGINT; scenarios are kept in
// `--state`, `~/.config/yggdryl/replay` by default.

const os = require('node:os')
const path = require('node:path')

const { bookJson, booksJson, eventJson, leafFromJson, leafJson, leavesJson, refusalText, rowsOf, toJson } =
  require('./replay/json.js')
const { openScenarios } = require('./replay/scenarios.js')
const { createReplayServer } = require('./replay/server.js')
const { loadSource } = require('./replay/sources.js')
const { synthetic } = require('./replay/synthetic.js')
const { bookAt, booksBetween, indexBooks, merged, rerun, walk } = require('./replay/walk.js')

/** Where the command line keeps scenarios unless `--state` names a folder. */
const DEFAULT_STATE_DIR = path.join(os.homedir(), '.config', 'yggdryl', 'replay')

const OPTIONS = Object.freeze({
  '--port': 'port',
  '--snapshot-millis': 'snapshotMillis',
  '--rowheader': 'rowheader',
  '--sending-time': 'sendingTime',
  '--registry': 'registry',
  '--state': 'state',
  '--web': 'web',
})

/** A whole number option: its value, or a refusal naming the option. */
function wholeNumber(option, value) {
  const number = Number(value)
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(number)) {
    throw new TypeError(`${option}: expected a non-negative whole number, got ${JSON.stringify(value)}`)
  }
  return number
}

/** The command line's arguments, each option read once; every default in one place. */
function parseArgs(argv) {
  const args = {
    source: undefined,
    port: 0,
    snapshotMillis: 0,
    global: false,
    rowheader: undefined,
    sendingTime: undefined,
    registry: undefined,
    state: DEFAULT_STATE_DIR,
    web: undefined,
  }
  for (let at = 0; at < argv.length; at += 1) {
    const argument = argv[at]
    if (argument === '--global') {
      args.global = true
    } else if (Object.hasOwn(OPTIONS, argument)) {
      const value = argv[at + 1]
      if (value === undefined) throw new TypeError(`${argument}: expected a value`)
      at += 1
      args[OPTIONS[argument]] = value
    } else if (argument.startsWith('--')) {
      throw new TypeError(`unknown option ${argument}; the options are ${['--global', ...Object.keys(OPTIONS)].join(', ')}`)
    } else if (args.source === undefined) {
      args.source = argument
    } else {
      throw new TypeError(`expected one source, got ${JSON.stringify(args.source)} and ${JSON.stringify(argument)}`)
    }
  }
  if (args.source === undefined) {
    throw new TypeError('expected a source: a .arrow, .ipc, .feather or .parquet file, a FIX capture, or synthetic')
  }
  args.port = wholeNumber('--port', String(args.port))
  args.snapshotMillis = wholeNumber('--snapshot-millis', String(args.snapshotMillis))
  return args
}

/**
 * Run the command line: load the source, serve it, print the URL. Answers
 * `{ url, close }`; SIGINT closes the service and exits.
 */
async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv)
  // An option not given is `undefined`, which the loader's own default fills.
  const source = loadSource(args.source, {
    rowheader: args.rowheader,
    sendingTime: args.sendingTime,
    registry: args.registry ?? null,
  })
  const service = createReplayServer({
    sources: [source],
    stateDir: args.state,
    webDir: args.web,
    snapshotMillis: args.snapshotMillis,
    global: args.global,
  })
  const url = await service.listen(args.port)
  const interrupted = () => {
    service.close().then(() => process.exit(0))
  }
  process.once('SIGINT', interrupted)
  process.stderr.write(
    `source ${source.id}: ${source.operations.length} operations, symbols ${source.symbols.join(', ')}` +
      `${source.refusals.length ? `, ${source.refusals.length} refused` : ''}\n`,
  )
  process.stdout.write(`${url}\n`)
  return {
    url,
    close() {
      process.off('SIGINT', interrupted)
      return service.close()
    },
  }
}

if (require.main === module) {
  main().catch((error) => {
    process.stderr.write(`${refusalText(error)}\n`)
    process.exit(1)
  })
}

module.exports = {
  DEFAULT_STATE_DIR,
  bookAt,
  bookJson,
  booksBetween,
  booksJson,
  createReplayServer,
  eventJson,
  indexBooks,
  leafFromJson,
  leafJson,
  leavesJson,
  loadSource,
  main,
  merged,
  openScenarios,
  parseArgs,
  refusalText,
  rerun,
  rowsOf,
  synthetic,
  toJson,
  walk,
}
