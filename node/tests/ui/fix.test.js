'use strict'

const assert = require('node:assert/strict')
const { test } = require('node:test')

const { byTag, installDOM } = require('./dom.js')
const adaptersPromise = import('../../ui/yggdryl.mjs')
const fixPromise = import('../../ui/fix.mjs')

const fixture = async ({ extraFields = [] } = {}) => {
  const { fixManifest } = await adaptersPromise
  const fields = [
    { t: 8, n: 'BeginString', y: 'utf8', d: 'Begin string' },
    { t: 9, n: 'BodyLength', y: 'int32', d: 'Body length' },
    { t: 10, n: 'CheckSum', y: 'utf8', d: 'Checksum' },
    { t: 35, n: 'MsgType', y: 'msgtype', d: 'Message type' },
    {
      t: 55,
      n: 'Symbol',
      y: 'utf8',
      d: 'Symbol',
      a: ['Ticker'],
      c: 2,
      x: 'Ticker symbol.',
    },
    { t: 453, n: 'NoPartyIDs', y: 'list', d: 'Parties', k: 'group', m: [448, 447] },
    { t: 448, n: 'PartyID', y: 'utf8', d: 'Party ID' },
    { t: 447, n: 'PartyIDSource', y: 'utf8', d: 'Party source' },
    ...extraFields,
  ]
  const index = {
    spec: {
      version: '5.0SP2',
      ep: 299,
      shards: 1,
      sources: [
        {
          id: 'FIX',
          url: 'https://example.test/fix',
          format: 'XML',
          version: '5.0SP2',
          sha256: '0123456789abcdef0123456789abcdef',
          license: 'https://example.test/license',
        },
      ],
    },
    kpi: {
      fields: fields.length,
      primitives: fields.length - 1,
      groups: 1,
      codes: 2,
      codeSets: 1,
      messages: 1,
      components: 0,
      lineage: 1,
      aliases: 1,
      alternates: 0,
      versions: 1,
      columns: 1,
      datatypes: 4,
      branches: 1,
      layoutGroups: 1,
      dtypes: [['utf8', fields.length - 3], ['int32', 1]],
      branchSizes: [['', fields.length]],
      versionList: ['5.0SP2'],
    },
    wire: { 453: { n: 'Parties', m: [448, 447] } },
    fields,
    messages: [
      {
        y: 'D',
        n: 'NewOrderSingle',
        i: 1,
        m: [
          ['f', 35, 1],
          ['f', 55, 1],
          ['g', 2, 0],
        ],
      },
    ],
    components: [],
    groups: [
      {
        n: 'Parties',
        i: 2,
        t: 453,
        m: [
          ['f', 448, 1],
          ['f', 447, 0],
        ],
      },
    ],
    header: [8, 9, 35],
    trailer: [10],
    projection: {
      columns: [{ c: '55', t: 55, n: 'Symbol', y: 'utf8', x: 'Ticker symbol.' }],
      carried: 0,
      call: 'reader.field()',
    },
    frames: [],
  }
  const details = {
    '55:': {
      c: [
        ['A', 'Alpha', 'First', '5.0', 0, ''],
        ['B', 'Beta', 'Second', '5.0', 0, ''],
      ],
      l: [['5.0', 'Symbol', 'String', '']],
    },
  }
  return { index, details, model: fixManifest({ index }) }
}

test('composeFixText counts UTF-8 bytes and keeps raw values separate from display', async () => {
  const { composeFixText, inspectFixText } = await fixPromise
  const { model } = await fixture()
  const composed = composeFixText({
    model,
    messageType: 'D',
    entries: [
      ['Ticker', 'CAFÉ '],
      [453, '2'],
      [448, 'P1'],
      [447, 'D'],
      [448, 'P2'],
      [447, 'D'],
    ],
    separator: '|',
  })

  const body = `35=D\x0155=CAFÉ \x01453=2\x01448=P1\x01447=D\x01448=P2\x01447=D\x01`
  assert.equal(composed.bodyLength, new TextEncoder().encode(body).length)
  assert.notEqual(composed.bodyLength, body.length)
  assert.ok(composed.wire.includes('\x01'))
  assert.ok(!composed.wire.includes('|'))
  assert.ok(composed.display.includes('|'))
  assert.equal(composed.pairs.find((entry) => entry.tag === 55).value, 'CAFÉ ')

  const read = inspectFixText({ model, text: `sending >> ${composed.display}` })
  assert.equal(read.direction, 'SENT')
  assert.equal(read.pairs.find((entry) => entry.tag === 55).value, 'CAFÉ ')
  assert.equal(read.messageType, 'D')
  assert.equal(read.unknown, 0)
  assert.deepEqual(read.checks.map((entry) => entry.ok), [true, true])
  const group = read.tree.find((entry) => entry.tag === 453)
  assert.equal(group.occurrences.length, 2)
  assert.deepEqual(
    group.occurrences.map((entry) => entry.members.map((member) => member.value)),
    [['P1', 'D'], ['P2', 'D']],
  )
})

