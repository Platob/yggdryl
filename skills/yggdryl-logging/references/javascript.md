# yggdryl-logging in JavaScript

`const { logging } = require('yggdryl')` - the core's tree, Python's
`logging` in camelCase: the level numbers `logging.NOTSET` to
`logging.CRITICAL`, `getLogger`, `basicConfig`, `disable` and `shutdown`,
and the classes `Logger`, `Formatter`, `StreamHandler`, `FileHandler` and
`NullHandler`, reached through the namespace alone. A level is a number or a
name in any case. The addon installs the tree when it loads, names its
thread `main` (`worker-N` in a worker), admits other crates' records only
from `WARNING`, and writes what no handler takes - from `WARNING` up - to
standard error as the terminal line. Not offered: filters, a handler calling
back into JavaScript (a record can be logged on any Rust thread), a
logger's `handlers` list (`hasHandlers()` answers).

A handler has `level` and `setLevel(level)`; a `StreamHandler` and a
`FileHandler` add `formatter`, `setFormatter(formatter)`, `flush()` and
`close()`, the stream handler its `colored`, the file handler `url`, `mode`,
`capacity` and `flushLevel`. A `NullHandler` has the level alone.

## Set up an application

`basicConfig()` gives a root with no handler a `StreamHandler` on standard
error writing the terminal line; `{ level, format, datefmt, handlers, force }`
are Python's arguments. `shutdown()` publishes and closes every handler - the
addon runs it at the main thread's `exit`, never a worker's.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging } = require('yggdryl')

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-skills-logging-app-'))
try {
  const location = path.join(folder, 'app.log')
  const handler = new logging.FileHandler(location)
  logging.basicConfig({
    level: 'INFO',
    format: '%(levelname)s %(name)s %(message)s',
    handlers: [handler],
  })

  logging.getLogger('skills.logging.feed').info('opened 3 venues')
  logging.getLogger('skills.logging.feed').debug("below the root's level")
  logging.getLogger('skills.logging.book').warning('crossed')

  logging.shutdown()
  assert.equal(
    fs.readFileSync(location, 'utf8'),
    'INFO skills.logging.feed opened 3 venues\nWARNING skills.logging.book crossed\n',
  )

  const root = logging.getLogger()
  assert.equal(root.removeHandler(handler), true)
  root.setLevel('WARNING')
} finally {
  fs.rmSync(folder, { recursive: true, force: true })
}
```

## Loggers and levels

A logger hangs from its nearest existing ancestor and takes its level while
at `NOTSET`. `disable(level)` drops every record at or below `level` on
every logger (`CRITICAL` when no level is given); `disable(NOTSET)` lifts it.

```javascript
const assert = require('node:assert/strict')
const { logging } = require('yggdryl')

assert.deepEqual(
  [logging.NOTSET, logging.TRACE, logging.DEBUG, logging.INFO, logging.WARNING, logging.ERROR, logging.CRITICAL],
  [0, 5, 10, 20, 30, 40, 50],
)

// Asked for after its child, the parent still becomes the child's parent.
const orders = logging.getLogger('skills.logging.tree.orders')
const tree = logging.getLogger('skills.logging.tree')
assert.ok(orders.parent.equals(tree))
assert.ok(tree.getChild('orders').equals(orders))
assert.equal(logging.getLogger().name, 'root')

tree.setLevel('warn')
assert.equal(tree.level, logging.WARNING)
tree.setLevel(15)
assert.equal(orders.level, logging.NOTSET)
assert.equal(orders.getEffectiveLevel(), 15)
assert.equal(orders.isEnabledFor('DEBUG'), false)
assert.equal(orders.isEnabledFor('info'), true)
assert.throws(() => tree.setLevel('verbose'), /invalid log level expression at byte 0/)
assert.throws(() => tree.setLevel(256), /level must be an unsigned 8-bit integer/)

logging.disable('INFO')
assert.equal(orders.isEnabledFor('INFO'), false)
logging.disable(logging.NOTSET)
assert.equal(orders.isEnabledFor('INFO'), true)

