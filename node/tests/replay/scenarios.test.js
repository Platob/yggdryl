'use strict'

// Named scenarios as files: `node/replay/scenarios.js`.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { after, test } = require('node:test')

const { graph } = require('yggdryl')

const { eventJson } = require('../../replay/json.js')
const { checkName, openScenarios } = require('../../replay/scenarios.js')
const { T0 } = require('../../replay/synthetic.js')
const { walk } = require('../../replay/walk.js')
const web = require('../../replay/web.js')

const made = []
after(() => {
  for (const folder of made) fs.rmSync(folder, { recursive: true, force: true })
})

function scratch() {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-scenarios-'))
  made.push(folder)
  return path.join(folder, 'state')
}

function order(at, code, price) {
  return new graph.OrderEvent(at, { crosscode: code, ticker: 'ALPHA', side: 'BUY', price, quantity: 5 })
}

test('names are lowercase file names that stay in the folder', () => {
  for (const name of ['what-if', 'a', 'case_2.b', '9', 'x'.repeat(128)]) assert.equal(checkName(name), name)
  // Upper case is refused: two names differing by case would be one file on Windows and macOS.
  for (const name of ['', '.hidden', '../up', 'a/b', 'a\\b', 'a b', 'x'.repeat(129), 'Alpha', 'Case_2.b', 7]) {
    assert.throws(() => checkName(name), {
      name: 'TypeError',
      message:
        "expected a scenario name of at most 128 lowercase letters, digits, '.', '_' or '-', " +
        `starting with a lowercase letter or a digit, got ${JSON.stringify(name)}`,
    })
  }
})

test('save, list, load and remove round trip through the native toJSON', async () => {
  const { createScenario, insertEvent } = await web()
  const store = await openScenarios(scratch())
  // Nothing is created before the first save.
  assert.deepEqual(store.list(), [])
  assert.equal(store.load('what-if'), null)
  assert.equal(fs.existsSync(store.dir), false)

  const late = order(T0 + 2n, 'L', '82.5')
  const early = order(T0 + 1n, 'E', '82.25')
  let scenario = insertEvent(createScenario('what-if'), eventJson(late))
  scenario = insertEvent(scenario, eventJson(early))
  const saved = store.save(scenario)
  assert.deepEqual(saved.events.map((event) => event.crosscode), ['E', 'L'])
  assert.deepEqual(store.list(), ['what-if'])
  store.save(createScenario('another'))
  assert.deepEqual(store.list(), ['another', 'what-if'])

  const { scenario: loaded, operations } = store.load('what-if')
  assert.deepEqual(loaded, saved)
  assert.ok(operations.every((operation) => operation instanceof graph.MarketData))
  assert.ok(operations[0].equals(new graph.MarketData(early)))
  assert.ok(operations[1].equals(new graph.MarketData(late)))
  // The file is the serialized scenario: instants and hashes as text.
  const text = fs.readFileSync(path.join(store.dir, 'what-if.json'), 'utf8')
  assert.equal(JSON.parse(text).events[0].currunix, String(T0 + 1n))

  assert.equal(store.remove('what-if'), true)
  assert.equal(store.remove('what-if'), false)
  assert.deepEqual(store.list(), ['another'])
  assert.equal(store.load('what-if'), null)
})

test('a save re-renders every event from its native text', async () => {
  const { createScenario, insertEvent } = await web()
  const store = await openScenarios(scratch())
  const event = eventJson(order(T0, 'N', '82.5'))
  const saved = store.save(insertEvent(createScenario('edited'), { ...event, price: '1' }))
  assert.equal(saved.events[0].price, '82.5')
  assert.deepEqual(saved.events[0], event)
})

test('a native text that is not a leaf is refused by the native reader, verbatim', async () => {
  const { createScenario, insertEvent } = await web()
  const store = await openScenarios(scratch())
  const broken = { ...eventJson(order(T0, 'N', '82.5')), native: 'bm90IGFycm93' }
  let expected
  try {
    graph.MarketData.fromJSON(broken.native)
  } catch (error) {
    expected = error.message
  }
  assert.ok(expected)
  assert.throws(() => store.save(insertEvent(createScenario('broken'), broken)), { message: expected })
  assert.deepEqual(store.list(), [])

  // A file edited by hand is refused the same way when it is loaded.
  const good = store.save(insertEvent(createScenario('tampered'), eventJson(order(T0, 'N', '82.5'))))
  const file = path.join(store.dir, 'tampered.json')
  fs.writeFileSync(file, JSON.stringify({ ...good, events: [{ ...good.events[0], native: broken.native }] }))
  assert.throws(() => store.load('tampered'), { message: expected })
  // A file holding another scenario's name is refused by name.
  fs.writeFileSync(file, JSON.stringify({ ...good, name: 'other' }))
  assert.throws(() => store.load('tampered'), { message: 'scenario file tampered.json holds the scenario "other"' })
  assert.throws(() => store.load('../tampered'), /expected a scenario name/)
})