test('composeFixText reports visible separator collisions without changing raw wire', async () => {
  const { composeFixText, inspectFixText } = await fixPromise
  const { model } = await fixture()
  const composed = composeFixText({
    model,
    messageType: 'D',
    entries: [[55, 'A|B  ']],
    separator: '|',
  })
  assert.deepEqual(composed.conflicts, ['55'])
  assert.ok(composed.wire.includes(`55=A|B  \x01`))
  const raw = inspectFixText({ model, text: composed.wire })
  assert.equal(raw.pairs.find((entry) => entry.tag === 55).value, 'A|B  ')
  assert.deepEqual(raw.checks.map((entry) => entry.ok), [true, true])
})

test('inspectFixText measures BodyLength after tag 9 and preserves space-delimited values', async () => {
  const { inspectFixText } = await fixPromise
  const { model } = await fixture({
    extraFields: [{ t: 58, n: 'Text', y: 'utf8', d: 'Text' }],
  })
  const malformed =
    '8=FIX.4.4\x019=10\x0149=X\x0135=D\x0155=A\x0110=207\x01'
  const reading = inspectFixText({ model, text: malformed })
  const bodyLength = reading.checks.find((check) => check.name === 'BodyLength (9)')
  const checkSum = reading.checks.find((check) => check.name === 'CheckSum (10)')
  assert.deepEqual(bodyLength, {
    name: 'BodyLength (9)',
    stated: '10',
    computed: '15',
    ok: false,
  })
  assert.equal(checkSum.computed, '207')
  assert.equal(checkSum.ok, true)

  const spaced = inspectFixText({
    model,
    text: '8=FIX.4.4 35=D 58=hello world 55=AAPL 10=000',
  })
  assert.equal(spaced.separator, ' ')
  assert.equal(spaced.pairs.find((entry) => entry.tag === 58).value, 'hello world')
  assert.equal(spaced.pairs.find((entry) => entry.tag === 55).value, 'AAPL')
})

test('FIX summary, sources, messages, and projection are reusable views', async () => {
  const {
    fixMessageExplorer,
    fixProjectionExplorer,
    fixRegistrySummary,
    fixSourceTable,
  } = await fixPromise
  const { model } = await fixture()
  const dom = installDOM()
  try {
    assert.match(fixRegistrySummary({ model }).textContent, /8fields/)
    assert.match(fixSourceTable({ model }).textContent, /FIXXML5\.0SP2/)
    const messages = fixMessageExplorer({ model })
    assert.equal(messages.show().y, 'D')
    assert.match(messages.summary.textContent, /NewOrderSingle/)
    const projection = fixProjectionExplorer({ model, pageSize: 1 })
    assert.match(projection.count.textContent, /1 of 1 columns/)
  } finally {
    dom.restore()
  }
})

test('field explorer caps rendering and loads field detail once on first open', async () => {
  const { fixFieldExplorer } = await fixPromise
  const extras = Array.from({ length: 5 }, (_, index) => ({
    t: 1000 + index,
    n: `Extra${index}`,
    y: 'utf8',
    d: `Extra ${index}`,
  }))
  const { model, details } = await fixture({ extraFields: extras })
  const dom = installDOM()
  let loads = 0
  try {
    const explorer = fixFieldExplorer({
      model,
      pageSize: 2,
      loadDetails: async () => {
        loads += 1
        return details
      },
    })
    assert.match(explorer.count.textContent, /2 drawn/)
    assert.ok(byTag(explorer.list, 'details').length <= 2)
    explorer.input.value = 'symbol'
    explorer.input.dispatchEvent({ type: 'input' })
    const field = byTag(explorer.list, 'details')[0]
    assert.equal(loads, 0)
    field.open = true
    field.dispatchEvent({ type: 'toggle' })
    await new Promise((resolve) => setImmediate(resolve))
    assert.equal(loads, 1)
    assert.match(field.textContent, /registry\.fieldByTag\(55\)/)
    field.open = false
    field.dispatchEvent({ type: 'toggle' })
    field.open = true
    field.dispatchEvent({ type: 'toggle' })
    assert.equal(loads, 1)
    await new Promise((resolve) => setImmediate(resolve))
  } finally {
    dom.restore()
  }
})

