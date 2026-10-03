'use strict'

// `node/src/logging.rs`: the core's logging tree, Python's `logging` in
// camelCase - loggers, levels, handlers through any location, formatters.

const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const yggdryl = require('yggdryl')
const { logging, IOBase, Url } = yggdryl

const PACKAGE = path.join(__dirname, '..')

function scratch(name) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `yggdryl-logging-${name}-`))
}

/** Run `script` in a fresh process, the package as `process.argv[1]`. */
function child(script) {
  const ran = spawnSync(process.execPath, ['-e', script, PACKAGE], {
    encoding: 'utf8',
    timeout: 60_000,
  })
  assert.equal(ran.status, 0, `child exited ${ran.status}:\n${ran.stdout}\n${ran.stderr}`)
  return ran
}

test('the namespace is exactly its doors, frozen, its levels the core table', () => {
  assert.ok(Object.isFrozen(logging))
  assert.deepEqual(Object.keys(logging), [
    'NOTSET',
    'TRACE',
    'DEBUG',
    'INFO',
    'WARNING',
    'ERROR',
    'CRITICAL',
    'Logger',
    'Formatter',
    'StreamHandler',
    'FileHandler',
    'NullHandler',
    'getLogger',
    'basicConfig',
    'disable',
    'shutdown',
  ])
  assert.deepEqual(
    [logging.NOTSET, logging.TRACE, logging.DEBUG, logging.INFO, logging.WARNING, logging.ERROR, logging.CRITICAL],
    [0, 5, 10, 20, 30, 40, 50],
  )
  for (const name of ['Logger', 'Formatter', 'StreamHandler', 'FileHandler', 'NullHandler']) {
    assert.equal(yggdryl[name], undefined, `${name} is reached through the namespace alone`)
  }
})

test('one name is one logger hung from its nearest ancestor', () => {
  const leaf = logging.getLogger('js.tree.a.b')
  assert.equal(leaf.parent.name, 'root')
  const middle = logging.getLogger('js.tree.a')
  assert.ok(leaf.parent.equals(middle))
  assert.ok(middle.getChild('b').equals(leaf))
  assert.ok(logging.getLogger().equals(logging.getLogger('root')))
  assert.equal(logging.getLogger().parent, null)
  assert.equal(String(leaf), 'js.tree.a.b (WARNING)')
})

test('a level is a number or a name, inherited and cached until changed', () => {
  const parent = logging.getLogger('js.levels')
  const child = logging.getLogger('js.levels.child')
  assert.equal(child.level, logging.NOTSET)
  assert.equal(child.getEffectiveLevel(), logging.WARNING)
  assert.equal(child.isEnabledFor('info'), false)
  parent.setLevel('DEBUG')
  assert.equal(child.getEffectiveLevel(), logging.DEBUG)
  assert.equal(child.isEnabledFor(logging.DEBUG), true)
  parent.setLevel(15)
  assert.equal(child.isEnabledFor(logging.DEBUG), false)
  assert.equal(child.isEnabledFor(15), true)
  assert.throws(() => parent.setLevel('LOUD'), /expected one of NOTSET/)
  assert.throws(() => parent.setLevel(1.5), /level must be an unsigned 8-bit integer/)
  assert.throws(() => parent.setLevel(256), /level must be an unsigned 8-bit integer/)
})

test('a file handler writes through a location, holding records under its capacity', () => {
  const folder = scratch('file')
  const location = path.join(folder, 'logs', 'feed.log')
  const handler = new logging.FileHandler(location, { capacity: 1 << 16, flushLevel: 'ERROR' })
  handler.setFormatter(new logging.Formatter('%(levelname)s %(name)s %(message)s'))
  assert.equal(handler.url, String(Url.fromPath(location)))
  assert.deepEqual([handler.mode, handler.capacity, handler.flushLevel, handler.level], ['append', 1 << 16, 40, 0])
  const logger = logging.getLogger('js.file')
  logger.setLevel('INFO')
  logger.addHandler(handler)
  logger.addHandler(handler)
  logger.info('opened')
  logger.debug('under the level')
  assert.equal(fs.existsSync(location), false, 'held under the capacity')
  logger.error('rejected')
  assert.equal(fs.readFileSync(location, 'utf8'), 'INFO js.file opened\nERROR js.file rejected\n')
  logger.warning('held')
  handler.flush()
  assert.ok(fs.readFileSync(location, 'utf8').endsWith('WARNING js.file held\n'))
  assert.equal(logger.removeHandler(handler), true)
  assert.equal(logger.removeHandler(handler), false)
  handler.close()
  fs.rmSync(folder, { recursive: true, force: true })
})

test('an overwriting handler replaces the location once and takes an IOBase', () => {
  const folder = scratch('overwrite')
  const location = path.join(folder, 'replaced.log')
  fs.writeFileSync(location, 'yesterday\n')
  const handler = new logging.FileHandler(new IOBase(location), { mode: 'overwrite', level: 'INFO' })
  handler.setFormatter(new logging.Formatter())
  const logger = logging.getLogger('js.overwrite')
  logger.setLevel(logging.DEBUG)
  logger.addHandler(handler)
  logger.debug('under the handler level')
  logger.info('today')
  logger.info('later')
  assert.equal(fs.readFileSync(location, 'utf8'), 'today\nlater\n')
  handler.close()
  assert.throws(() => new logging.FileHandler(location, { mode: 'merge' }), /expected append or overwrite/)
  fs.rmSync(folder, { recursive: true, force: true })
})