test('a save admits each event alone, as the walk would: what it cannot fold is refused verbatim and nothing is written', async () => {
  const store = await openScenarios(scratch())
  const undated = new graph.Order({ crosscode: 'U' })
  let expected
  try {
    walk([undated])
  } catch (error) {
    expected = error.message
  }
  assert.match(expected, /\$\.operation\.kind: expected order_event, .* got order/)
  assert.throws(() => store.save({ name: 'undated', events: [eventJson(undated)] }), { message: expected })
  // A book side is no operation, whatever instant the event claims.
  const side = { ...eventJson(new graph.BookSide('BUY')), currunix: String(T0) }
  assert.throws(() => store.save({ name: 'side', events: [side] }), /\$\.operation\.kind: expected order_event, .* got book_side/)
  assert.throws(() => store.save({ name: 'shapeless', events: [{ curruuid: 'x' }] }), {
    name: 'TypeError',
    message: 'expected an event holding its native leaf text',
  })
  assert.deepEqual(store.list(), [])
})

test('insert admits one leaf, renders it once and stores it at the instant the walk reads it', async () => {
  const { createScenario } = await web()
  const store = await openScenarios(scratch())
  assert.equal(store.read('grown'), null)
  const late = order(T0 + 2n, 'L', '82.5')
  const stored = store.insert(createScenario('grown'), late)
  assert.deepEqual(stored, eventJson(late))
  assert.deepEqual(store.read('grown'), { name: 'grown', events: [stored] })
  // Stated at T0 but read at its grid step T0 + 3: after the event at T0 + 2.
  const stepped = new graph.OrderEvent(T0, { crosscode: 'S', ticker: 'ALPHA', side: 'BUY', price: '82', quantity: 5, snapunix: T0 + 3n })
  const early = order(T0 + 1n, 'E', '82.25')
  store.insert(store.read('grown'), stepped)
  store.insert(store.read('grown'), early)
  assert.deepEqual(store.read('grown').events.map((event) => event.crosscode), ['E', 'L', 'S'])
  // What the walk refuses is refused verbatim, and the file stays as it was.
  assert.throws(() => store.insert(store.read('grown'), new graph.Order({ crosscode: 'U' })), /\$\.operation\.kind: .* got order/)
  assert.throws(() => store.insert(store.read('grown'), early), { message: `event ${early.curruuid} is already in scenario grown` })
  assert.equal(store.read('grown').events.length, 3)
  assert.throws(() => store.insert(createScenario('../up'), early), /expected a scenario name/)
})

test('a save decodes no event whose leaf it is handed, and re-renders every event from its leaf', async () => {
  const { createScenario, insertEvent } = await web()
  const store = await openScenarios(scratch())
  const leaf = order(T0, 'H', '82.5')
  // The event's text is no leaf and its price was edited: the leaf handed beside it is the event.
  const edited = { ...eventJson(leaf), price: '1', native: 'bm90IGFycm93' }
  const saved = store.save(insertEvent(createScenario('handed'), edited), [leaf])
  assert.deepEqual(saved.events, [eventJson(leaf)])
  assert.deepEqual(store.load('handed').scenario, saved)
  assert.throws(() => store.save(createScenario('handed'), [leaf]), {
    name: 'TypeError',
    message: 'expected one operation per scenario event: 0 events, 1 operations',
  })
})

test('an insert reads the held events as stored: none decoded, none re-rendered', async () => {
  const { createScenario } = await web()
  const store = await openScenarios(scratch())
  store.insert(createScenario('held'), order(T0, 'A', '82.5'))
  const file = path.join(store.dir, 'held.json')
  const stored = JSON.parse(fs.readFileSync(file, 'utf8'))
  stored.events[0] = { ...stored.events[0], price: '1', native: 'bm90IGFycm93' }
  fs.writeFileSync(file, JSON.stringify(stored))
  const added = store.insert(store.read('held'), order(T0 + 1n, 'B', '82.25'))
  assert.equal(added.crosscode, 'B')
  const after = store.read('held')
  assert.deepEqual(after.events[0], stored.events[0])
  assert.deepEqual(after.events[1], added)
})