test('decode workbench labels manifest reading and runs an optional host explicitly', async () => {
  const { composeFixText, fixDecodeWorkbench } = await fixPromise
  const { model } = await fixture()
  const composed = composeFixText({ model, messageType: 'D', entries: [[55, 'AAPL']] })
  const dom = installDOM()
  let release
  const pendingHost = new Promise((resolve) => {
    release = resolve
  })
  let calls = 0
  try {
    const workbench = fixDecodeWorkbench({
      model,
      frames: [],
      value: composed.display,
      decode: async (text, reading) => {
        calls += 1
        assert.equal(text, composed.display)
        assert.equal(reading.messageType, 'D')
        await pendingHost
        return 'typed by host'
      },
    })
    assert.match(workbench.element.textContent, /Manifest reading:/)
    assert.match(workbench.element.textContent, /display-only boolean\/date glosses/)
    assert.match(workbench.element.textContent, /does not run native validation/)
    assert.equal(calls, 0)
    const running = workbench.decode()
    assert.equal(workbench.element.getAttribute('aria-busy'), 'true')
    release()
    assert.equal(await running, 'typed by host')
    assert.equal(calls, 1)
    assert.equal(workbench.host.textContent, 'typed by host')
    assert.equal(workbench.element.getAttribute('aria-busy'), 'false')
  } finally {
    dom.restore()
  }
})

test('decode corpus answers require the generated direction as well as the body', async () => {
  const { composeFixText, fixDecodeWorkbench } = await fixPromise
  const { model } = await fixture()
  const composed = composeFixText({ model, messageType: 'D', entries: [[55, 'AAPL']] })
  const dom = installDOM()
  try {
    const frame = {
      label: 'sent order',
      line: `sending >> ${composed.display}`,
      direction: 'SENT',
      columns: [{ t: 55, n: 'Symbol', y: 'utf8', v: 'AAPL' }],
      lift: [],
      anomalies: [],
      emitted: '',
      call: '',
    }
    const workbench = fixDecodeWorkbench({
      model,
      frames: [frame],
      value: `receiving << ${composed.display}`,
    })
    assert.doesNotMatch(workbench.view.textContent, /What the package answered/)
    workbench.setValue(`sending >> ${composed.display}`)
    assert.match(workbench.view.textContent, /What the package answered/)
    assert.match(workbench.view.textContent, /1 of 1/)
  } finally {
    dom.restore()
  }
})

test('decode uses display-only glosses and caps DOM creation before formatting rows', async () => {
  const { fixDecodeWorkbench } = await fixPromise
  const { model } = await fixture({
    extraFields: [
      { t: 43, n: 'PossDupFlag', y: 'boolean', d: 'Possible duplicate' },
      { t: 52, n: 'SendingTime', y: 'datetime64(ns,"UTC")', d: 'Sending time' },
    ],
  })
  const dom = installDOM()
  const originalCreate = dom.document.createElement.bind(dom.document)
  let rowsCreated = 0
  dom.document.createElement = (tag) => {
    if (String(tag).toLowerCase() === 'tr') rowsCreated += 1
    return originalCreate(tag)
  }
  try {
    const values = Array.from({ length: 1_000 }, (_, index) => `55=X${index}`).join('|')
    const workbench = fixDecodeWorkbench({
      model,
      value: `8=FIX.4.4|35=D|43=Y|52=20240201-12:34:56|${values}|`,
      pageSize: 2,
    })
    assert.match(workbench.view.textContent, /1.004 of 1.004 pairs; 2 drawn, 1.002 not drawn/)
    assert.equal(byTag(workbench.view, 'tr').length, 3)
    assert.ok(rowsCreated < 20, `created ${rowsCreated} table rows for a two-row page`)
    const filter = byTag(workbench.view, 'input')[0]
    filter.value = '43'
    filter.dispatchEvent({ type: 'input' })
    assert.match(workbench.view.textContent, /true/)
    filter.value = '52'
    filter.dispatchEvent({ type: 'input' })
    assert.match(workbench.view.textContent, /2024-02-01T12:34:56Z/)
    filter.value = ''
    filter.dispatchEvent({ type: 'input' })
    const more = byTag(workbench.view, 'button').find((node) => /Show 2 more/.test(node.textContent))
    assert.ok(more)
    more.dispatchEvent({ type: 'click' })
    assert.equal(byTag(workbench.view, 'tr').length, 5)
  } finally {
    dom.restore()
  }
})