test('a disabled logger and one that does not propagate keep records from the ancestors', () => {
  const folder = scratch('propagate')
  const location = path.join(folder, 'top.log')
  const top = logging.getLogger('js.chain')
  const leaf = logging.getLogger('js.chain.leaf')
  const handler = new logging.FileHandler(location)
  handler.setFormatter(new logging.Formatter())
  top.addHandler(handler)
  top.setLevel('INFO')
  assert.equal(leaf.hasHandlers(), true)
  leaf.info('climbs')
  leaf.propagate = false
  assert.equal(leaf.propagate, false)
  assert.equal(leaf.hasHandlers(), false)
  leaf.info('stays')
  leaf.propagate = true
  leaf.disabled = true
  assert.equal(leaf.isEnabledFor('CRITICAL'), false)
  leaf.critical('dropped')
  leaf.disabled = false
  assert.equal(fs.readFileSync(location, 'utf8'), 'climbs\n')
  top.removeHandler(handler)
  handler.close()
  fs.rmSync(folder, { recursive: true, force: true })
})

test('a formatter spells Python keys and refuses what cannot spell a record', () => {
  const formatter = new logging.Formatter('%(asctime)s %(message)s', { datefmt: '%H:%M %Z', timezone: 'Europe/Paris' })
  assert.deepEqual([formatter.format, formatter.datefmt, formatter.timezone], [
    '%(asctime)s %(message)s',
    '%H:%M %Z',
    'Europe/Paris',
  ])
  assert.equal(new logging.Formatter().format, '%(message)s')
  assert.throws(() => new logging.Formatter('%(colour)s'), /invalid log format expression at byte 2/)
  assert.throws(() => new logging.Formatter('%(name)s', { datefmt: '%Q' }), /invalid log date format expression/)
})

test('a record no handler takes reaches standard error as the terminal line', () => {
  const ran = child(`
    const { logging } = require(process.argv[1])
    logging.getLogger('js.resort').info('under the last resort level')
    function rejectOrder() {
      logging.getLogger('js.resort').error('said')
    }
    rejectOrder()
  `)
  assert.match(
    ran.stderr,
    /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} ✗ ERROR    \[main\] js\.resort rejectOrder:5 › said\n$/,
  )
})

test('basicConfig configures a bare root once, and disable drops what it names', () => {
  const ran = child(`
    const { logging } = require(process.argv[1])
    logging.basicConfig({ level: 'INFO', format: '%(levelname)s|%(name)s|%(message)s', handlers: [new logging.StreamHandler('stdout')] })
    logging.basicConfig({ level: 'CRITICAL' })
    logging.getLogger('js.basic').info('configured')
    logging.disable('WARNING')
    logging.getLogger('js.basic').warning('dropped')
    logging.disable(logging.NOTSET)
    logging.getLogger('js.basic').debug('under the root level')
    logging.getLogger('js.basic').warning('kept')
  `)
  assert.equal(ran.stdout, 'INFO|js.basic|configured\nWARNING|js.basic|kept\n')
  assert.equal(ran.stderr, '')
})

test('the core reports through the tree and what a handler held is published at exit', () => {
  const folder = scratch('exit')
  const location = path.join(folder, 'core.log')
  child(`
    const { logging, fix } = require(process.argv[1])
    const handler = new logging.FileHandler(${JSON.stringify(location)}, { capacity: 1 << 20 })
    handler.setFormatter(new logging.Formatter('%(name)s %(message)s'))
    logging.getLogger('yggdryl').addHandler(handler)
    const codec = new fix.FixCodec(new fix.FixRegistry())
    codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=bad|10=0|'))
  `)
  const written = fs.readFileSync(location, 'utf8')
  assert.match(written, /^yggdryl\.fix\.\w+ FIX clock left unstated/)
  fs.rmSync(folder, { recursive: true, force: true })
})

test('a deduplicating logger says a record once, then at each tenfold count', () => {
  const folder = scratch('dedup')
  const location = path.join(folder, 'dedup.log')
  const top = logging.getLogger('js.dedup')
  const leaf = logging.getLogger('js.dedup.orders')
  const handler = new logging.FileHandler(location)
  handler.setFormatter(new logging.Formatter())
  top.addHandler(handler)
  assert.equal(top.deduplicating, null)
  assert.equal(leaf.isDeduplicating(), false)
  top.deduplicating = true
  assert.equal(top.deduplicating, true)
  assert.equal(leaf.isDeduplicating(), true)
  for (let at = 0; at < 100; at += 1) leaf.warning('late fill')
  leaf.warning('late fills')
  leaf.deduplicating = false
  leaf.warning('late fill')
  assert.equal(
    fs.readFileSync(location, 'utf8'),
    'late fill\nlate fill (seen 10 times)\nlate fill (seen 100 times)\nlate fills\nlate fill\n',
  )
  leaf.deduplicating = null
  top.deduplicating = null
  top.removeHandler(handler)
  handler.close()
  fs.rmSync(folder, { recursive: true, force: true })
})

