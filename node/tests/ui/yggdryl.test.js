'use strict'

const assert = require('node:assert/strict')
const { test } = require('node:test')

const { byClass, byTag, installDOM } = require('./dom.js')
const adapterPromise = import('../../ui/yggdryl.mjs')

const manifest = () => ({
  wire: { 453: { n: 'Parties', m: [448, 447] } },
  fields: [
    { t: 35, n: 'MsgType', d: 'Message type', y: 'msgtype' },
    { t: 55, n: 'Symbol', d: 'Symbol', a: ['Ticker'], y: 'utf8' },
    { t: 55, n: 'VendorSymbol', d: 'Venue symbol', b: 'venue', y: 'utf8' },
    { t: 453, n: 'NoPartyIDs', d: 'Parties', k: 'group', y: 'list', x: 'Party entries.' },
    { t: 448, n: 'PartyID', y: 'utf8' },
    { t: 447, n: 'PartyIDSource', y: 'utf8' },
  ],
  messages: [
    { y: 'D', n: 'NewOrderSingle', i: 1, m: [['c', 10, 1], ['g', 20, 0]] },
    { y: '8', n: 'ExecutionReport', i: 2, m: [['f', 55, 1]] },
  ],
  components: [{ n: 'Instrument', i: 10, m: [['f', 55, 1]] }],
  groups: [{ n: 'Parties', i: 20, t: 453, m: [['f', 448, 1], ['f', 447, 0]] }],
  header: [35],
  trailer: [10],
})

test('datatype adapter displays the package string without parsing it', async () => {
  const held = installDOM()
  try {
    const { dataType } = await adapterPromise
    const view = dataType({ value: 'decimal128(20, 8)', className: 'money' })
    assert.equal(view.textContent, 'decimal128(20, 8)')
    assert.match(view.className, /ygg-ui__datatype money/)
  } finally {
    held.restore()
  }
})

test('field documents expose facts and build nested fields only when opened', async () => {
  const held = installDOM()
  try {
    const { fieldDocument } = await adapterPromise
    const document = {
      name: 'quote',
      nullable: false,
      metadata: { display: 'Quote' },
      dtype: {
        type: 'struct',
        fields: [
          { name: 'symbol', nullable: false, dtype: { type: 'utf8' } },
          { field: { name: 'price', nullable: true, dtype: { type: 'float64' } }, type_id: 1 },
        ],
      },
    }
    const view = fieldDocument({ document })
    assert.equal(byTag(view, 'details').length, 1)
    const root = byTag(view, 'details')[0]
    root.open = true
    root.dispatchEvent('toggle')
    assert.equal(byTag(view, 'details').length, 3)
    assert.match(view.textContent, /quote.*required.*struct.*metadata\.display.*Quote/)
    assert.match(view.textContent, /symbol.*price/)
  } finally {
    held.restore()
  }
})

test('FIX manifest indexes names once and gives the standard branch tag precedence', async () => {
  const held = installDOM()
  try {
    const { fixManifest } = await adapterPromise
    const index = manifest()
    const model = fixManifest({ index, details: { '55:': { c: [['A', 'Apple']] } } })
    assert.strictEqual(model.data, index)
    assert.equal(model.field(55).n, 'Symbol')
    assert.equal(model.field('#55').n, 'Symbol')
    assert.equal(model.field('ticker').t, 55)
    assert.equal(model.field('message_type').t, 35)
    assert.equal(model.field('missing'), null)
    assert.equal(model.searchIndex.search('ticker').length, 1)
    assert.equal(model.detail(model.field(55)).c[0][0], 'A')
    assert.equal(model.detail('55:', { '55:': { l: [['4.4']] } }).l.length, 1)
  } finally {
    held.restore()
  }
})

test('FIX manifest builds message group scopes and carrier indexes lazily once', async () => {
  const held = installDOM()
  try {
    const { fixManifest } = await adapterPromise
    const model = fixManifest({ index: manifest() })
    const groups = model.groupsOf('D')
    assert.deepEqual(groups.get(453), { n: 'Parties', m: [448, 447] })
    assert.strictEqual(model.groupsOf('D'), groups)
    assert.equal(model.groupsOf('missing').size, 0)

    const tags = model.layoutTags('D')
    assert.deepEqual(tags.map(({ tag, required }) => [tag, required]), [
      [55, true],
      [453, false],
      [448, false],
      [447, false],
    ])
    const first = model.carriers(55)
    const second = model.carriers(55)
    assert.deepEqual(first.map((message) => message.y), ['D', '8'])
    assert.strictEqual(first, second)
  } finally {
    held.restore()
  }
})

test('FIX layout tree is recursive, lazy and marks field semantics', async () => {
  const held = installDOM()
  try {
    const { fixManifest } = await adapterPromise
    const model = fixManifest({ index: manifest() })
    const view = model.layoutTree(model.messages.get('D').m)
    assert.equal(byTag(view, 'details').length, 2)
    const component = byTag(view, 'details')[0]
    component.open = true
    component.dispatchEvent('toggle')
    assert.match(component.textContent, /55Symbol.*utf8.*required/)
    const group = byTag(view, 'details')[1]
    group.open = true
    group.dispatchEvent('toggle')
    assert.match(group.textContent, /Party entries\./)
    assert.ok(byClass(group, 'ygg-ui__pill--required').length >= 1)
  } finally {
    held.restore()
  }
})