test('decode and validation errors are discarded after their input changes', async () => {
  const { composeFixText, fixDecodeWorkbench, fixEncodeWorkbench } = await fixPromise
  const { model } = await fixture()
  const composed = composeFixText({ model, messageType: 'D', entries: [[55, 'AAPL']] })
  const dom = installDOM()
  let rejectDecode
  let rejectValidation
  try {
    const decoder = fixDecodeWorkbench({
      model,
      frames: [],
      value: composed.display,
      decode: () =>
        new Promise((_, reject) => {
          rejectDecode = reject
        }),
    })
    const decoding = decoder.decode()
    decoder.setValue(composed.display.replace('AAPL', 'MSFT'))
    assert.equal(decoder.element.getAttribute('aria-busy'), 'false')
    assert.equal(decoder.host.textContent, '')
    rejectDecode(new Error('stale decode error'))
    assert.equal(await decoding, null)
    assert.equal(decoder.error.hidden, true)
    assert.doesNotMatch(decoder.error.textContent, /stale decode error/)

    const encoder = fixEncodeWorkbench({
      model,
      values: new Map([[55, 'AAPL']]),
      validate: () =>
        new Promise((_, reject) => {
          rejectValidation = reject
        }),
    })
    const validating = encoder.validate()
    encoder.setValue(55, 'MSFT')
    assert.equal(encoder.element.getAttribute('aria-busy'), 'false')
    rejectValidation(new Error('stale validation error'))
    assert.equal(await validating, null)
    assert.equal(encoder.error.hidden, true)
    assert.doesNotMatch(encoder.error.textContent, /stale validation error/)
  } finally {
    dom.restore()
  }
})

test('decode detail enrichment preserves the current host error', async () => {
  const { fixDecodeWorkbench } = await fixPromise
  const { model, details } = await fixture()
  const dom = installDOM()
  let resolveDetails
  const detailGate = new Promise((resolve) => {
    resolveDetails = resolve
  })
  try {
    const decoder = fixDecodeWorkbench({
      model,
      loadDetails: () => detailGate,
      value: '8=FIX.4.4|35=D|55=AAPL|',
      decode: async () => {
        throw new Error('authoritative decode failed')
      },
    })
    assert.equal(await decoder.decode(), null)
    assert.equal(decoder.error.hidden, false)
    assert.match(decoder.error.textContent, /authoritative decode failed/)
    assert.equal(dom.document.activeElement, decoder.error)
    resolveDetails(details)
    await new Promise((resolve) => setImmediate(resolve))
    assert.equal(decoder.error.hidden, false)
    assert.match(decoder.error.textContent, /authoritative decode failed/)
    assert.equal(dom.document.activeElement, decoder.error)
  } finally {
    dom.restore()
  }
})

test('encode workbench exposes raw SOH wire and sends that exact draft to callbacks', async () => {
  const { fixEncodeWorkbench } = await fixPromise
  const { model, details } = await fixture()
  const dom = installDOM()
  let decoded = null
  let validated = null
  try {
    const workbench = fixEncodeWorkbench({
      model,
      loadDetails: details,
      values: new Map([[55, 'AAPL  ']]),
      onDecode: (wire) => {
        decoded = wire
      },
      validate: async (wire) => {
        validated = wire
        return 'valid package answer'
      },
    })
    assert.match(workbench.element.textContent, /Browser draft:/)
    assert.ok(workbench.draft.wire.includes('\x01'))
    assert.ok(workbench.draft.display.includes('|'))
    assert.equal(workbench.draft.pairs.find((entry) => entry.tag === 55).value, 'AAPL  ')
    await new Promise((resolve) => setImmediate(resolve))
    assert.ok(
      byTag(workbench.form, 'option').some((option) =>
        option.textContent.includes('AAPL   · not in the generated code set'),
      ),
    )
    const buttons = byTag(workbench.element, 'button')
    buttons.find((node) => /Read it/.test(node.textContent)).dispatchEvent({ type: 'click' })
    assert.equal(decoded, workbench.draft.wire)
    assert.equal(await workbench.validate(), 'valid package answer')
    assert.equal(validated, workbench.draft.wire)
  } finally {
    dom.restore()
  }
})