tree.setLevel(logging.NOTSET)
```

## Write a log through any location

`new logging.FileHandler(location, { mode, capacity, flushLevel, level })`
writes through a path or URL text, a `Url` or an `IOBase` - a local file, an
object in a bucket, a ZIP member - with one append per publish. A
`capacity` holds records back until that many bytes are held or a record at
`flushLevel` (`ERROR` by default) arrives: state one over a remote store,
where an append is a whole `PUT`. `mode` is `'append'` (the default) or
`'overwrite'`, which replaces the location with the first publish only.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging, IOBase } = require('yggdryl')

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-skills-logging-file-'))
try {
  const logger = logging.getLogger('skills.logging.file')
  logger.setLevel('INFO')
  logger.propagate = false

  const location = path.join(folder, 'logs', 'feed.log')
  const handler = new logging.FileHandler(location, { capacity: 64 * 1024, flushLevel: 'error' })
  handler.setFormatter(new logging.Formatter('%(levelname)s %(message)s'))
  assert.deepEqual([handler.mode, handler.capacity, handler.flushLevel], ['append', 65536, logging.ERROR])
  assert.ok(handler.url.startsWith('file://'))
  logger.addHandler(handler)

  logger.info('opened')
  assert.equal(fs.existsSync(location), false) // held under the capacity
  logger.error('rejected') // a record at the flush level publishes everything held
  assert.equal(fs.readFileSync(location, 'utf8'), 'INFO opened\nERROR rejected\n')
  logger.info('closed')
  handler.flush()
  assert.ok(fs.readFileSync(location, 'utf8').endsWith('INFO closed\n'))
  logger.removeHandler(handler)
  handler.close()

  const replaced = path.join(folder, 'replaced.log')
  fs.writeFileSync(replaced, 'yesterday\n')
  const fresh = new logging.FileHandler(new IOBase(replaced), { mode: 'overwrite' })
  fresh.setFormatter(new logging.Formatter())
  logger.addHandler(fresh)
  logger.info('today')
  assert.equal(fs.readFileSync(replaced, 'utf8'), 'today\n')
  logger.removeHandler(fresh)
  fresh.close()

  assert.throws(() => new logging.FileHandler(location, { mode: 'merge' }), /expected append or overwrite/)
  assert.throws(() => new logging.FileHandler(location, { mode: 'a' }), /invalid mode expression/)
  logger.setLevel(logging.NOTSET)
  logger.propagate = true
} finally {
  fs.rmSync(folder, { recursive: true, force: true })
}
```

## Formats and the terminal line

`new logging.Formatter(format?, { datefmt, timezone })` is Python's
`%`-style format, parsed when built and refused at the byte; dates are UTC
unless a zone is named, and `new logging.Formatter()` is `%(message)s`.
`logging.Formatter.terminal()` is the line a handler stating no formatter
writes, with the JavaScript caller's `function:line` as its call site. A
`StreamHandler` decides `colored` from its stream when built - `NO_COLOR`,
`FORCE_COLOR`, `CLICOLOR_FORCE`, `TERM=dumb`, a terminal - and a file is
always plain.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging } = require('yggdryl')

const formatter = new logging.Formatter('%(asctime)s %(levelname)-8s %(name)s %(message)s', {
  datefmt: '%Y-%m-%dT%H:%M:%SZ',
})
assert.deepEqual([formatter.datefmt, formatter.timezone], ['%Y-%m-%dT%H:%M:%SZ', 'UTC'])
assert.equal(new logging.Formatter().format, '%(message)s')
assert.equal(new logging.Formatter('%(asctime)s', { timezone: 'Europe/Paris' }).timezone, 'Europe/Paris')
assert.throws(() => new logging.Formatter('%(name)d'), /name is text and takes the s, r or a conversion/)
assert.throws(() => new logging.Formatter('plain text'), /naming at least one %\(key\)/)