test('a record carries its JavaScript call site and thread, read only when enabled', () => {
  const folder = scratch('site')
  const location = path.join(folder, 'site.log')
  const logger = logging.getLogger('js.site')
  logger.setLevel('INFO')
  const handler = new logging.FileHandler(location)
  handler.setFormatter(new logging.Formatter('%(funcName)s|%(lineno)d|%(threadName)s|%(filename)s|%(caller)s'))
  logger.addHandler(handler)
  function submitOrder() {
    logger.info('placed')
  }
  submitOrder()
  logger.debug('below the level')
  logger.log('WARNING', 'stated')
  const [first, second] = fs.readFileSync(location, 'utf8').trim().split('\n')
  const line = (text) => Number(text.split('|')[1])
  assert.match(first, /^submitOrder\|\d+\|main\|logging\.test\.js\|submitOrder:\d+$/)
  assert.ok(line(second) > line(first))
  logger.removeHandler(handler)
  handler.close()
  fs.rmSync(folder, { recursive: true, force: true })
})

test('the terminal formatter is prebuilt and a stream handler says whether it colours', () => {
  const terminal = logging.Formatter.terminal()
  assert.match(terminal.format, /%\(levelglyph\)s %\(levelname\)-8s/)
  assert.match(terminal.format, /\[%\(threadName\)s\]/)
  assert.match(terminal.format, /%\(caller\)s/)
  const stream = new logging.StreamHandler('stderr')
  assert.equal(typeof stream.colored, 'boolean')
  stream.colored = true
  assert.equal(stream.colored, true)
  assert.equal(stream.formatter.format, terminal.format, 'a handler stating none spells the terminal line')
})

test('a refused level names the argument it came in', () => {
  assert.throws(
    () => new logging.FileHandler('never.log', { flushLevel: 300 }),
    /flushLevel must be an unsigned 8-bit integer, got 300/,
  )
  assert.throws(() => new logging.FileHandler('never.log', { level: 'LOUD' }), /level: invalid log level/)
})

test('basicConfig with nothing configured writes the coloured-or-plain terminal line', () => {
  const ran = child(`
    const { logging } = require(process.argv[1])
    logging.basicConfig({ level: 'INFO', handlers: [new logging.StreamHandler('stdout')] })
    logging.getLogger('js.terminal').info('configured')
  `)
  assert.match(ran.stdout, / • INFO     \[main\] js\.terminal \[eval\]:4 › configured\n$/)
  assert.ok(!ran.stdout.includes('\x1b'), 'a pipe is no colour terminal')
})

test('a record an exit listener logs after the shutdown is still published', () => {
  const folder = scratch('late')
  const location = path.join(folder, 'late.log')
  child(`
    const { logging } = require(process.argv[1])
    const handler = new logging.FileHandler(${JSON.stringify(location)}, { capacity: 1 << 20 })
    handler.setFormatter(new logging.Formatter())
    logging.getLogger('js.late').addHandler(handler)
    logging.getLogger('js.late').warning('before exit')
    process.on('exit', () => logging.getLogger('js.late').warning('while exiting'))
  `)
  assert.equal(fs.readFileSync(location, 'utf8'), 'before exit\nwhile exiting\n')
  fs.rmSync(folder, { recursive: true, force: true })
})

test('a stack overflowing while a call site is read leaves the stack traces as they were', () => {
  const logger = logging.getLogger('js.overflow')
  logger.propagate = false
  logger.setLevel('INFO')
  logger.addHandler(new logging.NullHandler())
  const limit = Error.stackTraceLimit
  const prepare = Error.prepareStackTrace
  const walk = () => {
    logger.info('deeper')
    walk()
  }
  assert.throws(walk, RangeError)
  assert.equal(Error.stackTraceLimit, limit)
  assert.equal(Error.prepareStackTrace, prepare)
  assert.equal(typeof new Error('after').stack, 'string')
})

test('a worker ending shuts nothing down: the tree is the process main thread', () => {
  const folder = scratch('worker')
  const location = path.join(folder, 'held.log')
  child(`
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const { Worker } = require('node:worker_threads')
    const { logging } = require(process.argv[1])
    const handler = new logging.FileHandler(${JSON.stringify(location)}, { capacity: 1 << 20 })
    handler.setFormatter(new logging.Formatter())
    const logger = logging.getLogger('js.worker')
    logger.setLevel('INFO')
    logger.addHandler(handler)
    logger.info('held')
    const worker = new Worker('require(process.argv[2])', { eval: true, argv: [process.argv[1]] })
    worker.on('exit', () => {
      assert.equal(fs.existsSync(${JSON.stringify(location)}), false, 'still held after the worker')
      logger.info('after the worker')
    })
  `)
  assert.equal(fs.readFileSync(location, 'utf8'), 'held\nafter the worker\n', 'published at the main exit')
  fs.rmSync(folder, { recursive: true, force: true })
})