test('encode detail enrichment preserves unchanged validation and field focus', async () => {
  const { fixEncodeWorkbench } = await fixPromise
  const { model, details } = await fixture()
  const dom = installDOM()
  let resolveDetails
  let resolveValidation
  const detailGate = new Promise((resolve) => {
    resolveDetails = resolve
  })
  const validationGate = new Promise((resolve) => {
    resolveValidation = resolve
  })
  try {
    const encoder = fixEncodeWorkbench({
      model,
      loadDetails: () => detailGate,
      values: { Symbol: 'AAPL' },
      validate: () => validationGate,
    })
    const initialField = [...byTag(encoder.form, 'input'), ...byTag(encoder.form, 'select')].find(
      (node) => node.value === 'AAPL',
    )
    assert.ok(initialField)
    initialField.focus()
    const wire = encoder.draft.wire
    const validating = encoder.validate()
    assert.equal(encoder.element.getAttribute('aria-busy'), 'true')
    assert.equal(encoder.status.textContent, 'Validating…')

    resolveDetails(details)
    await new Promise((resolve) => setImmediate(resolve))
    const enrichedField = [...byTag(encoder.form, 'input'), ...byTag(encoder.form, 'select')].find(
      (node) => node.value === 'AAPL',
    )
    assert.ok(enrichedField)
    assert.equal(dom.document.activeElement, enrichedField)
    assert.equal(encoder.draft.wire, wire)
    assert.equal(encoder.element.getAttribute('aria-busy'), 'true')
    assert.equal(encoder.status.textContent, 'Validating…')

    resolveValidation('authoritative validation')
    assert.equal(await validating, 'authoritative validation')
    assert.equal(encoder.status.textContent, 'authoritative validation')
    assert.equal(encoder.element.getAttribute('aria-busy'), 'false')
    assert.equal(dom.document.activeElement, enrichedField)
  } finally {
    dom.restore()
  }
})

test('encode workbench resolves named and optional configured fields without changing values', async () => {
  const { fixEncodeWorkbench } = await fixPromise
  const { model } = await fixture({
    extraFields: [{ t: 58, n: 'Text', y: 'utf8', d: 'Free text' }],
  })
  const dom = installDOM()
  try {
    const workbench = fixEncodeWorkbench({
      model,
      values: { Symbol: 'MSFT  ', Text: 'hello world' },
    })
    assert.equal(workbench.draft.pairs.find((entry) => entry.tag === 55).value, 'MSFT  ')
    assert.equal(workbench.draft.pairs.find((entry) => entry.tag === 58).value, 'hello world')
    assert.throws(
      () => fixEncodeWorkbench({ model, values: { NotAField: 'x' } }),
      /unknown FIX field NotAField/,
    )
  } finally {
    dom.restore()
  }
})

test('detail asset failures are visible and accept recursive custom content', async () => {
  const { fixDecodeWorkbench, fixEncodeWorkbench, fixFieldExplorer } = await fixPromise
  const { model } = await fixture()
  const dom = installDOM()
  let rendered = 0
  const loadDetails = async () => {
    throw new Error('detail offline')
  }
  const renderDetailsError = (error) => {
    rendered += 1
    return ['Run generator: ', error.message]
  }
  try {
    const fields = fixFieldExplorer({ model, loadDetails, renderDetailsError })
    fields.input.value = 'symbol'
    fields.input.dispatchEvent({ type: 'input' })
    const field = byTag(fields.list, 'details')[0]
    field.open = true
    field.dispatchEvent({ type: 'toggle' })

    const decoder = fixDecodeWorkbench({
      model,
      loadDetails,
      renderDetailsError,
      value: '8=FIX.4.4|35=D|55=AAPL|',
    })
    const encoder = fixEncodeWorkbench({ model, loadDetails, renderDetailsError })
    await new Promise((resolve) => setImmediate(resolve))
    await new Promise((resolve) => setImmediate(resolve))
    assert.match(field.textContent, /Run generator: detail offline/)
    assert.match(decoder.element.textContent, /Run generator: detail offline/)
    assert.match(encoder.element.textContent, /Run generator: detail offline/)
    assert.equal(rendered, 3)
  } finally {
    dom.restore()
  }
})