const stderr = new logging.StreamHandler('stderr')
assert.equal(stderr.formatter.format, logging.Formatter.terminal().format)
assert.equal(typeof stderr.colored, 'boolean')
stderr.colored = false
assert.equal(stderr.colored, false)
assert.throws(() => new logging.StreamHandler('file'), /expected the stream stderr or stdout/)

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-skills-logging-format-'))
try {
  const dated = new logging.FileHandler(path.join(folder, 'dated.log'))
  dated.setFormatter(formatter)
  const plain = new logging.FileHandler(path.join(folder, 'terminal.log'))
  const logger = logging.getLogger('skills.logging.format')
  logger.setLevel('INFO')
  logger.propagate = false
  logger.addHandler(dated)
  logger.addHandler(plain)

  function openFeed() {
    logger.warning('late fill')
  }
  openFeed()
  assert.match(
    fs.readFileSync(path.join(folder, 'dated.log'), 'utf8'),
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z WARNING {2}skills\.logging\.format late fill\n$/,
  )
  assert.match(
    fs.readFileSync(path.join(folder, 'terminal.log'), 'utf8'),
    /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} ! WARNING {2}\[main\] skills\.logging\.format openFeed:\d+ › late fill\n$/,
  )

  for (const handler of [dated, plain]) {
    logger.removeHandler(handler)
    handler.close()
  }
  logger.setLevel(logging.NOTSET)
  logger.propagate = true
} finally {
  fs.rmSync(folder, { recursive: true, force: true })
}
```

## Deduplicate repeats

A logger whose `deduplicating` is `true` says a record the first time and
again at its 10th, 100th, 1000th occurrence as `message (seen N times)`,
dropping the rest before any handler; `false` passes every record, `null`
takes the nearest ancestor's choice, off at the root unless set. The key is
the logger, the level and the message.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging } = require('yggdryl')

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-skills-logging-dedup-'))
try {
  const location = path.join(folder, 'dedup.log')
  const feed = logging.getLogger('skills.logging.dedup')
  const orders = feed.getChild('orders')
  feed.setLevel('INFO')
  feed.propagate = false
  const handler = new logging.FileHandler(location)
  handler.setFormatter(new logging.Formatter())
  feed.addHandler(handler)

  assert.equal(feed.deduplicating, null)
  assert.equal(orders.isDeduplicating(), false)
  feed.deduplicating = true
  assert.equal(orders.isDeduplicating(), true)

  for (let at = 0; at < 100; at += 1) orders.warning('late fill')
  orders.error('late fill') // another level is another key
  assert.equal(
    fs.readFileSync(location, 'utf8'),
    'late fill\nlate fill (seen 10 times)\nlate fill (seen 100 times)\nlate fill\n',
  )

  feed.deduplicating = null
  feed.removeHandler(handler)
  handler.close()
  feed.setLevel(logging.NOTSET)
  feed.propagate = true
} finally {
  fs.rmSync(folder, { recursive: true, force: true })
}
```

## Where the core's records land

The core logs under its Rust module path: `yggdryl.iceberg.table` narrates
a create or a commit at `INFO`, `yggdryl.aws.session` reports the AWS
credential walk, and what a FIX parse or a market read passes over is a
`WARNING` on `yggdryl.fix.*`, said once per kind for the process with its
detail and then counted at each tenfold occurrence. With nothing configured
the last resort writes it to standard error; a handler on `yggdryl` takes it
instead.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging, fix } = require('yggdryl')

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-skills-logging-core-'))
try {
  const location = path.join(folder, 'core.log')
  const handler = new logging.FileHandler(location)
  handler.setFormatter(new logging.Formatter('%(levelname)s %(name)s %(message)s'))
  const core = logging.getLogger('yggdryl')
  core.addHandler(handler)

  const codec = new fix.FixCodec(new fix.FixRegistry())
  for (let at = 0; at < 10; at += 1) {
    // `52=bad` names no instant: the message is read and its clock left unstated.
    codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=bad|10=0|'))
  }
  const said = fs
    .readFileSync(location, 'utf8')
    .split('\n')
    .filter((line) => line.includes('FIX clock left unstated'))
  assert.equal(said.length, 2)
  assert.match(said[0], /^WARNING yggdryl\.fix\.\w+ FIX clock left unstated.*\(sendingtime\)/)
  assert.match(said[1], /seen 10 times$/)

  core.removeHandler(handler)
  handler.close()
} finally {
  fs.rmSync(folder, { recursive: true, force: true })
}
```

## Gotchas in JavaScript

- `removeHandler(handler)` detaches the very handler object that was added,
  and never closes it: call `handler.close()` yourself.
- A `FileHandler` over a location backed by a JavaScript-implemented file
  system is refused: a record can be logged on any Rust thread, and such a
  file system answers on its isolate's thread only.
- A record carries its JavaScript call site: `%(caller)s` is `function:line`
  where a function is named, else the file's stem and the line, and
  `%(funcName)s` is `(unknown function)` at a script's top level. A record
  the core logs names its Rust module's last segment and line instead
  (`table:227`).
- A level is read wherever it is taken: a number from `0` to `255` or a name
  in any case; `setLevel(256)` throws `level must be an unsigned 8-bit
  integer` and an unknown name `invalid log level expression at byte 0`.