const fieldDocument = (tag, name, description = '') => ({
  dtype: { type: 'utf8' },
  metadata: { 'fix:tag': String(tag), description },
  name,
  nullable: true,
})

test('registry editor is read-only over a compact manifest', async () => {
  const { fixRegistryEditor } = await fixPromise
  const { model } = await fixture()
  const dom = installDOM()
  try {
    const editor = fixRegistryEditor({ model, pageSize: 2 })
    assert.equal(editor.editor, null)
    assert.equal(editor.fields, null)
    assert.equal(editor.stale, false)
    assert.match(editor.element.textContent, /Read-only generated manifest/)
    assert.doesNotMatch(editor.element.textContent, /Insert \/ replace/)
  } finally {
    dom.restore()
  }
})

test('registry editor routes exact actions and adopts only the authoritative list', async () => {
  const { fixRegistryEditor } = await fixPromise
  const dom = installDOM()
  const calls = []
  let authoritative = [fieldDocument(55, 'symbol', 'before')]
  let releaseInsert
  const insertGate = new Promise((resolve) => {
    releaseInsert = resolve
  })
  try {
    const editor = fixRegistryEditor({
      fields: authoritative,
      actions: {
        list: async () => {
          calls.push(['list'])
          return authoritative
        },
        insert: async (field) => {
          calls.push(['insert', field])
          await insertGate
        },
        update: async (field) => {
          calls.push(['update', field])
        },
        removeById: async (id) => {
          calls.push(['removeById', id])
        },
      },
      pageSize: 1,
    })
    const removeButton = byTag(editor.element, 'button').find((node) => node.textContent === 'Remove')
    assert.match(removeButton.className, /ygg-ui__button--danger/)

    assert.equal(editor.select('55:'), true)
    const changed = fieldDocument(55, 'symbol', '  exact whitespace  ')
    editor.editor.value = JSON.stringify(changed)
    authoritative = [changed]
    assert.equal(await editor.update(), true)
    assert.deepEqual(calls[0], ['update', changed])
    assert.deepEqual(calls[1], ['list'])
    assert.equal(editor.fields[0].metadata.description, '  exact whitespace  ')

    const inserted = fieldDocument(56, 'target')
    editor.editor.value = JSON.stringify(inserted)
    const running = editor.insert()
    assert.equal(editor.element.getAttribute('aria-busy'), 'true')
    assert.equal(await editor.insert(), false)
    assert.equal(editor.fields.length, 1)
    authoritative = [changed, inserted]
    releaseInsert()
    assert.equal(await running, true)
    assert.equal(editor.fields.length, 2)

    assert.equal(editor.select('56:'), true)
    authoritative = [changed]
    assert.equal(await editor.remove(), true)
    assert.deepEqual(calls.find((entry) => entry[0] === 'removeById'), ['removeById', '56:'])
    assert.equal(editor.fields.length, 1)
  } finally {
    dom.restore()
  }
})

test('registry editor preserves input and state when a host mutation fails', async () => {
  const { fixRegistryEditor } = await fixPromise
  const dom = installDOM()
  const original = fieldDocument(55, 'symbol', 'before')
  try {
    const editor = fixRegistryEditor({
      fields: [original],
      actions: {
        list: async () => [fieldDocument(55, 'symbol', 'should not load')],
        insert: async () => {},
        update: async () => {
          throw new Error('native conflict')
        },
        removeById: async () => {},
      },
    })
    editor.select('55:')
    const draft = JSON.stringify(fieldDocument(55, 'symbol', '  retained  '), null, 2)
    editor.editor.value = draft
    assert.equal(await editor.update(), false)
    assert.equal(editor.editor.value, draft)
    assert.deepEqual(editor.fields[0], original)
    assert.equal(editor.error.hidden, false)
    assert.match(editor.error.textContent, /native conflict/)
    assert.equal(editor.editor.getAttribute('aria-invalid'), 'true')
    assert.equal(dom.document.activeElement, editor.error)
  } finally {
    dom.restore()
  }
})

test('registry editor locks a stale snapshot after a committed mutation until refresh succeeds', async () => {
  const { fixRegistryEditor } = await fixPromise
  const dom = installDOM()
  const original = fieldDocument(55, 'symbol', 'before')
  let mutations = 0
  let reloads = 0
  const updated = fieldDocument(55, 'symbol', 'after')
  try {
    const editor = fixRegistryEditor({
      fields: [original],
      actions: {
        list: async () => {
          reloads += 1
          if (reloads === 1) throw new Error('reload unavailable')
          return [updated]
        },
        insert: async () => {},
        update: async () => {
          mutations += 1
        },
        removeById: async () => {},
      },
    })
    editor.select('55:')
    editor.editor.value = JSON.stringify(updated)
    assert.equal(await editor.update(), true)
    assert.equal(mutations, 1)
    assert.equal(editor.stale, true)
    assert.deepEqual(editor.fields[0], original)
    assert.match(editor.error.textContent, /mutation committed/i)
    assert.match(editor.status.textContent, /snapshot is unchanged and stale/i)
    assert.equal(editor.editor.disabled, true)
    assert.equal(editor.editor.getAttribute('aria-invalid'), 'false')
    assert.equal(editor.editor.hasAttribute('aria-describedby'), false)
    assert.equal(editor.status.getAttribute('aria-describedby'), editor.error.id)
    const stalePanel = byTag(editor.listing.element, 'details')[0]
    stalePanel.open = true
    stalePanel.dispatchEvent({ type: 'toggle' })
    const staleEdit = byTag(stalePanel, 'button').find((node) => /^Edit /.test(node.textContent))
    assert.equal(staleEdit.disabled, true)
    assert.equal(await editor.update(), false)
    assert.equal(mutations, 1)
    const retry = byTag(editor.element, 'button').find((node) =>
      /Retry authoritative refresh/.test(node.textContent),
    )
    assert.equal(retry.hidden, false)
    assert.deepEqual(await editor.refresh(), [updated])
    assert.equal(editor.stale, false)
    assert.equal(editor.editor.disabled, false)
    assert.deepEqual(editor.fields, [updated])
    assert.match(editor.editor.value, /"description": "after"/)
    assert.equal(retry.hidden, true)
  } finally {
    dom.restore()
  }
})

test('registry editor rejects selection during a pending mutation and reconciles refreshes', async () => {
  const { fixRegistryEditor } = await fixPromise
  const dom = installDOM()
  const symbol = fieldDocument(55, 'symbol', 'before')
  const side = fieldDocument(54, 'side', 'other')
  let authoritative = [symbol, side]
  let release
  const gate = new Promise((resolve) => {
    release = resolve
  })
  try {
    const editor = fixRegistryEditor({
      fields: authoritative,
      actions: {
        list: async () => authoritative,
        insert: async () => {},
        update: async (field) => {
          await gate
          authoritative = [field, side]
        },
        removeById: async () => {},
      },
    })
    assert.equal(editor.select('55:'), true)
    const changed = fieldDocument(55, 'symbol', 'changed')
    editor.editor.value = JSON.stringify(changed)
    const running = editor.update()
    assert.equal(editor.editor.disabled, true)
    const pendingPanel = byTag(editor.listing.element, 'details')[0]
    pendingPanel.open = true
    pendingPanel.dispatchEvent({ type: 'toggle' })
    const pendingEdit = byTag(pendingPanel, 'button').find((node) => /^Edit /.test(node.textContent))
    assert.equal(pendingEdit.disabled, true)
    assert.equal(editor.select('54:'), false)
    assert.equal(editor.selectedId, '55:')
    release()
    assert.equal(await running, true)
    assert.equal(editor.selectedId, '55:')
    assert.match(editor.editor.value, /"description": "changed"/)

    authoritative = [side]
    assert.deepEqual(await editor.refresh(), [side])
    assert.equal(editor.selectedId, null)
    assert.equal(editor.editor.value, '')
  } finally {
    dom.restore()
  }
})

test('registry editor exposes copies and rejects ambiguous configuration', async () => {
  const { fixRegistryEditor } = await fixPromise
  const { model } = await fixture()
  const dom = installDOM()
  const original = fieldDocument(55, 'symbol')
  try {
    const editor = fixRegistryEditor({ fields: [original] })
    const exposed = editor.fields
    exposed[0].name = 'changed outside'
    exposed.length = 0
    assert.equal(editor.fields.length, 1)
    assert.equal(editor.fields[0].name, 'symbol')
    assert.throws(() => fixRegistryEditor({}), /read-only registry needs model or fields/)
    assert.throws(
      () => fixRegistryEditor({ model, fields: [original] }),
      /separate editor modes/,
    )
  } finally {
    dom.restore()
  }
})
