'use strict'

// The `fix` suite, in its own block: it brings its own
// fixtures, and `const` is block-scoped.
{
  // The FIX boundary: the typed vocabulary on the protocol view, the registry,
  // and the message. Every answer here is the core's; what these check is the
  // crossing - the key coercion, the tag width, the error class each refusal
  // arrives as, the storage locations a JavaScript caller names, and the
  // language protocols the loader wires over the native halves.
  //
  // An identifier crosses as a number - the digest the core derives from a
  // tag and a name - so there is no class for it, and a dictionary's membership
  // is a list of names on the field it contributed to; every refusal is the
  // native one.

  const assert = require('node:assert/strict')
  const fs = require('node:fs')
  const os = require('node:os')
  const path = require('node:path')
  const test = require('node:test')

  const { DataType, Field, IOBase, MimeType, Scalar, TextLine, Url, fields, fix, xxhash } = require('yggdryl')

  const SEED = path.join(__dirname, '..', '..', 'config', 'fix')


  // A codec that reads every message type. The corpora below are captures, and
  // a capture holds the session traffic and the bridge rows stating no type
  // that `DEFAULT_REFUSED_MSGTYPES` drop; a case about the refusals says so for
  // itself.
  function reading(registry, options) {
    return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
  }

  // The crate's own definitions, which every registry holds from construction
  // beside the seeded SendingTime (52) and TransactTime (60) clocks. A
  // definition is filed by the shape it has: the columns are scalar fields,
  // the `metadata` Map is a group, and the source UUID list is a registered
  // scalar column.
  const CRATE = fix.crateFields()
  // Which category a definition lands in is the core's answer, not a shape a
  // test guesses: a snapshot states the three, so the fields it lists are the
  // fields, whatever datatype each carries.
  const CRATE_SCALAR_NAMES = new Set(new fix.FixRegistry().toJSON().fields.map((field) => field.name))
  const CRATE_SCALARS = CRATE.filter((field) => CRATE_SCALAR_NAMES.has(field.name))
  const CRATE_GROUPS = CRATE.filter((field) => !CRATE_SCALAR_NAMES.has(field.name))
  // The crate's scalar tags, in the registry's own order.
  const CRATE_TAGS = CRATE_SCALARS.map((field) => field.fix.tag)
  // What a new registry holds before anything is inserted: every crate
  // definition and the two seeded clocks.
  const SEEDED = 2 + CRATE.length
  // The scalar fields the committed dictionary stores.
  const STORED = 6241
  // The named code sets it stores beside them, one per vocabulary however many
  // fields read by it.
  const CODESETS = 736

  /**
 * The scalar fields a registry holds, and the definitions behind them.
 *
 * The walk answers the scalars first, so the snapshot's own count is where
 * the definitions start.
 */
  function scalars(registry) {
    return [...registry].slice(0, registry.toJSON().fields.length)
  }

  function definitionsOf(registry) {
    return [...registry].slice(registry.toJSON().fields.length)
  }
  // The one intake clock the Rust suites read undated bytes under
  // (`fixed_codec` in `rust/tests/fix.rs`): 2024-01-02T10:15:30Z. Without it an
  // undated message reads UTC now, which is deliberately not deterministic.
  const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)

  function fixedCodec(registry, options = {}) {
    return reading(registry, { ...options, defaultSendingTime: SENDING })
  }

  function scratch() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fix-'))
  }

  let seedRegistry
  function seed() {
    seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
    return seedRegistry.clone()
  }

  function fixField(name, dtype, tag, { branches, tags, names, description } = {}) {
    const field = Field.from(`${name}: ${dtype}`)
    field.fix.tag = tag
    if (branches) field.fix.branches = branches
    if (tags) field.fix.tags = tags
    if (names) field.fix.names = names
    if (description !== undefined) field.fix.description = description
    return field
  }

  test('the protocol view carries the typed fix vocabulary', () => {
    const field = Field.from('OrderQty: decimal128(20, 8)')
    field.fix.tag = 38
    field.fix.tags = [1088]
    field.fix.names = ['Qty', 'Quantity']
    field.fix.msgcat = 'ORDR'
    field.fix.description = 'Quantity ordered.'

    assert.equal(field.fix.tag, 38)
    assert.deepEqual(field.fix.tags, [1088])
    assert.deepEqual(field.fix.names, ['Qty', 'Quantity'])
    assert.equal(field.fix.msgcat, 'ORDR')
    assert.equal(field.fix.description, 'Quantity ordered.')
    // Ordinary namespaced text, in the one metadata map: a list is the
    // compact JSON array it is.
    assert.equal(field.get('FIX:names'), '["Qty","Quantity"]')
    assert.equal(field.get('FIX:tags'), '[1088]')
    assert.equal(field.fix.get('tag'), '38')
    // Four: a description is a fact about the column rather than a
    // FIX fact, so it lives on the generic key every catalog reads.
    assert.equal(field.get('description'), 'Quantity ordered.')
    assert.equal(field.has('FIX:description'), false)
    assert.equal(field.fix.size, 4)

    const before = field.toJSON()
    assert.throws(() => {
      field.fix.msgcat = 'order'
    })
    assert.deepEqual(field.toJSON(), before, 'a refused category is atomic')
    field.fix.msgcat = null
    assert.equal(field.fix.msgcat, null)

    // An empty array removes a list property; `delete` removes any of them.
    field.fix.tags = []
    assert.deepEqual(field.fix.tags, [])
    assert.equal(field.fix.has('tags'), false)
    field.fix.names = []
    assert.deepEqual(field.fix.names, [])
    assert.equal(field.fix.delete('tag'), true)
    assert.equal(field.fix.tag, null)

    const absent = Field.from('Symbol: utf8')
    assert.equal(absent.fix.tag, null)
    assert.deepEqual(absent.fix.tags, [])
    assert.deepEqual(absent.fix.names, [])
    assert.equal(absent.fix.description, null)
  })

  test('the typed vocabulary answers only on the fix view', () => {
    const field = Field.from('Symbol: utf8')
    field.fix.tag = 55

    for (const [view, scheme] of [
      [field.http, 'http'],
      [field.iceberg, 'iceberg'],
      [field.protocol('parquet'), 'parquet'],
    ]) {
      assert.throws(() => view.tag, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.tags, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.names, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.identifiers, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.description, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.branches, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.hasBranch('cme'), { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.id, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.directions, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.msgcat, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.codeset, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.codeset = 'sidecodeset'
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.tag = 55
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.directions = [{ code: 'S', patterns: ['^TX '] }]
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.msgcat = 'ORDR'
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.derivation, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.derivation = 'orderqty - cumqty'
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.names = ['Ticker']
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.identifiers = []
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => {
        view.branches = ['cme']
      }, { name: 'TypeError', message: new RegExp(scheme) })
      assert.throws(() => view.addBranch('cme'), { name: 'TypeError', message: new RegExp(scheme) })
    }
    // The Map-like surface still works on every view, this one included.
    assert.equal(field.protocol('fix').get('tag'), '55')
  })

  test('identifier declarations resolve aliases and decimal tags into direct member order', () => {
    const client = fixField('clordid', 'utf8', 11, { names: ['ClientOrder'] })
    const order = fixField('orderid', 'utf8', 37)
    const declaration = fields.struct('order', [client, order], { nullable: false })
    const view = declaration.fix
    assert.deepEqual(view.identifiers, [])
    view.identifiers = ['37', 'ClientOrder']
    assert.deepEqual(view.identifiers, ['clordid', 'orderid'])
    assert.equal(declaration.get('FIX:identifiers'), 'clordid,orderid')
    assert.deepEqual(Field.fromJSON(declaration.toJSON()).fix.identifiers, ['clordid', 'orderid'])
    view.identifiers = ['ORDERID']
    assert.deepEqual(view.identifiers, ['orderid'])
    view.identifiers = []
    assert.deepEqual(view.identifiers, [])
    assert.equal(declaration.has('FIX:identifiers'), false)
  })

  test('identifier declaration refusals leave the entire field unchanged', () => {
    const client = fixField('clordid', 'utf8', 11)
    const nested = fields.struct('nested', [Field.from('child: utf8')])
    const declaration = fields.struct('order', [client, nested], { nullable: false })
    declaration.fix.identifiers = ['clordid']
    const before = declaration.toJSON()
    for (const invalid of [[''], ['absent'], ['nested'], ['nested.child'], ['clordid,orderid'], ['clordid', '11']]) {
      assert.throws(() => { declaration.fix.identifiers = invalid }, /order.fix:identifiers/)
      assert.deepEqual(declaration.toJSON(), before)
    }
    for (const invalid of [
      'clordid', 11, null, ['clordid', 11], new Set(['clordid']),
      (function* () { yield 'clordid' })(), [, 'clordid'],
    ]) {
      assert.throws(() => { declaration.fix.identifiers = invalid })
      assert.deepEqual(declaration.toJSON(), before)
    }
  })

  test('a derivation crosses as canonical text', () => {
    // One term over the message's fields, stored as its canonical spelling;
    // null removes it, and a text that is not a term throws.
    const field = new Field('leavesqty', 'float64')
    field.fix.tag = 151
    assert.equal(field.fix.derivation, null)

    field.fix.derivation = 'orderqty-cumqty'
    assert.equal(field.fix.derivation, 'orderqty - cumqty')
    assert.equal(field.get('FIX:derivation'), 'orderqty - cumqty')

    assert.throws(() => {
      field.fix.derivation = 'orderqty -'
    })
    assert.equal(field.fix.derivation, 'orderqty - cumqty')

    // An edited derivation is what the reader fills by.
    const registry = fix.FixRegistry.fromHandle(SEED)
    const leaves = registry.getFieldByTag(151)
    leaves.fix.derivation = "case when msgtype in ('8', '9') then orderqty * 2 end"
    registry.update(leaves)
    const codec = fixedCodec(registry)
    const held = codec.parseLine(Buffer.from('8=FIX.4.4|35=8|37=A|38=100|14=0|10=0|')).next().value
    // An exact column renders as its own text, at the one scale this crate
    // keeps a number at.
    assert.equal(held.byTag(151).toJSON(), '200.000000000000000000')

    field.fix.derivation = null
    assert.equal(field.fix.derivation, null)
    assert.equal(field.has('FIX:derivation'), false)
  })

  test('direction rules cross as a typed list', () => {
    const field = new Field('MsgDirection', 'utf8')
    field.fix.tag = 385
    assert.deepEqual(field.fix.directions, [])

    // One record per code of the set, the patterns decoded, on tag 385.
    const rules = [
      { code: 'S', patterns: ['(?i)^TX\\b'] },
      { code: 'R', patterns: ['(?i)^RX\\b'] },
    ]
    field.fix.directions = rules
    assert.deepEqual(field.fix.directions, rules)
    // The stored text is the canonical document, backslashes escaped.
    assert.equal(
      field.get('FIX:directions'),
      '[{"code":"S","patterns":["(?i)^TX\\\\b"]},' +
        '{"code":"R","patterns":["(?i)^RX\\\\b"]}]',
    )
    assert.deepEqual(JSON.parse(field.get('FIX:directions')), rules)

    // A codec compiles the rules of the dictionary it is built over, once,
    // and the line door fills tag 385 from them; the verb table no longer
    // applies under a stated table.
    const registry = new fix.FixRegistry()
    registry.insert(field)
    const codec = reading(registry)
    const read = (line) => codec.parseLine(Buffer.from(line)).next().value
    assert.equal(read('TX 8=FIX.4.4|35=D|10=0|').byTag(385).asJs(), 'S')
    assert.equal(read('RX 8=FIX.4.4|35=D|10=0|').byTag(385).asJs(), 'R')
    assert.equal(read('sending >> 8=FIX.4.4|35=D|10=0|').getByTag(385), null)

    // A pattern the regex crate refuses is refused whole, the field unchanged.
    assert.throws(() => {
      field.fix.directions = [{ code: 'S', patterns: ['('] }]
    }, /valid byte regex/)
    assert.deepEqual(field.fix.directions, rules)

    // An empty array removes the property.
    field.fix.directions = []
    assert.deepEqual(field.fix.directions, [])
    assert.equal(field.has('FIX:directions'), false)
    assert.deepEqual(new Field('MsgDirection', 'utf8').fix.directions, [])
  })

  test('a field names the code set it reads by, and the dictionary holds it', () => {
    const registry = new fix.FixRegistry()
    assert.deepEqual(registry.codesetNames(), ['msgcatcodeset'])

    // The set is stated first: a dictionary refuses a field naming a
    // vocabulary nothing states, so the members exist before a field points
    // at them.
    registry.setCodeset('sidecodeset', [
      { value: '1', name: 'Buy' },
      { value: '2', name: 'Sell', aliases: ['Sold'], doc: 'Sell side', group: 'Outright' },
    ])
    assert.deepEqual(registry.codesetNames(), ['msgcatcodeset', 'sidecodeset'])

    const side = fixField('Side', 'utf8', 54)
    assert.equal(side.fix.codeset, null)
    side.fix.codeset = 'sidecodeset'
    assert.equal(side.fix.codeset, 'sidecodeset')
    // The name is ordinary namespaced text, in the one metadata map: a field
    // carries the name and never a copy of the members.
    assert.equal(side.get('FIX:codeset'), 'sidecodeset')
    registry.insert(side)

    // One vocabulary, read through the set the field names.
    const set = registry.codesetOf(registry.fieldByTag(54))
    assert.equal(set.name, 'sidecodeset')
    assert.deepEqual(set.codes, [
      { value: '1', name: 'Buy' },
      { value: '2', name: 'Sell', aliases: ['Sold'], doc: 'Sell side', group: 'Outright' },
    ])
    assert.deepEqual(registry.codeset('sidecodeset'), set)
    // The name is folded, so whichever spelling a caller states reaches it.
    assert.deepEqual(registry.getCodeset('SideCodeSet'), set)
    assert.equal(registry.getCodeset('absent'), null)
    assert.throws(() => registry.codeset('absent'), /expected a codesets at "absent", got nothing/)
    // A field drawing on no set answers null rather than a refusal.
    assert.equal(registry.codesetOf(fixField('Symbol', 'utf8', 55)), null)

    // Every spelling of a code reaches its wire value - the value itself, the
    // symbolic name, an alias - and a value answers its name. What the set
    // does not answer to is null: a venue sends codes no dictionary lists.
    assert.equal(registry.codeValue('sidecodeset', '2'), '2')
    assert.equal(registry.codeValue('sidecodeset', 'Buy'), '1')
    assert.equal(registry.codeValue('sidecodeset', 'sold'), '2')
    assert.equal(registry.codeValue('sidecodeset', 'Neither'), null)
    assert.equal(registry.codeName('sidecodeset', '1'), 'Buy')
    assert.equal(registry.codeName('sidecodeset', '9'), null)

    // A set a held field still reads by may not be taken away, by removal or
    // by an empty statement; the field lets go first.
    assert.throws(() => registry.removeCodeset('sidecodeset'), /sidecodeset.*side/i)
    assert.throws(() => registry.setCodeset('sidecodeset', []), /sidecodeset.*side/i)
    const held = registry.fieldByTag(54)
    held.fix.codeset = null
    assert.equal(held.fix.codeset, null)
    assert.equal(held.has('FIX:codeset'), false)
    registry.insert(held)
    assert.deepEqual(registry.removeCodeset('sidecodeset'), set.codes)
    assert.deepEqual(registry.codesetNames(), ['msgcatcodeset'])
    assert.equal(registry.removeCodeset('sidecodeset'), null)

    // The committed dictionary is the same shape at scale: one set per
    // vocabulary, and a field states only which one it reads by.
    const shipped = seed()
    assert.equal(shipped.codesetNames().length, CODESETS)
    assert.equal(shipped.fieldByTag(54).fix.codeset, 'sidecodeset')
    assert.equal(shipped.codeValue('sidecodeset', 'Buy'), '1')
    assert.equal(shipped.codeName('sidecodeset', '2'), 'Sell')
    assert.equal(shipped.codesetOf(shipped.fieldByTag(54)).name, 'sidecodeset')
    for (const tag of [447, 525]) {
      const partySource = shipped.codesetOf(shipped.fieldByTag(tag))
      assert.equal(shipped.codeValue(partySource.name, 'proprietary/customcode'), 'D')
    }
  })

  test('a field naming a code set the dictionary does not hold is refused', () => {
    const stray = fixField('Side', 'utf8', 54)
    stray.fix.codeset = 'sidecodeset'

    // Every door a field arrives through holds the same invariant, and each
    // refusal names the set that is missing rather than the field.
    const refused = /expected a codesets at "sidecodeset", got nothing/
    assert.throws(() => fix.FixRegistry.fromFields([stray]), refused)
    const registry = new fix.FixRegistry()
    assert.throws(() => registry.insert(stray), refused)
    assert.equal(registry.getFieldByTag(54), null)
    assert.deepEqual(registry.codesetNames(), ['msgcatcodeset'])

    // With the set stated the same field arrives, and a held field sent to a
    // set nothing states is refused on update, the dictionary unchanged.
    registry.setCodeset('sidecodeset', [{ value: '1', name: 'Buy' }])
    registry.insert(stray)
    registry.insert(fixField('Symbol', 'utf8', 55))
    const settled = registry.intoJson()
    const moved = registry.fieldByTag(55)
    moved.fix.codeset = 'othercodeset'
    assert.throws(() => registry.update(moved), /expected a codesets at "othercodeset", got nothing/)
    assert.equal(registry.intoJson(), settled)
    assert.equal(registry.fieldByTag(55).fix.codeset, null)

    // A snapshot is read under the same rule: the vocabularies lead it, so a
    // document whose field names one it does not carry is not a dictionary.
    const document = registry.toJSON()
    document.fields.find((field) => field.metadata['FIX:tag'] === '54').metadata['FIX:codeset'] = 'othercodeset'
    assert.throws(
      () => fix.FixRegistry.fromJson(JSON.stringify(document)),
      /expected a codesets at "othercodeset", got nothing/,
    )
  })

  test('a code set merges by wire value, keeping what the dictionary held', () => {
    const registry = new fix.FixRegistry()

    // A set the dictionary does not hold arrives whole through the fold.
    registry.mergeCodeset('sidecodeset', [
      { value: '1', name: 'Buy', doc: 'Buy side' },
      { value: '2', name: 'Sell' },
    ])
    assert.deepEqual(registry.codeset('sidecodeset').codes, [
      { value: '1', name: 'Buy', doc: 'Buy side' },
      { value: '2', name: 'Sell' },
    ])

    // A venue's statement enriches what is held rather than replacing it: a
    // wire value already held keeps its name, its wording and its order, and
    // the spelling the venue declared becomes another alias; a value nothing
    // held joins the end.
    registry.mergeCodeset('sidecodeset', [
      { value: '2', name: 'Sold' },
      { value: '7', name: 'Undisclosed' },
    ])
    assert.deepEqual(registry.codeset('sidecodeset').codes, [
      { value: '1', name: 'Buy', doc: 'Buy side' },
      { value: '2', name: 'Sell', aliases: ['Sold'] },
      { value: '7', name: 'Undisclosed' },
    ])
    assert.equal(registry.codeValue('sidecodeset', 'Sold'), '2')
    assert.equal(registry.codeValue('sidecodeset', 'Sell'), '2')

    // A statement replaces, which is the other verb: what it does not carry
    // is gone.
    registry.setCodeset('sidecodeset', [{ value: '1', name: 'Buy' }])
    assert.deepEqual(registry.codeset('sidecodeset').codes, [{ value: '1', name: 'Buy' }])
    assert.equal(registry.codeValue('sidecodeset', 'Sold'), null)

    // A field keeps the set it already reads by, and a second statement of
    // that field's vocabulary folds into the held set: the members are the
    // dictionary's to fold, so nothing moves the field to a set holding
    // strictly less than the one it reads by.
    registry.setCodeset('venuesidecodeset', [{ value: '2', name: 'Sell' }])
    const side = fixField('Side', 'utf8', 54)
    side.fix.codeset = 'sidecodeset'
    registry.insert(side)
    const venue = fixField('Side', 'utf8', 54)
    venue.fix.codeset = 'venuesidecodeset'
    registry.update(venue)
    assert.equal(registry.fieldByTag(54).fix.codeset, 'sidecodeset')
    assert.deepEqual(registry.codeset('sidecodeset').codes, [
      { value: '1', name: 'Buy' },
      { value: '2', name: 'Sell' },
    ])
    assert.deepEqual(registry.codesetNames(), ['msgcatcodeset', 'sidecodeset', 'venuesidecodeset'])
  })

  test('a tag crosses as a number and is never narrowed', () => {
    const field = Field.from('Symbol: utf8')

    for (const value of [2 ** 31, -(2 ** 31) - 1, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
      assert.throws(() => {
        field.fix.tag = value
      }, /signed 32-bit integer/)
      assert.throws(() => {
        field.fix.tags = [55, value]
      }, /signed 32-bit integer/)
    }
    // A refusal leaves the field untouched.
    assert.equal(field.fix.tag, null)
    assert.equal(field.fix.size, 0)

    // The core's own refusals arrive with the full key in the message. A tag
    // is positive: zero is what an unresolved arrival records, never an
    // identity a field can claim (`rust/tests/fix/entry.rs`).
    for (const tag of [0, -1, -(2 ** 31)]) {
      assert.throws(() => {
        field.fix.tag = tag
      }, /FIX:tag.*from 1 to 2147483647/)
      assert.throws(() => {
        field.fix.counter = tag
      }, /FIX:counter.*from 1 to 2147483647/)
      assert.throws(() => {
        field.fix.tags = [4, tag]
      }, /FIX:tags.*from 1 to 2147483647/)
    }
    assert.throws(() => {
      field.fix.tags = [55, 55]
    }, /FIX:tags/)
    assert.throws(() => {
      field.fix.names = ['Sym', 'sym']
    }, /FIX:names/)
    assert.throws(() => {
      field.fix.names = ['Sym"bol']
    }, /FIX:names/)
    assert.equal(field.fix.tag, null)
    assert.equal(field.fix.counter, null)
    assert.deepEqual(field.fix.tags, [])
    assert.equal(field.fix.size, 0)

    // The widest positive tag is one, on every positive tag writer.
    field.fix.tag = 2 ** 31 - 1
    field.fix.counter = 2 ** 31 - 1
    assert.equal(field.fix.tag, 2 ** 31 - 1)
    assert.equal(field.fix.counter, 2 ** 31 - 1)
  })

  test('an externally stated nonpositive tag is refused where it is read', () => {
    // Text written past the typed setters is read strictly: decimal digits of a
    // positive tag, never signed and never wider than an i32. A registry refuses
    // the field and is left exactly as it was. The alternates are one JSON
    // array, and their elements are held to the same shape.
    for (const [key, property] of [['FIX:tag', 'tag'], ['FIX:counter', 'counter'], ['FIX:tags', 'tags']]) {
      for (const digits of ['0', '000', '-1', '+1', '2147483648']) {
        const text = key === 'FIX:tags' ? `[${digits}]` : digits
        const field = fixField('incoming', 'utf8', 90_001)
        field.set(key, text)
        assert.throws(() => field.fix[property], new RegExp(key), `${key}=${text}`)
        const registry = new fix.FixRegistry()
        const before = registry.clone()
        assert.throws(() => registry.insert(field), `${key}=${text}`)
        assert.ok(registry.equals(before), `${key}=${text}`)
      }
    }
    // Leading zeros are still the tag on the two bare decimals, and never in
    // the array: a JSON number spells none, and the array is JSON.
    const field = fixField('positive', 'utf8', 1)
    for (const key of ['FIX:tag', 'FIX:counter']) field.set(key, '0001')
    field.set('FIX:tags', '[1]')
    assert.equal(field.fix.tag, 1)
    assert.equal(field.fix.counter, 1)
    assert.deepEqual(field.fix.tags, [1])
    field.set('FIX:tags', '[0001]')
    assert.throws(() => field.fix.tags, /FIX:tags/)
  })

  test('the identifier is a number derived from the tag and the name', () => {
    const trade = Field.from('TradeID: utf8')
    // There is no identity without a tag.
    assert.equal(trade.fix.id, null)

    trade.fix.tag = 5001
    const id = trade.fix.id
    assert.equal(typeof id, 'number')
    assert.ok(Number.isInteger(id))
    assert.ok(id >= -(2 ** 31) && id < 2 ** 31, 'a signed 32-bit digest')
    // Derived on every read, never stored: the view holds the tag alone.
    assert.equal(trade.fix.size, 1)
    assert.equal(trade.has('FIX:id'), false)
    // What the field answers is what the registry answers by it.
    assert.equal(fix.FixRegistry.fromFields([trade]).fieldById(id).name, 'TradeID')

    // The name folds once - ASCII case, `_`, `-` and space dropped - so three
    // spellings of one field under one tag are one identity; another tag or
    // another name is another.
    const ids = ['MsgType', 'msgtype', 'Msg_Type', 'msg-type', 'Msg Type'].map((name) => {
      const field = Field.from(`${name}: utf8`)
      field.fix.tag = 35
      return field.fix.id
    })
    assert.equal(new Set(ids).size, 1)
    const other = Field.from('MsgSeqNum: utf8')
    other.fix.tag = 35
    assert.notEqual(other.fix.id, ids[0])
    const moved = Field.from('MsgType: utf8')
    moved.fix.tag = 36
    assert.notEqual(moved.fix.id, ids[0])
    // A renamed field is another identity, from the same tag.
    trade.fix.tag = 35
    assert.notEqual(trade.fix.id, ids[0])

    // The identity is read, never assigned: the tag and the name are what it
    // is made of, and a property with no setter refuses an assignment.
    assert.throws(() => {
      trade.fix.id = 42
    }, TypeError)
    assert.equal(trade.fix.tag, 35)

    // Any positive tag holds an identity: nothing gates a tag on its
    // dictionary any more. Zero holds none, because no field can claim it: it
    // marks an arrival the dictionary did not resolve.
    for (const tag of [1, 35, 4999, 5000, 39_999, 40_000, 65_000, 2 ** 31 - 1]) {
      const field = Field.from('Any: utf8')
      field.fix.tag = tag
      assert.ok(Number.isInteger(field.fix.id), `${tag}`)
    }
    const zero = Field.from('Any: utf8')
    assert.throws(() => {
      zero.fix.tag = 0
    }, /FIX:tag/)
    assert.equal(zero.fix.id, null)
  })

  test('membership is a sorted list of dictionary names on the field', () => {
    const trade = Field.from('TradeID: utf8')
    // An absent property is an empty list, and no field the specification
    // alone defines states one.
    assert.deepEqual(trade.fix.branches, [])
    assert.equal(trade.has('FIX:branches'), false)
    assert.equal(trade.fix.hasBranch('cme'), false)

    // Assigning replaces the list: folded once, deduplicated under the fold,
    // sorted, and stored comma-joined under `FIX:branches`.
    trade.fix.branches = ['CME', 'Bloomberg', 'cme']
    assert.deepEqual(trade.fix.branches, ['bloomberg', 'cme'])
    assert.equal(trade.get('FIX:branches'), 'bloomberg,cme')
    assert.equal(trade.fix.hasBranch('CME'), true)
    assert.equal(trade.fix.hasBranch('bloomberg'), true)
    assert.equal(trade.fix.hasBranch('ice'), false)

    // `addBranch` is idempotent under the fold.
    trade.fix.addBranch('ICE')
    trade.fix.addBranch('cme')
    assert.deepEqual(trade.fix.branches, ['bloomberg', 'cme', 'ice'])

    // Membership is provenance: it never touches the identity.
    trade.fix.tag = 5001
    const id = trade.fix.id
    trade.fix.branches = ['venue']
    assert.equal(trade.fix.id, id)

    // An empty array removes the property, as every list property is removed.
    trade.fix.branches = []
    assert.deepEqual(trade.fix.branches, [])
    assert.equal(trade.has('FIX:branches'), false)

    // A name that is empty or carries the separator is the core's refusal,
    // naming the key, and nothing is written by it.
    trade.fix.branches = ['cme']
    for (const bad of [[''], ['c,me'], ['ice', '']]) {
      assert.throws(() => {
        trade.fix.branches = bad
      }, /FIX:branches/)
    }
    assert.throws(() => trade.fix.addBranch(''), /FIX:branches/)
    assert.throws(() => trade.fix.addBranch('a,b'), /FIX:branches/)
    assert.deepEqual(trade.fix.branches, ['cme'])
    // The list is strings, never a bare string or a number.
    assert.throws(() => {
      trade.fix.branches = 'cme'
    })
    assert.throws(() => {
      trade.fix.branches = [5001]
    }, /into rust type `String`/)
    assert.throws(() => trade.fix.addBranch(5001), /into rust type `String`/)
    assert.deepEqual(trade.fix.branches, ['cme'])
  })

  test('the registry resolves every key the way the core does', () => {
    const registry = seed()
    // The store's fields, and the crate's own beside them: a loaded dictionary
    // holds the crate's definition over any document it finds, and `size`
    // counts the components and the groups the walk answers behind them.
    assert.equal(scalars(registry).length, STORED + CRATE_SCALARS.length)
    assert.equal(registry.size, [...registry].length)

    assert.equal(registry.fieldByTag(55).name, 'symbol')
    assert.equal(registry.getFieldByTag(55).name, 'symbol')
    const symbol = registry.fieldByTag(55).fix.id
    assert.equal(registry.fieldById(symbol).name, 'symbol')
    assert.ok(registry.getFieldById(symbol).equals(registry.fieldByTag(55)))
    // The alternate tag 20 reaches ExecType, which claims 150 canonically.
    assert.equal(registry.fieldByTag(150).name, 'exectype')
    // `OrdStatus` and `ExecType` keep the text the wire spells: the ranked
    // The two the ranked state is read off are the dictionary's own text.
    assert.ok(registry.fieldByTag(39).dtype.equals(DataType.from('utf8')))
    assert.ok(registry.fieldByTag(150).dtype.equals(DataType.from('utf8')))
    // A name answers the canonical spelling whatever case it was asked in.
    assert.equal(registry.fieldByName('symbol').name, 'symbol')
    assert.equal(registry.fieldByName('SYMBOL').name, 'symbol')
    assert.equal(registry.fieldByName('clordid').name, 'clordid')
    // A path reaches a repeating group and one of its members.
    assert.equal(registry.fieldByPath('NoPartyIDs').fix.tag, 453)
    // An occurrence is not a path segment: the walk steps through the list and
    // the member is spelled directly under the counter.
    assert.equal(registry.fieldByPath('parties.partyid').fix.tag, 448)
    assert.equal(registry.fieldByPath('parties.partyrole').name, 'partyrole')
    assert.equal(registry.getFieldByPath('parties.partyid.partyid'), null)

    // The generic pair answers exactly what the specialized one does.
    for (const key of [55, 'Symbol', 'nopartyids', 'parties.partyid']) {
      const answer = registry.field(key)
      assert.ok(answer.equals(registry.getField(key)))
      assert.ok(answer.equals(registry.get(key)))
      assert.equal(registry.has(key), true)
    }
    assert.equal(registry.has(9999), false)
    assert.equal(registry.has('Nope'), false)
    assert.equal(registry.getField(9999), null)
    assert.equal(registry.get('Nope'), null)
    assert.equal(registry.getFieldByPath('Symbol.absent'), null)
    assert.equal(registry.has('55'), false, 'a tag query never consults names')
    // Membership is provenance and the seed states none.
    assert.deepEqual(registry.dialects(), [])
  })

  test('protocol and MsgType inference stays native and shallow', () => {
    const registry = seed()
    const cases = [
      ['prefix 8=FIX.4.4|35=D|55=AAPL|10=001| Symbol=suffix', MimeType.FIX, 'D'],
      ['ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL', MimeType.ULLINK, '8'],
      ['8=FIX.4.4|35=UL|#SYMBOL=TTF|10=001|', MimeType.FIXUL, 'UL'],
      [
        '8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|',
        MimeType.FIXUL,
        'D',
      ],
      ['level=INFO message=random', MimeType.KEYVALUE, null],
      [
        // A JSON document is JSON, which is what it is, and it declares no
        // type: the codec reads none and names such a row `unknown`.
        '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}',
        MimeType.JSON,
        null,
      ],
    ]
    for (const [line, protocol, msgtype] of cases) {
      assert.ok(MimeType.inferBytes(Buffer.from(line)).equals(protocol))
      assert.ok(MimeType.inferText(line).equals(protocol))
      const bytes = fix.FixCodec.inferMsgtypeBytes(Buffer.from(line))
      assert.equal(bytes?.toString() ?? null, msgtype)
      assert.equal(fix.FixCodec.inferMsgtypeText(line), msgtype)
    }

    assert.equal(fix.FixCodec.inferMsgtypeBytes(Buffer.from('35=AE|')).toString(), 'AE')
    assert.equal(fix.FixCodec.inferMsgtypeText('MSGTYPE=AE|'), 'AE')

    // A JSON document states no half of the exchange on its own, and a
    // `send` its own payload spells is never read as the marker: which way it
    // moved is the prose in front of it, read into FIX's own tag 385 by the
    // rules the dictionary carries on that field. The document itself is a
    // body the codec does not read: the row is one message named `unknown`,
    // stating no type and no entries, whatever the document says inside.
    const answered =
      '{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},' +
      '"value":{"Name":"Router_TradeCapture","Version":"4.7.0"},"status":200}'
    const asked = '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}'
    assert.ok(MimeType.inferText(answered).equals(MimeType.JSON))
    assert.equal(fix.FixCodec.inferMsgtypeText(answered), null)
    const codec = reading(new fix.FixRegistry())
    const read = (line) => {
      const messages = codec.parseLine(Buffer.from(line))
      const message = messages.next().value
      assert.equal(messages.next().done, true, line)
      assert.equal(message.field.name, 'unknown', line)
      assert.equal(message.header().msgtype, '', line)
      assert.equal(message.getByName('Name'), null, line)
      assert.deepEqual(message.entries(), [], line)
      return message
    }
    // A direction is the line's, read apart from the document: prose in front
    // of it spells one, and a duration behind it is the line's too.
    for (const [body, verb, code] of [[answered, 'Response', 'R'], [asked, 'Request', 'S']]) {
      assert.equal(read(body).header().msgdirection, null)
      assert.equal(read(`${verb}: ${body}`).header().msgdirection, code)
      assert.equal(read(`[Jolokia] (DEBUG) ${verb}: ${body} (12 ms)`).byTag(385).asJs(), code)
    }
  })

  test('one namespace: a reused name merges and a reused tag stands beside its holder', () => {
    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
      fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
    ])
    assert.deepEqual(registry.dialects(), ['cme'])

    // A venue reusing a name under another tag is the same field spelled with
    // another number: `addField` folds it into the holder, which gains the tag
    // as an alternate, the alias, and the membership; `insert` refuses it.
    const venueSymbol = fixField('Symbol', 'utf8', 5055, { branches: ['cme'], names: ['VenueTicker'] })
    assert.throws(() => registry.insert(venueSymbol), /held by Symbol/)
    assert.equal(registry.addField(venueSymbol), false)
    assert.equal(registry.size, 2 + SEEDED)
    const symbol = registry.fieldByTag(55)
    assert.equal(registry.fieldByTag(5055).name, 'Symbol')
    assert.deepEqual(symbol.fix.tags, [5055])
    assert.deepEqual(symbol.fix.names, ['Ticker', 'VenueTicker'])
    assert.deepEqual(symbol.fix.branches, ['cme'])
    assert.equal(registry.fieldByName('venueticker').fix.id, symbol.fix.id)
    assert.equal(registry.getFieldByPath('Symbol').fix.id, symbol.fix.id)

    // A venue reusing a tag under another name is a new thing it defined over
    // that tag: it is registered under its own identity beside the holder,
    // the holder gains the name as an alias, and the bare tag keeps answering
    // the holder while the newcomer is reached by its name or its identity.
    const venueId = fixField('VenueSymbol', 'utf8', 55, { branches: ['cme'] })
    assert.equal(registry.insert(venueId), null)
    assert.equal(registry.size, 3 + SEEDED)
    assert.equal(registry.fieldByTag(55).name, 'Symbol')
    assert.deepEqual(registry.fieldByTag(55).fix.names, ['Ticker', 'VenueTicker', 'VenueSymbol'])
    const newcomer = registry.fieldByName('venuesymbol')
    assert.equal(newcomer.name, 'VenueSymbol')
    assert.notEqual(newcomer.fix.id, symbol.fix.id)
    assert.equal(newcomer.fix.id, venueId.fix.id)
    assert.equal(registry.fieldById(newcomer.fix.id).name, 'VenueSymbol')
    assert.equal(registry.fieldById(symbol.fix.id).name, 'Symbol')
    assert.equal(registry.has(55), true)
    assert.deepEqual(registry.dialects(), ['cme'])

    // A bare string is a name, never an identifier, and a bare number is a
    // tag: an identifier is only ever spelled through the `ById` doors.
    assert.equal(registry.getField('symbol').fix.id, symbol.fix.id)
    assert.equal(registry.getField(`${symbol.fix.id}`), null)
    assert.equal(registry.has(`${symbol.fix.id}`), false)
    assert.equal(registry.get(55).name, 'Symbol')
    assert.equal(registry.remove(`${symbol.fix.id}`), null)
    assert.equal(registry.size, 3 + SEEDED)
  })

  test('removeById reaches one of two fields on a tag by its own identity', () => {
    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
      fixField('TradeID', 'utf8', 5001, { branches: ['cme'], names: ['VenueTrade'] }),
    ])
    registry.insert(fixField('VenueSymbol', 'utf8', 55, { branches: ['cme'] }))
    assert.equal(registry.size, 3 + SEEDED)
    const symbol = registry.fieldByTag(55).fix.id
    const venue = registry.fieldByName('VenueSymbol').fix.id

    // The generic `remove` reads a number as the tag, which the holder answers;
    // the identity is what reaches the newcomer.
    const removed = registry.removeById(venue)
    assert.equal(removed.name, 'VenueSymbol')
    assert.equal(registry.size, 2 + SEEDED)
    assert.equal(registry.getFieldById(venue), null)
    // The alias the holder gained when the newcomer arrived is the holder's
    // to keep: the name still answers, now to the holder alone.
    assert.equal(registry.getFieldByName('venuesymbol').name, 'Symbol')
    assert.equal(registry.fieldByTag(55).name, 'Symbol')
    // A field that is not there answers null rather than throwing.
    assert.equal(registry.removeById(venue), null)
    // And the holder is reached by identifier just as well.
    assert.equal(registry.removeById(symbol).name, 'Symbol')
    assert.equal(registry.getFieldByName('ticker'), null)
    assert.equal(registry.size, 1 + SEEDED)
    assert.equal(registry.removeById(registry.fieldByTag(5001).fix.id).name, 'TradeID')
    assert.equal(registry.size, SEEDED)

    // An identifier is an exact number, never text.
    assert.throws(() => registry.removeById(1.5), /id must be a signed 32-bit integer/)
    assert.throws(() => registry.removeById('55'), /into rust type `f64`/)
    assert.equal(registry.size, SEEDED)
  })

  test('absence throws with the core message, its get twin answers null', () => {
    const registry = seed()

    assert.throws(
      () => registry.fieldByTag(9999),
      /^Error: expected a fix field at "tag 9999", got nothing$/,
    )
    // An identifier no field stands behind: one derived for a tag the seed
    // does not hold.
    const stray = fixField('TradeID', 'utf8', 5001)
    assert.throws(
      () => registry.fieldById(stray.fix.id),
      new RegExp(`^Error: expected a fix field at "identifier ${stray.fix.id}", got nothing$`),
    )
    assert.throws(() => registry.fieldByName('Nope'), /name \\"Nope\\"/)
    assert.throws(() => registry.fieldByPath('Symbol.absent'), /path Symbol\.absent/)
    assert.throws(() => registry.field(9999), /tag 9999/)
    assert.equal(registry.getFieldByName('Nope'), null)
    assert.equal(registry.getFieldById(stray.fix.id), null)
  })

  test('a key is a number tag or a string name, and nothing else', () => {
    const registry = seed()

    for (const key of [3.5, 2 ** 31, -(2 ** 31) - 1, Number.NaN]) {
      assert.throws(() => registry.get(key), /key must be a signed 32-bit integer/)
    }
    assert.throws(() => registry.getFieldByTag(2 ** 31), /tag must be a signed 32-bit integer/)
    assert.throws(() => registry.fieldByTag(1.5), /tag must be a signed 32-bit integer/)

    for (const [key, named] of [
      [55n, 'BigInt'],
      [{ tag: 55 }, 'Object'],
      [null, 'Null'],
      [true, 'Boolean'],
      [undefined, 'Undefined'],
    ]) {
      assert.throws(() => registry.get(key), {
        name: 'TypeError',
        message: `key must be a number tag or a string name, got ${named}`,
      })
    }
    // The specialized halves take exactly one shape, checked by Node-API.
    assert.throws(() => registry.fieldByName(55), /into rust type `String`/)
    assert.throws(() => registry.fieldByTag('55'), /into rust type `f64`/)
  })

  test('every identifier argument is an exact number at the boundary', () => {
    const registry = seed()

    // A fractional or out-of-range number is refused rather than narrowed into
    // another identity, and text is not a number at all.
    for (const bad of [1.5, 2 ** 31, -(2 ** 31) - 1, Number.NaN, Number.POSITIVE_INFINITY]) {
      assert.throws(() => registry.fieldById(bad), /id must be a signed 32-bit integer/)
      assert.throws(() => registry.getFieldById(bad), /id must be a signed 32-bit integer/)
      assert.throws(() => registry.removeById(bad), /id must be a signed 32-bit integer/)
    }
    for (const wrong of ['55', null, 55n, { id: 55 }]) {
      assert.throws(() => registry.fieldById(wrong), /into rust type `f64`/)
      assert.throws(() => registry.getFieldById(wrong), /into rust type `f64`/)
    }
    for (const wrong of [55, 3.5]) {
      assert.throws(() => registry.getFieldByName(wrong), /into rust type `String`/)
      assert.throws(() => registry.fieldByPath(wrong), /none of these types `String`, `JsFieldPath`/)
    }
  })

  test('the registry iterates lazily in ascending identifier order', () => {
    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 55),
      fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
      fixField('Price', 'decimal128(20, 8)', 44),
      fixField('VenueQty', 'int64', 5002, { branches: ['cme'] }),
      fixField('Account', 'utf8', 1),
      fixField('Tail', 'utf8', 9001),
    ])
    // Two fields on one tag: the second stands beside the holder.
    registry.insert(fixField('VenueSymbol', 'utf8', 55))

    // Tag-major, the tag's holder first, then by identifier - the core's own
    // order. The venue fields therefore precede the later standard tag, the
    // seeded SendingTime (52) and TransactTime (60) clocks are ordinary fields
    // in their tag places, and the crate's own scalar fields close the
    // scalars: their tags sit above any a test claims. The definitions walk
    // behind them, in name order - here the crate's two Map groups, filed as
    // groups by the shape they have.
    assert.deepEqual(
      scalars(registry).map((field) => field.fix.tag),
      [1, 44, 52, 55, 55, 60, 5001, 5002, 9001, ...CRATE_TAGS],
    )
    // The crate's own definitions close the walk.
    assert.deepEqual(
      definitionsOf(registry).map((field) => field.name).sort(),
      CRATE_GROUPS.map((field) => field.name).sort(),
    )
    assert.equal([...registry].length, registry.size)
    assert.deepEqual(
      [...registry].filter((field) => [52, 60].includes(field.fix.tag)).map((field) => field.name),
      ['sendingtime', 'transacttime'],
    )
    const pair = [...registry].filter((field) => field.fix.tag === 55).map((field) => field.name)
    assert.deepEqual(pair, ['Symbol', 'VenueSymbol'])
    assert.deepEqual(
      [...registry.keys()].map((field) => field.fix.id),
      [...registry].map((field) => field.fix.id),
    )
    assert.ok([...registry].every((field) => Number.isInteger(field.fix.id)))

    // An unfinished walk shares the registry, so a mutation refuses until the
    // walk ends - by exhaustion or by the `return` a `break` sends.
    const walk = registry.keys()
    assert.equal(walk.next().value.name, 'Account')
    assert.equal(walk.next().value.name, 'Price')
    assert.throws(() => registry.remove(1), /shared with a message/)
    assert.throws(() => registry.removeById(registry.fieldByTag(5001).fix.id), /shared with a message/)
    walk.return()
    assert.equal(registry.remove(1).name, 'Account')
    assert.deepEqual(
      scalars(registry).map((field) => field.fix.tag),
      [44, 52, 55, 55, 60, 5001, 5002, 9001, ...CRATE_TAGS],
    )
    // A walk that stops among the definitions releases the registry as
    // promptly as one that stops among the scalars.
    const late = registry.keys()
    for (let at = 0; at < 8 + CRATE_TAGS.length + 1; at += 1) late.next()
    assert.throws(() => registry.remove(44), /shared with a message/)
    late.return()
    assert.equal(registry.remove(44).name, 'Price')
  })

  test('the seed iterates in canonical-tag order and every field is standard', () => {
    const registry = seed()

    const names = [...registry].map((field) => field.name)
    assert.deepEqual(names.slice(0, 4), ['account', 'advid', 'advrefid', 'advside'])
    assert.equal(names.length, registry.size)

    const tags = scalars(registry).map((field) => field.fix.tag)
    assert.deepEqual(tags, [...tags].sort((left, right) => left - right))
    // Every stored field is a specification field, and the crate's own are
    // standard fields above every published tag, so no field states a
    // membership at all - and the crate's close the scalars, since nothing
    // the seed stores sits in their block from 65000. The store's own
    // SendingTime and TransactTime are what 52 and 60 answer: a loaded
    // definition supplies its metadata rather than colliding with a seed.
    assert.ok([...registry].every((field) => field.fix.branches.length === 0))
    assert.ok([...registry].every((field) => !field.has('FIX:branches')))
    assert.deepEqual(registry.dialects(), [])
    assert.deepEqual(tags.filter((tag) => tag >= 65000), CRATE_TAGS)
    assert.equal(tags.length, STORED + CRATE_SCALARS.length)
    // The definitions walk behind the scalars: the components and the groups
    // the dictionary declares, the crate's Map group among them, each a
    // nested datatype and none of them a scalar the tag doors answer.
    const definitions = definitionsOf(registry)
    assert.ok(definitions.every((field) => field.dtype.kind === 'nested'))
    assert.equal(definitions.length + tags.length, registry.size)
    assert.ok(definitions.some((field) => field.name === 'parties'))
    assert.ok(definitions.some((field) => field.name === 'metadata'))
  })

  test('the registry takes every storage location', () => {
    const reference = seed()
    const url = Url.fromPath(SEED)

    for (const location of [SEED, path.resolve(SEED), url.toString(), url, new IOBase(SEED)]) {
      assert.ok(fix.FixRegistry.fromHandle(location).equals(reference))
    }

    // A folder that is not there loads as a new registry - the crate's own
    // fields and the two seeded clocks, nothing else - and is not created.
    const root = scratch()
    try {
      const missing = path.join(root, 'missing')
      assert.ok(fix.FixRegistry.fromHandle(missing).equals(new fix.FixRegistry()))
      assert.equal(fix.FixRegistry.fromHandle(missing).size, SEEDED)
      assert.equal(fs.existsSync(missing), false)
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
  })

  test('a malformed native shard names its location', (t) => {
    const root = scratch()
    t.after(() => fs.rmSync(root, { recursive: true, force: true }))
    fs.mkdirSync(path.join(root, 'fields'))
    fs.writeFileSync(path.join(root, 'fields', '0.json'), '[{"name":"broken"}]')
    assert.throws(() => fix.FixRegistry.fromHandle(root), /0.json/)
  })

  test('a written catalog reloads its three categories and the sets they read by', (t) => {
    const root = scratch()
    t.after(() => fs.rmSync(root, { recursive: true, force: true }))
    const dictionary = path.join(root, 'dictionary')
    const reference = seed()
    reference.writeInto(dictionary)
    // Four folders: the three categories, and the vocabularies the fields name.
    assert.deepEqual(fs.readdirSync(dictionary).sort(), ['codesets', 'components', 'fields', 'groups'])
    // One document per set, filed under the name a field states.
    assert.deepEqual(
      fs.readdirSync(path.join(dictionary, 'codesets')).sort(),
      reference.codesetNames().map((name) => `${name}.json`).sort(),
    )
    const shards = fs.readdirSync(path.join(dictionary, 'fields'))
    // A shard is named by its tag block, nine digits with leading zeros. The
    // crate's own fields are written too - a store states the whole row - and
    // their block from 65000 is one shard of its own, the fixed row is the
    // `fixmsg` component and the crate's metadata Map is a group; the reload
    // takes the definition every registry holds over the document it finds.
    assert.ok(shards.every((shard) => /^\d{9}\.json$/.test(shard)), shards.join(', '))
    assert.equal(shards.includes('000000000.json'), true)
    assert.equal(shards.includes('000000650.json'), true)
    assert.equal(fs.existsSync(path.join(dictionary, 'components', 'fixmsg.json')), true)
    assert.equal(fs.existsSync(path.join(dictionary, 'groups', 'metadata.json')), true)
    assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))
    const reloaded = fix.FixRegistry.fromHandle(new IOBase(dictionary))
    for (const key of [453, 'partyid', 447, 452]) assert.equal(reloaded.remove(key), null)
    reloaded.insert(fixField('Unreferenced', 'utf8', 39999))
    assert.equal(reloaded.remove(39999).name, 'Unreferenced')
    reloaded.writeInto(new IOBase(dictionary))
    assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))
  })

  test('membership is stored on the field, in the one shard tree', (t) => {
    const root = scratch()
    t.after(() => fs.rmSync(root, { recursive: true, force: true }))
    const dictionary = path.join(root, 'dictionary')

    const registry = fix.FixRegistry.fromFields([
      fixField('MsgType', 'utf8', 35),
      fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
    ])
    registry.writeInto(dictionary)

    // One shard arithmetic for every field: 5001 / 100 is 50, whatever the
    // field's membership, and nothing is keyed by a dictionary name.
    assert.deepEqual(fs.readdirSync(path.join(dictionary, 'fields')).sort(), [
      '000000000.json',
      '000000050.json',
      '000000650.json',
    ])
    assert.equal(fs.existsSync(path.join(dictionary, 'branches.json')), false)
    assert.equal(
      fs.existsSync(path.join(dictionary, 'fields', '000000650.json')),
      true,
      "the crate's own fields are stored like every other",
    )

    const reloaded = fix.FixRegistry.fromHandle(dictionary)
    assert.ok(reloaded.equals(registry))
    assert.equal(reloaded.stableHash(), registry.stableHash())
    assert.equal(reloaded.fieldById(registry.fieldByTag(5001).fix.id).name, 'TradeID')
    assert.equal(reloaded.fieldByName('tradeid').name, 'TradeID')
    assert.deepEqual(reloaded.fieldByTag(5001).fix.branches, ['cme'])
    assert.deepEqual(reloaded.fieldByTag(35).fix.branches, [])
    assert.deepEqual(reloaded.dialects(), ['cme'])
    // Membership is metadata like any other: it is in the snapshot's fields
    // and nowhere else. The vocabularies are the fourth key, beside the three
    // categories, because a code set is the dictionary's and not a field's.
    const document = registry.toJSON()
    assert.deepEqual(Object.keys(document).sort(), ['codesets', 'components', 'fields', 'groups'])
    assert.deepEqual(document.codesets.map((codeset) => codeset.name), ['msgcatcodeset'])
    assert.equal(document.fields.find((field) => field.name === 'TradeID').metadata['FIX:branches'], 'cme')
  })

  test('insert, update and remove carry the core rules across', () => {
    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
      fixField('Price', 'decimal128(20, 8)', 44, { names: ['Px'] }),
    ])
    assert.equal(registry.size, 2 + SEEDED)
    assert.equal(registry.insert(fixField('Side', 'utf8', 54)), null)
    assert.equal(registry.fieldByTag(54).name, 'Side')

    // A key another field holds is refused, naming both; nothing changes.
    assert.throws(
      () => registry.insert(fixField('SymbolSfx', 'utf8', 65, { names: ['ticker'] })),
      /alias \\"ticker\\" of SymbolSfx, held by Symbol/,
    )
    assert.equal(registry.size, 3 + SEEDED)

    // One namespace: the same alias under a venue's membership is the same
    // conflict.
    assert.throws(
      () => registry.insert(fixField('VenueSym', 'utf8', 5055, { branches: ['cme'], names: ['ticker'] })),
      /held by Symbol/,
    )
    assert.equal(registry.size, 3 + SEEDED)
    assert.equal(registry.fieldByName('TICKER').name, 'Symbol')

    // A merge concatenates the two list properties, incoming first.
    registry.update(fixField('SYMBOL', 'utf8', 55, { tags: [65], names: ['Sym'] }))
    const merged = registry.fieldByTag(65)
    assert.equal(merged.name, 'Symbol')
    assert.deepEqual(merged.fix.names, ['Sym', 'Ticker'])
    // A datatype disagreement is refused, never widened.
    assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 55)))
    assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

    assert.equal(registry.remove('sym').name, 'Symbol')
    assert.equal(registry.getFieldByTag(65), null)
    assert.equal(registry.remove(9999), null)

    // A field with no tag cannot enter at all.
    assert.throws(() => registry.insert(Field.from('Untagged: utf8')), /FIX:tag/)
  })

  test('addField answers whether the field arrived or folded into a stored one', () => {
    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 55, { tags: [65], names: ['Ticker'], description: 'stored' }),
      fixField('Price', 'float64', 44),
    ])

    // A name that folds to a stored name merges into that field: the stored
    // identity, spelling and nullability stand, the alternate tags and the
    // aliases are the union - stored order first, the incoming canonical tag
    // last - and the incoming metadata wins a shared key.
    const incoming = fixField('symbol', 'utf8', 9001, {
      tags: [66],
      names: ['Sym', 'TICKER'],
      description: 'incoming',
    })
    assert.equal(registry.addField(incoming), false)
    assert.equal(registry.size, 2 + SEEDED)
    const stored = registry.fieldByTag(55)
    assert.equal(stored.name, 'Symbol')
    assert.equal(stored.fix.tag, 55)
    assert.deepEqual(stored.fix.tags, [65, 66, 9001])
    assert.deepEqual(stored.fix.names, ['Ticker', 'Sym'])
    assert.equal(stored.fix.description, 'incoming')

    // Every spelling the incoming field carried now reaches the stored one.
    for (const key of [9001, 66, 65, 'sym', 'TICKER']) {
      assert.equal(registry.field(key).name, 'Symbol', `${key}`)
    }

    // Folding it again changes nothing, and a field nothing answers to arrives
    // whole. `TransactTime` is no longer such a field: every registry seeds it
    // at 60 as a nanosecond UTC clock, so a text spelling of it folds into the
    // seed and its datatype is refused like any other disagreement.
    const folded = registry.intoJson()
    assert.equal(registry.addField(incoming), false)
    assert.equal(registry.intoJson(), folded)
    assert.equal(registry.addField(fixField('Text', 'utf8', 58)), true)
    assert.equal(registry.size, 3 + SEEDED)
    const arrived = registry.intoJson()
    assert.throws(() => registry.addField(fixField('TransactTime', 'utf8', 60)))
    assert.equal(registry.intoJson(), arrived)
    assert.equal(registry.fieldByTag(60).name, 'transacttime')

    // A datatype that disagrees with the stored field is refused, and the
    // refusal writes nothing: merging metadata never redeclares a datatype.
    const settled = registry.intoJson()
    assert.throws(() => registry.addField(fixField('SYMBOL', 'int32', 9002)), /utf8/)
    assert.equal(registry.intoJson(), settled)
    assert.equal(registry.getFieldByTag(9002), null)

    // One of this crate's own tags is every dictionary's already: neither added
    // nor merged.
    assert.equal(registry.addField(fix.crateFields()[0]), false)
    assert.equal(registry.intoJson(), settled)
  })

  test('a shared registry refuses mutation and a clone is independent', () => {
    const registry = seed()
    const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })
    const message = new fix.FixMsg(root, { symbol: 'AAPL' }, registry)

    for (const mutation of [
      () => registry.insert(fixField('side', 'utf8', 54)),
      () => registry.update(fixField('symbol', 'utf8', 55)),
      () => registry.remove(55),
      () => registry.removeById(registry.fieldByTag(55).fix.id),
    ]) {
      assert.throws(mutation, /shared with a message or installed as the process default/)
    }
    assert.ok(message.registry.equals(registry))
    // The shared dictionary stays readable, and a deep copy is writable.
    assert.equal(registry.fieldByTag(55).name, 'symbol')
    const copy = registry.clone()
    assert.ok(copy.equals(registry))
    copy.insert(fixField('Unreferenced', 'utf8', 39999))
    assert.equal(copy.equals(registry), false)
    assert.equal(copy.size, registry.size + 1)
  })

  // A hand-built order states its own SendingTime, as every hand-built message
  // in the Rust suites does, so two builds of it settle the same clocks. It
  // declares typed facts - the type, the side, the clock - among its content,
  // which is where a plain row states them.
  function order(registry) {
    return fields.struct(
      'NewOrderSingle',
      [
        registry.fieldByTag(35),
        registry.fieldByTag(11),
        registry.fieldByTag(55),
        registry.fieldByTag(38),
        registry.fieldByTag(54),
        registry.fieldByName('nopartyids'),
        registry.fieldByName('Parties'),
        Field.from('9999: utf8'),
        registry.fieldByTag(52),
      ],
      { nullable: false },
    )
  }

  const ORDER_VALUE = {
    msgtype: 'D',
    clordid: 'C-1',
    symbol: 'AAPL',
    orderqty: Scalar.decimal(100n * 10n ** 18n, 18),
    side: '1',
    nopartyids: 1,
    parties: [{ partyid: 'BROKER', partyidsource: 'D', partyrole: 1 }],
    9999: 'custom',
    sendingtime: SENDING,
  }

  // The content row a built order keeps: every child that is not a typed fact,
  // in the root's order. `ClOrdID(11)` and `OrderQty(38)` are facts the message
  // lifts and holds; the side is an ordinary child and stays.
  const CONTENT = ['symbol', 'side', 'nopartyids', 'parties', '9999']

  // The nanosecond instant `SENDING` is, as the typed facts state it.
  const SENDING_NS = 1_704_190_530_000_000_000n

  /** The entries of a message flattened pre-order into `[tag, name, value]`. */
  function flat(message) {
    const held = []
    const walk = (entries) => {
      for (const entry of entries) {
        held.push([entry.tag, entry.name, entry.value])
        walk(entry.entries)
      }
    }
    walk(message.entries())
    return held
  }

  test('a message holds its typed facts beside the content row it resolves through the registry', () => {
    const registry = seed()
    const root = order(registry)
    const message = new fix.FixMsg(root, ORDER_VALUE, registry)

    // A child stating a typed fact - the type, the identifier, the quantity,
    // the sending clock - fills the holder that owns it and leaves the row, so
    // the row is what the root declared beyond them, in its order.
    assert.ok(message.registry.equals(registry))
    assert.equal(message.field.equals(root), false)
    assert.deepEqual([...message.field.dtype.keys()], CONTENT)
    assert.equal(message.size, CONTENT.length)
    assert.equal(message.value.kind, 'serie')
    assert.equal(message.value.length, CONTENT.length)
    // The header states what the root stated: no version, the type, the
    // clock, and none of the trailer the frame never carried.
    assert.deepEqual(message.header(), {
      beginstring: '',
      msgtype: 'D',
      sendercompid: null,
      targetcompid: null,
      msgseqnum: null,
      sendingtime: SENDING_NS,
      possdupflag: null,
      msgdirection: null,
      signaturelength: null,
      signature: null,
      checksum: null,
    })
    // The side is the event's; the event happened at the sending clock, which
    // is where its chain was created too; the cross code is the client's
    // order identifier, the first stated of the tags a chain is named by.
    assert.equal(message.side, 'BUY')
    assert.equal(message.crosscode, 'C-1')
    assert.equal(message.currunix, SENDING_NS)
    assert.equal(message.event().creaunix, SENDING_NS)
    assert.equal(message.state, '00UNKNOWN')
    assert.equal(message.seqnum, 0)
    assert.equal(message.prevuuid, null)
    assert.deepEqual(message.srcuuids, [])
    // No `Price(44)` is no price stated: null, never a zero standing in.
    assert.equal(message.price, null)
    // `OrderQty(38)` is the quantity the event is about, so the root's child
    // filled it rather than staying a column.
    assert.equal(message.quantity, '100')
    assert.equal('px' in message, false)
    assert.equal('qty' in message, false)
    assert.equal(message.currency, 'XXX')
    assert.equal(message.text, null)
    assert.deepEqual(message.metadata, {})
    // Nothing was captured: no plugin, no context, no session, so no session
    // event either. Where the line came from is the reader's statement, on
    // the row, and never here.
    assert.deepEqual(message.capture(), {
      msgpluginid: null,
      msgctxid: null,
      msgsessionid: null,
      msgsesseventid: null,
    })

    // A lookup reaches the row and the holders alike, always as a Scalar: a
    // typed tag answers its holder's fact as the Scalar its column types.
    assert.equal(message.byTag(35).asJs(), 'D')
    assert.equal(message.byId(registry.fieldByTag(35).fix.id).asJs(), 'D')
    assert.equal(message.byName('MsgType').asJs(), 'D')
    assert.equal(message.byTag(54).asJs(), 'BUY')
    assert.ok(message.byTag(52).equals(SENDING))
    assert.ok(message.byTag(65003).equals(SENDING), 'unix, as the clock its column holds')
    assert.equal(message.byTag(65039).asJs(), message.curruuid)
    assert.equal(message.byTag(65017).asJs(), message.currhashcode)
    assert.equal(message.byTag(65048).asJs(), 'C-1')
    assert.equal(message.byTag(55).asJs(), 'AAPL')
    assert.equal(message.byId(registry.fieldByTag(55).fix.id).asJs(), 'AAPL')
    assert.equal(message.byName('SYMBOL').asJs(), 'AAPL')
    assert.equal(message.byTag(38).toString(), '"100.000000000000000000"')
    assert.equal(message.byPath('parties[0].partyid').asJs(), 'BROKER')
    // An unknown tag is retained under its rendered name, never dropped.
    assert.equal(message.byTag(9999).asJs(), 'custom')
    // A fact the message states nothing for is absent, not null.
    assert.equal(message.getByTag(44), null, 'no price')
    assert.equal(message.getByTag(58), null, 'no text')
    assert.equal(message.getByTag(65049), null, 'no metadata')
    // An identifier is exact: one the dictionary holds no field under misses,
    // and so does one whose field the root does not declare.
    const stray = fixField('TradeID', 'utf8', 5001)
    assert.equal(message.getById(stray.fix.id), null)
    assert.equal(message.getById(registry.fieldByTag(44).fix.id), null)

    assert.ok(message.get(55).equals(message.byTag(55)))
    assert.ok(message.at('symbol').equals(message.byTag(55)))
    assert.equal(message.get(1234), null)
    assert.equal(message.getByName('nope'), null)
    assert.equal(message.getByPath('parties.partyid'), null)
    assert.throws(
      () => message.byTag(1234),
      /^Error: expected a fix value at "tag 1234", got nothing$/,
    )
    assert.throws(
      () => message.byId(stray.fix.id),
      new RegExp(`^Error: expected a fix value at "identifier ${stray.fix.id}", got nothing$`),
    )
    assert.throws(() => message.byName('nope'), /name \\"nope\\"/)
    assert.throws(() => message.byPath('parties.partyid'), /path parties\.partyid/)
    assert.throws(() => message.at(55n), {
      name: 'TypeError',
      message: 'key must be a number tag or a string name, got BigInt',
    })
    assert.throws(() => message.byTag(2 ** 31), /tag must be a signed 32-bit integer/)
    // An identifier that is not an exact number is refused, never a miss.
    assert.throws(() => message.byId(1.5), /id must be a signed 32-bit integer/)
    assert.throws(() => message.getById(2 ** 31), /id must be a signed 32-bit integer/)
    assert.throws(() => message.getById('55'), /into rust type `f64`/)
  })

  test('the event view exposes every precise clock, market and operation fact', () => {
    const message = fixedCodec(seed()).parseFixLine(Buffer.from(
      '8=FIX.4.4|35=D|49=SENDER|56=TARGET|34=7|50=TRADER|52=20240102-10:15:30|1=ACC-1|11=A1|37=O-1|55=AAPL|22=4|48=US0378331005|54=1|15=USD|38=100|996=Shares|44=10.5|31=10.25|32=40|6=10.3|14=40|151=60|140=9.75|58=note|60=20240102-10:15:31|10=0|',
    ))
    const event = message.event()

    assert.equal(event.currunix, SENDING_NS + 1_000_000_000n)
    assert.equal(event.creaunix, event.currunix)
    assert.equal(event.execunix, null)
    assert.equal(event.recdunix, null)
    // A merge keeps no clock of the reference it chose.
    assert.equal('refrecdunix' in event, false)
    assert.equal(event.marketoperationid, 10)
    assert.equal(event.price, '10.5')
    assert.equal(event.quantity, '100')
    assert.equal(event.lastpx, '10.25')
    assert.equal(event.lastqty, '40')
    assert.equal(event.avgpx, '10.3')
    assert.equal(event.cumqty, '40')
    assert.equal(event.leavesqty, '60')
    assert.equal(event.prevpx, '9.75')
    assert.equal(event.prevqty, null)
    assert.equal(event.tif, '0')
    assert.equal(event.tradable, null)
    // The market facts under their trait names: the instrument by its
    // ticker and its security identifiers, source to code - the CUSIP an
    // American ISIN embeds derived beside the stated ISIN - what it is
    // priced and counted in, and the FX parts no FIX lift fills yet.
    assert.equal(event.ticker, 'AAPL')
    assert.equal(event.currency, 'USD')
    assert.equal(event.unit, 'Shares')
    assert.equal(event.side, 'BUY')
    assert.deepEqual(event.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
    assert.equal(event.cficode, null)
    assert.equal(event.miccode, null)
    assert.equal(event.spotrate, null)
    assert.equal(event.forwardpoints, null)
    assert.deepEqual(event.metadata, {})
    // The operation facts: each map built from the fields that state it,
    // upper-cased keys in key order, and the lane a buy at a price fills
    // for itself - the other lane is the other party's.
    assert.deepEqual(event.accountids, { ACCOUNT: 'ACC-1' })
    assert.deepEqual(event.userids, { SENDERSUBID: 'TRADER' })
    assert.deepEqual(event.altids, { CLORDID: 'A1', ORDERID: 'O-1' })
    assert.deepEqual(event.bid, {
      price: '10.5', spotrate: null, forwardpoints: null, currency: 'USD', quantity: '100', unit: 'Shares',
    })
    assert.equal(event.ask, null)
    // The message answers the same facts as getters.
    for (const name of [
      'ticker', 'unit', 'securityids', 'spotrate', 'forwardpoints', 'metadata',
      'accountids', 'userids', 'altids', 'bid', 'ask',
    ]) {
      assert.deepEqual(message[name], event[name], name)
    }
    // The retired spellings are gone rather than aliased.
    for (const gone of [
      'identifiers', 'isincode', 'cusipcode', 'sedolcode', 'bloombergcode', 'figicode', 'symbolticker',
      'bidpx', 'bidqty', 'bidcurrency', 'bidunit', 'askpx', 'askqty', 'askcurrency', 'askunit',
    ]) {
      assert.equal(gone in event, false, gone)
      assert.equal(gone in message, false, gone)
    }
  })

  test('the entries are the content row read as a tree, and the wire is the header before them', () => {
    const registry = seed()
    const message = new fix.FixMsg(order(registry), ORDER_VALUE, registry)

    // One entry per child the row states, a group as its counter entry with
    // the occurrence's members nested under an entry of their own; a key no
    // dictionary explains carries tag 0 and its own spelling.
    const entries = message.entries()
    assert.deepEqual(entries.map((entry) => [entry.tag, entry.name, entry.value]), [
      [55, 'symbol', 'AAPL'],
      [54, 'side', '1'],
      [453, 'parties', '1'],
      [0, '9999', 'custom'],
    ])
    assert.deepEqual(entries[2].entries, [
      {
        tag: 0,
        name: 'party',
        value: null,
        entries: [
          { tag: 448, name: 'partyid', value: 'BROKER', entries: [] },
          { tag: 447, name: 'partyidsource', value: 'D', entries: [] },
          { tag: 452, name: 'partyrole', value: '1', entries: [] },
        ],
      },
    ])
    assert.ok(entries.every((entry) => Array.isArray(entry.entries)))
    // Iterating the message walks the entries.
    assert.deepEqual([...message], entries)
    // The typed facts are not entries: the wire puts the header and the
    // fields the message lifted in front of them, coded facts as their wire
    // code, and `SendingTime` only because the root stated it. What the
    // message merely *implies* about its market - the bid lane a buy of a
    // hundred fills - reaches no byte of it.
    const wire =
      '35=D|52=20240102-10:15:30|11=C-1|38=100|55=AAPL|54=1|453=1|448=BROKER|447=D|452=1|9999=custom|'
    assert.equal(message.intoBytes(124).toString(), wire)
    assert.equal(message.intoText('|'), wire)
    assert.equal(message.intoText(), wire.replaceAll('|', '\x01'))
    assert.deepEqual(message.intoBytes(), Buffer.from(wire.replaceAll('|', '\x01')))
    assert.throws(() => message.intoText('||'), /one character/)
    assert.throws(() => message.intoBytes(256), /one byte/)
    // The digest is over the wire entries: sixteen bytes, the same for two
    // builds and another for other content.
    assert.equal(message.digest().length, 16)
    assert.deepEqual(message.digest(), new fix.FixMsg(order(registry), ORDER_VALUE, registry).digest())
    assert.notDeepEqual(
      message.digest(),
      new fix.FixMsg(order(registry), { ...ORDER_VALUE, symbol: 'MSFT' }, registry).digest(),
    )
  })

  test('the identity is settled on every message and follows what it says', () => {
    const registry = seed()
    const root = order(registry)
    const message = new fix.FixMsg(root, ORDER_VALUE, registry)
    const same = new fix.FixMsg(root, ORDER_VALUE, registry)
    const other = new fix.FixMsg(root, { ...ORDER_VALUE, symbol: 'MSFT' }, registry)

    // A UUID crosses as its hyphenated text, a hash as a bigint.
    assert.match(message.curruuid, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/)
    assert.match(message.crossuuid, /^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/)
    assert.equal(typeof message.currhashcode, 'bigint')
    assert.equal(typeof message.crosshashcode, 'bigint')
    assert.notEqual(message.currhashcode, 0n)
    assert.notEqual(message.crosshashcode, 0n)
    // Two builds of one statement are one identity; other content is another
    // message of the same chain.
    assert.equal(message.currhashcode, same.currhashcode)
    assert.equal(message.curruuid, same.curruuid)
    assert.notEqual(other.currhashcode, message.currhashcode)
    assert.notEqual(other.curruuid, message.curruuid)
    assert.equal(other.crossuuid, message.crossuuid)
    assert.equal(other.crosshashcode, message.crosshashcode)
    // The event states the same facts the getters do.
    const event = message.event()
    assert.equal(event.curruuid, message.curruuid)
    assert.equal(event.crossuuid, message.crossuuid)
    assert.equal(event.currhashcode, message.currhashcode)
    assert.equal(event.crosshashcode, message.crosshashcode)
    assert.equal(event.crosscode, message.crosscode)
    assert.deepEqual(event.altids, message.altids)
    assert.deepEqual(event.securityids, message.securityids)
    assert.equal(event.side, message.side)
    assert.equal(event.currunix, message.currunix)
    assert.equal(event.state, message.state)
    assert.equal(event.seqnum, message.seqnum)
    assert.equal(event.prevuuid, message.prevuuid)
    assert.equal(event.marketoperationid, message.marketoperationid)
    assert.equal(event.price, message.price)
    assert.equal(event.quantity, message.quantity)
    assert.equal('px' in event, false)
    assert.equal('qty' in event, false)
    // A buy of a hundred at no price fills the bid lane's size and nothing
    // else; the other lane is the other party's.
    assert.deepEqual(event.bid, {
      price: null, spotrate: null, forwardpoints: null, currency: null, quantity: '100', unit: null,
    })
    assert.equal(event.ask, null)
    assert.deepEqual(event.securityids, {})
    for (const code of ['cficode', 'miccode']) {
      assert.equal(event[code], null, code)
    }
    assert.equal(event.unit, '')
    assert.equal(event.exprtime, null)
    assert.equal(event.prevunix, null)
    assert.equal(event.snapunix, null)
    // No cross code names no chain: the chain identity is the message's own.
    const heartbeat = new fix.FixMsg(
      fields.struct('Heartbeat', [registry.fieldByTag(35), registry.fieldByTag(52)], { nullable: false }),
      { msgtype: '0', sendingtime: SENDING },
      registry,
    )
    assert.equal(heartbeat.crosscode, '')
    assert.equal(heartbeat.crosshashcode, 0n)
    assert.equal(heartbeat.crossuuid, heartbeat.curruuid)
    assert.equal(heartbeat.size, 0)
    assert.deepEqual(heartbeat.entries(), [])
    assert.equal(heartbeat.intoText('|'), '35=0|52=20240102-10:15:30|')
  })

  test('the bridge session event is captured but never the crosscode or content', () => {
    // FIX names the chain; a complete bridge bracket names its delivery.
    const registry = seed()
    const message = fixedCodec(registry).parseUllinkLine(Buffer.from(
      'MSGTYPE=8|#ORDERID=ORDER-1|#CLORDID=CLIENT-1|#MSGSESSIONID=SESSION-1|' +
      '#MSGCTXID=CONTEXT-1|#MSGSEQNUM=7|#SYMBOL=n/A|#VENUEOWNTHING=n/A|',
    ))

    // The message type, session, context and sequence joined as they are
    // stated, on the capture: delivery provenance, never a name the message
    // goes by.
    assert.equal(message.crosscode, 'ORDER-1')
    assert.equal(message.capture().msgsesseventid, '8:SESSION-1:CONTEXT-1:7')
    assert.equal(message.byTag(65065).asJs(), '8:SESSION-1:CONTEXT-1:7')
    assert.deepEqual(message.altids, { CLORDID: 'CLIENT-1', ORDERID: 'ORDER-1' })
    assert.equal(message.getByTag(55), null)
    assert.equal(message.getByName('venueownthing'), null)
    const contentHash = message.currhashcode
    const contentUuid = message.curruuid

    // Every write settles it again, and none of it is content.
    message.set('msgsessionid', 'SESSION-2')
    assert.equal(message.capture().msgsessionid, 'SESSION-2')
    assert.equal(message.capture().msgsesseventid, '8:SESSION-2:CONTEXT-1:7')
    assert.deepEqual(message.altids, { CLORDID: 'CLIENT-1', ORDERID: 'ORDER-1' })
    assert.equal(message.currhashcode, contentHash)
    assert.equal(message.curruuid, contentUuid)

    // A missing part unsays it rather than leaving a stale key behind.
    message.set('msgctxid', null)
    assert.equal(message.capture().msgctxid, null)
    assert.equal(message.capture().msgsesseventid, null)
    assert.equal(message.getByTag(65065), null)
    assert.equal(message.currhashcode, contentHash)

    message.set('msgctxid', 'CONTEXT-2')
    assert.equal(message.capture().msgsesseventid, '8:SESSION-2:CONTEXT-2:7')
    assert.equal(message.currhashcode, contentHash)

    // It is a column of the fixed row, closing the session band it is joined
    // from, and a row read back states it again. The merge reference's
    // recording clock is a column no longer.
    const schema = fix.schema(registry)
    const at = schema.indexOf('msgsesseventid')
    assert.equal(at, schema.indexOf('msgsessionid') + 1)
    assert.equal(schema.fieldAt(at).fix.tag, 65065)
    assert.equal(schema.indexOf('refrecdunix'), null)
    const row = message.intoRow(schema)
    assert.equal(row.asJs()[at], '8:SESSION-2:CONTEXT-2:7')
    const rebuilt = fix.FixMsg.fromRow(schema, row, registry)
    assert.equal(rebuilt.crosscode, message.crosscode)
    assert.deepEqual(rebuilt.altids, message.altids)
    assert.deepEqual(rebuilt.capture(), message.capture())
    assert.ok(rebuilt.intoRow(schema).equals(row))
  })

  test("a line's session event joins its four values as stated", () => {
    // The bridge's session and context captures, the type and the sequence.
    const codec = fixedCodec(seed(), { captureNames: ['msgsessionid', 'msgctxid'] })
    const line = new TextLine(0n, '8=FIX.4.4|35=8|34=1094|10=0|', ['e7256476', '9effef3e6a'])
    const [message] = codec.parseTextLine(line)

    const capture = message.capture()
    assert.equal(capture.msgsessionid, 'e7256476')
    assert.equal(capture.msgctxid, '9effef3e6a')
    // Joined by `:` with nothing in front of a part, and held by the capture
    // rather than among the names the message goes by.
    assert.equal(capture.msgsesseventid, '8:e7256476:9effef3e6a:1094')
    assert.equal(message.byTag(65065).asJs(), '8:e7256476:9effef3e6a:1094')
    assert.equal('msgsesseventid' in message.altids, false)

    // A part missing is no session event at all.
    for (const [body, captures] of [
      ['8=FIX.4.4|35=8|10=0|', ['e7256476', '9effef3e6a']],
      ['8=FIX.4.4|35=8|34=1094|10=0|', ['e7256476', null]],
    ]) {
      const [partial] = codec.parseTextLine(new TextLine(0n, body, captures))
      assert.equal(partial.capture().msgsesseventid, null, body)
    }
  })

  test('a message is a value: equality, hash, clone and JSON', () => {
    const registry = seed()
    const root = order(registry)
    const message = new fix.FixMsg(root, ORDER_VALUE, registry)
    const same = new fix.FixMsg(root, ORDER_VALUE, registry)

    assert.ok(message.equals(same))
    assert.equal(message.stableHash(), same.stableHash())
    assert.equal(typeof message.stableHash(), 'bigint')
    assert.equal(message.equals(new fix.FixMsg(root, { ...ORDER_VALUE, symbol: 'MSFT' }, registry)), false)
    // A typed fact is part of the value: another side is another message.
    assert.equal(message.equals(new fix.FixMsg(root, { ...ORDER_VALUE, side: '2' }, registry)), false)

    const copy = message.clone()
    assert.ok(copy.equals(message))
    assert.ok(copy.registry.equals(registry))
    assert.equal(copy.curruuid, message.curruuid)
    assert.equal(message.toString(), 'FixMsg("NewOrderSingle", 5 values)')

    // The JSON is the content row's two documents; the typed facts are the
    // holders' to answer.
    const document = message.toJSON()
    assert.deepEqual(Object.keys(document), ['field', 'value'])
    assert.equal(document.field.metadata['FIX:tag'], undefined, 'the root carries no tag')
    assert.equal(document.value[0], 'AAPL')
    assert.equal(document.value.length, CONTENT.length)
    assert.ok(JSON.stringify(message).includes('"NewOrderSingle"'))
  })

  test("a venue's field and MsgType are both reachable from a venue message", () => {
    const registry = fix.FixRegistry.fromFields([
      fixField('MsgType', 'utf8', 35),
      fixField('TradeID', 'utf8', 5001, { branches: ['cme'], names: ['VenueTrade'] }),
      fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
    ])
    const root = fields.struct(
      'VenueOrder',
      [Field.from('MsgType: utf8'), Field.from('TradeID: utf8'), Field.from('Symbol: utf8')],
      { nullable: false },
    )
    const message = new fix.FixMsg(
      root,
      { MsgType: 'D', TradeID: 'T-1', Symbol: 'AAPL' },
      registry,
    )

    // One namespace, one step: the venue's own field, the specification's,
    // and every alias either declares resolve alike, whatever dictionary
    // contributed them - membership is provenance, never a tier. The type is
    // the header's, whatever the root spelled it as.
    assert.equal(message.byTag(5001).asJs(), 'T-1')
    assert.equal(message.byName('venuetrade').asJs(), 'T-1')
    assert.equal(message.byTag(35).asJs(), 'D')
    assert.equal(message.header().msgtype, 'D')
    assert.equal(message.size, 2)
    assert.equal(message.byName('ticker').asJs(), 'AAPL')
    assert.equal(message.byId(registry.fieldByTag(5001).fix.id).asJs(), 'T-1')
    assert.equal(message.byId(registry.fieldByTag(35).fix.id).asJs(), 'D')
    assert.ok(registry.fieldByTag(5001).fix.hasBranch('cme'))
    assert.equal(registry.fieldByTag(35).fix.hasBranch('cme'), false)
    // A message root is not a dictionary member: it carries no membership.
    assert.deepEqual(message.field.fix.branches, [])

    // A root that does not declare the child misses it, whatever the
    // dictionary holds.
    const plain = fields.struct(
      'Order',
      [Field.from('MsgType: utf8'), Field.from('TradeID: utf8')],
      { nullable: false },
    )
    const standard = new fix.FixMsg(plain, { MsgType: 'D', TradeID: 'T-1' }, registry)
    assert.equal(standard.byTag(35).asJs(), 'D')
    assert.equal(standard.byTag(5001).asJs(), 'T-1')
    assert.equal(standard.getByTag(55), null)
    assert.equal(standard.getByName('ticker'), null)
  })

  test('a message refuses a value its field refuses', () => {
    const registry = seed()
    const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })

    // A text field reads any value that spells text, a number included, so what
    // it refuses is a value with no spelling at all. The dictionary folds its
    // names, so the field the refusal names is `symbol`.
    assert.throws(() => new fix.FixMsg(root, { symbol: [1] }, registry), /symbol/)
    assert.throws(() => new fix.FixMsg(Field.from('scalar: utf8'), { symbol: 'AAPL' }, registry))
    assert.throws(() => fix.FixMsg(root, { symbol: 'AAPL' }, registry), /without 'new'/)
  })

  test('a message links the process default when none is named', () => {
    const registry = fix.FixRegistry.fromFields([fixField('Symbol', 'utf8', 55)])
    const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })

    const global = fix.globalRegistry()
    assert.ok(global instanceof fix.FixRegistry)
    // Whatever this machine has installed, the two calls answer one dictionary.
    assert.ok(global.equals(fix.globalRegistry()))
    assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }).registry.equals(global))
    // An explicit registry is kept instead.
    assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }, registry).registry.equals(registry))
  })

  test('a registry is a value: equality, hash, clone, JSON and text', () => {
    const registry = seed()

    assert.ok(registry.equals(seed()))
    assert.equal(registry.stableHash(), seed().stableHash())
    assert.equal(typeof registry.stableHash(), 'bigint')
    assert.equal(registry.equals(new fix.FixRegistry()), false)
    assert.equal(registry.toString(), `FixRegistry(${registry.size} fields)`)
    // A new registry is never empty: it holds the crate's own fields and the
    // seeded SendingTime and TransactTime clocks.
    assert.equal(new fix.FixRegistry().size, SEEDED)
    assert.equal(new fix.FixRegistry().toString(), `FixRegistry(${SEEDED} fields)`)

    const document = registry.toJSON()
    // The crate's own are seeded and stored alike, so the snapshot states the
    // store's fields and the crate's scalars beside them.
    assert.equal(document.fields.length, STORED + CRATE_SCALARS.length)
    const stored = document.fields[0]
    const held = JSON.parse(JSON.stringify(registry.fieldByTag(1)))
    assert.equal(stored.name, held.name)
    assert.deepEqual(stored.dtype, held.dtype)
    assert.equal(stored.metadata['FIX:tag'], '1')
    // A field states the name of the vocabulary it reads by and nothing more,
    // in the snapshot exactly as on the field: the members are the
    // dictionary's, under `codesets`, which leads the document because a
    // reader has the sets before it meets a field naming one.
    const coded = document.fields.find((field) => field.metadata['FIX:codeset'] !== undefined)
    const codedHeld = JSON.parse(JSON.stringify(registry.fieldByTag(Number(coded.metadata['FIX:tag']))))
    assert.equal(typeof coded.metadata['FIX:codeset'], 'string')
    assert.equal(codedHeld.metadata['FIX:codeset'], coded.metadata['FIX:codeset'])
    assert.equal(Object.keys(document)[0], 'codesets')
    const set = document.codesets.find((held) => held.name === coded.metadata['FIX:codeset'])
    assert.deepEqual(set.codes, registry.codeset(set.name).codes)
    assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
  })

  test('the fix namespace is frozen and the raw exports are gone', () => {
    const yggdryl = require('yggdryl')

    assert.ok(Object.isFrozen(fix))
    assert.deepEqual(
      Object.keys(fix).sort(),
      [
        'FixCodec',
        'FixMessages',
        'FixMsg',
        'FixRegistry',
        'MsgType',
        'ULBRIDGE_ROWHEADER',
        'crateFields',
        'globalRegistry',
        'installGlobalRegistry',
        'schema',
        'schemaCarrying',
        'schemaTags',
      ],
    )
    for (const name of [
      'FixFieldIterator',
      'FixMsg',
      'FixMsgEntries',
      'FixCodec',
      'FixLifecycle',
      'FixDefinitionIterator',
      'MsgTypeIterator',
      'FixRegistry',
      'JsFixMsg',
      'JsFixCodec',
      'JsFixLifecycle',
      'JsFixRegistry',
      'fixCrateFields',
      'fixSchema',
      'fixSchemaCarrying',
      'fixSchemaTags',
      'fixGlobalRegistryNative',
      'fixInstallGlobalRegistryNative',
    ]) {
      assert.equal(name in yggdryl, false, name)
    }
    assert.equal(typeof fix.installGlobalRegistry, 'function')
  })

  test('installing the process default wins before anything resolves it', () => {
    // Process-wide state, so it is driven in a process of its own.
    const script = `
    const assert = require('node:assert/strict')
    const { Field, fields, fix } = require(process.argv[1])
    const seed = fix.FixRegistry.fromHandle(process.argv[2])
    fix.installGlobalRegistry(seed)
    assert.ok(fix.globalRegistry().equals(seed))
    assert.equal(fix.globalRegistry().fieldByTag(55).name, 'symbol')
    assert.equal(fix.globalRegistry().fieldByName('SYMBOL').name, 'symbol')
    const root = fields.struct('row', [fix.globalRegistry().fieldByTag(55)], { nullable: false })
    assert.ok(new fix.FixMsg(root, { symbol: 'AAPL' }).registry.equals(seed))
    assert.throws(() => fix.installGlobalRegistry(new fix.FixRegistry()), /already resolved/)
    console.log('ok')
  `
    const { execFileSync } = require('node:child_process')
    const output = execFileSync(
      process.execPath,
      ['-e', script, require.resolve('yggdryl'), SEED],
      { encoding: 'utf8' },
    )
    assert.equal(output.trim(), 'ok')
  })

  test('a reader parses every frame shape the core reads', () => {
    const registry = seed()
    // Two reads of undated bytes agree only under one stated intake clock.
    const reader = fixedCodec(registry)

    assert.equal(reader.parseLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|')).next().value.byTag(55).toJSON(), 'AAPL')
    assert.equal(reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|10=0|')).next().value.byTag(55).toJSON(), 'AAPL')
    assert.equal(
      reader.parseFixLine(Buffer.from('8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01')).byTag(55).toJSON(),
      'AAPL',
    )
    assert.equal(reader.parsePairs([['55', 'AAPL']]).byTag(55).toJSON(), 'AAPL')
    assert.ok(reader.registry.equals(registry))

    // What every built message has, whatever its line carried: a header
    // opening with `beginstring` - the wire's own, else the version it was read
    // at - and the sending clock the codec settles for an undated one, which
    // stamps the event and its creation alike and never goes back on the wire.
    const pairs = reader.parsePairs([['55', 'AAPL']])
    assert.deepEqual([...pairs.field.dtype.keys()], ['symbol'])
    assert.equal(pairs.header().beginstring, 'FIX.4.4')
    assert.equal(pairs.header().msgtype, '')
    assert.equal(pairs.byTag(8).toJSON(), 'FIX.4.4')
    assert.equal(pairs.header().sendingtime, SENDING_NS)
    assert.ok(pairs.byTag(52).equals(SENDING))
    assert.equal(pairs.currunix, SENDING_NS)
    assert.equal(pairs.event().creaunix, SENDING_NS)
    assert.deepEqual(flat(pairs), [[55, 'symbol', 'AAPL']])
    assert.equal(pairs.intoText('|'), '8=FIX.4.4|55=AAPL|')
    // The stated clock is the one that goes back out and the reference the
    // event is dated against; this transaction stands two years in front of
    // it, far outside the codec's default one-second delay, so the sending
    // clock dates the event.
    const dated = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20240102-10:15:30|60=20260102-10:15:31.5|11=A|10=0|'))
    assert.equal(dated.header().sendingtime, SENDING_NS)
    assert.equal(dated.currunix, SENDING_NS)
    assert.ok(dated.byTag(60).equals(new DataType('datetime64(ns,"UTC")').scalar(1_767_348_931_500_000_000n)))
    assert.equal(dated.intoText('|').startsWith('8=FIX.4.4|35=D|52=20240102-10:15:30|'), true)
    // A transaction time stating only a day is no different: the sending
    // clock stands.
    const day = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|60=20260814|11=A|10=0|'))
    assert.equal(day.currunix, SENDING_NS)

    // A bridge frame, byte for byte: `#`-prefixed name keys, one occurrence
    // whose value packs its members behind the two control bytes ULLINK uses.
    const bridge = reader.parseUllinkLine(
      Buffer.from(
        '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2' +
          '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
        'binary',
      ),
    )
    const inferred = reader.parseLine(
      Buffer.from(
        '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2' +
          '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
        'binary',
      ),
    ).next().value
    assert.ok(inferred.equals(bridge))
    assert.equal(bridge.byTag(55).toJSON(), 'TTF')
    assert.equal(bridge.byTag(38).toJSON(), '1200.000000000000000000')
    assert.equal(bridge.byTag(44).toJSON(), '41.250000000000000000')
    assert.equal(bridge.side, 'BUY')
    assert.equal(bridge.byPath('parties[0].partyid').asJs(), 'BUYSIDE')
    // The counter said two occurrences and one arrived: the group entry
    // counts what is there.
    const parties = bridge.entries().find((entry) => entry.tag === 453)
    assert.equal(parties.value, '1')
    assert.equal(parties.entries.length, 1)
    assert.equal(bridge.digest().length, 16)
  })

  test('a reader takes the pins the core takes', () => {
    const registry = seed()

    // Tag 32 is `lastshares` at 4.2 and `lastqty` from 4.3 on. A version
    // settles how a value is read, never what a field is called: what the 4.2
    // spelling restates to is the quantity the event last traded, which the
    // holder answers under either name.
    const dated = reading(registry)
    const named = dated.parseLine(Buffer.from('8=FIX.4.2|35=8|32=100|10=0|')).next().value
    assert.equal(named.field.indexOf('lastqty'), null, 'the event holds it')
    assert.equal(named.lastqty, '100')
    assert.ok(named.getByName('lastshares') !== null)
    assert.ok(named.getByName('lastqty') !== null)
    assert.equal(named.header().beginstring, 'FIX.4.2')

    // A codec pins no version: a row states one, or the line implies it.
    assert.equal(dated.version, undefined)

    // A stated absence produces no field at all.
    const silent = reading(registry, { nullValues: ['<none>'] })
    assert.equal(silent.parseLine(Buffer.from('8=FIX.4.4|35=D|55=<none>|10=0|')).next().value.getByTag(55), null)
  })

  test('a parse derives what the dictionary derives and states it on the wire', () => {
    const reader = fixedCodec(seed())

    // There is no enriching pass: the parse runs the dictionary's own
    // `FIX:derivation` rules. A `SecurityID` an ISIN's check digit closes has
    // stated its source, and under that source the event's ISIN and the
    // country its prefix names; an order stating no time in force is a day
    // order.
    const line = '8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|'
    const filled = reader.parseLine(Buffer.from(line)).next().value
    assert.equal(filled.byTag(22).toJSON(), '4')
    assert.equal(filled.securityids.ISIN, 'US0378331005')
    assert.equal(filled.byTag(470).toJSON(), 'US')
    assert.equal(filled.byTag(59).toJSON(), '0', 'an order stating no time in force is a day order')
    // What the message now states is what it emits: the frame leads, the
    // fields it lifted follow, then the row with the day order among it.
    assert.equal(
      filled.intoText('|'),
      '8=FIX.4.4|35=D|11=A|48=US0378331005|22=4|59=0|470=US|10=0|',
    )
    // `TimeInForce(59)` is an ordinary column, so what the dictionary derived
    // for it is an entry like any other, and the trait reads the standing off
    // it.
    assert.ok(flat(filled).some(([tag]) => tag === 59))
    assert.equal(filled.tif, '0')

    // A value no standard closes answers nothing rather than a guess.
    const opaque = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|')).next().value
    assert.equal(opaque.getByTag(22), null)
    assert.deepEqual(opaque.event().securityids, {})
  })

  test('the official time delay bounds which clock dates the message', () => {
    const registry = seed()
    assert.equal(new fix.FixCodec(registry).officialTimeDelayMs, 1_000)
    assert.equal(new fix.FixCodec(registry, { officialTimeDelayMs: undefined }).officialTimeDelayMs, 1_000)
    assert.equal(new fix.FixCodec(registry, { officialTimeDelayMs: 0 }).officialTimeDelayMs, 0)
    assert.equal(new fix.FixCodec(registry, { officialTimeDelayMs: -1 }).officialTimeDelayMs, -1)
    assert.throws(() => new fix.FixCodec(registry, { officialTimeDelayMs: 1.5 }), /whole number/i)

    const codec = reading(registry)
    const SENT = 1_787_308_200_415_000_000n
    // A transaction half a second in front of the sending clock is the same
    // event said twice, so the more exact saying of it dates the message.
    const near = codec.parseFixLine(
      Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|11=A|10=0|'),
    )
    assert.equal(near.currunix, 1_787_308_199_900_000_000n)
    assert.equal(near.event().creaunix, near.currunix)
    // Five seconds out is a different event of the session's day.
    const apart = codec.parseFixLine(
      Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:55|11=A|10=0|'),
    )
    assert.equal(apart.currunix, SENT)

    // A message stating no transaction is dated by the regulatory stamp its
    // `TrdRegTimestampType(770)` says is about the event; the nearer stamp is
    // when the report reached a repository, which is not that.
    const stamped = codec.parseFixLine(
      Buffer.from(
        '8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=2|' +
          '769=20260821-10:30:00.400|770=23|769=20260821-10:29:59.900|770=1|10=0|',
      ),
    )
    assert.equal(stamped.currunix, 1_787_308_199_900_000_000n)
    const unranked = codec.parseFixLine(
      Buffer.from('8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|769=20260821-10:30:00.400|770=23|10=0|'),
    )
    assert.equal(unranked.currunix, SENT)

    // The pin is the caller's to widen and to close.
    const wide = reading(registry, { officialTimeDelayMs: 10_000 })
    assert.equal(
      wide.parseFixLine(
        Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:55|11=A|10=0|'),
      ).currunix,
      1_787_308_195_000_000_000n,
    )
    const shut = reading(registry, { officialTimeDelayMs: 0 })
    assert.equal(
      shut.parseFixLine(
        Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|11=A|10=0|'),
      ).currunix,
      SENT,
    )
  })

  test("a single-sided quote reads as its lane's side", () => {
    const codec = fixedCodec(seed())

    // A bid alone is a buy at the bid: the price, the quantity and the lane's
    // currency read off it, and none of it reaches the wire.
    const bid = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=S|117=Q1|55=AAPL|15=USD|132=101.5|134=200|10=0|'))
    assert.equal(bid.side, 'BUY')
    assert.equal(bid.price, '101.5')
    assert.equal(bid.quantity, '200')
    assert.equal(bid.currency, 'USD')
    assert.ok(!bid.intoText('|').includes('|54='))
    // The lane itself crosses whole, each slot as stated and null where
    // none is, and the other lane is the other party's.
    assert.deepEqual(bid.bid, {
      price: '101.5', spotrate: null, forwardpoints: null, currency: 'USD', quantity: '200', unit: null,
    })
    assert.equal(bid.ask, null)

    // An offer alone is a sell at the offer.
    const offer = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=S|117=Q2|55=AAPL|133=102|135=50|10=0|'))
    assert.equal(offer.side, 'SELL')
    assert.equal(offer.price, '102')
    assert.equal(offer.quantity, '50')
    assert.deepEqual(offer.ask, {
      price: '102', spotrate: null, forwardpoints: null, currency: null, quantity: '50', unit: null,
    })
    assert.deepEqual(offer.event().ask, offer.ask)
    assert.equal(offer.bid, null)

    // Both lanes name no side; a stated side stands whatever lane it quotes.
    const two = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=S|117=Q3|55=AAPL|132=101|133=102|10=0|'))
    assert.equal(two.side, 'UNKNOWN')
    assert.equal(two.bid.price, '101')
    assert.equal(two.bid.quantity, null)
    assert.equal(two.ask.price, '102')
    const stated = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=S|117=Q4|55=AAPL|54=2|132=101|10=0|'))
    assert.equal(stated.side, 'SELL')
  })

  test('an execution report stating no execution clock executed at its instant', () => {
    // Intake dates the execution a report states, rather than a later walk.
    const registry = seed()
    const codec = fixedCodec(registry)

    // A fill stating no ExecutionTimestamp, no execution TrdRegTimestamp and
    // no TransactTime executed when it happened, and is a raw observation.
    const fill = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|10=0|'))
    assert.equal(fill.currunix, SENDING_NS)
    assert.equal(fill.event().execunix, SENDING_NS)
    assert.equal(fill.prevuuid, null)
    // The row states it, and a row read back keeps it.
    const schema = fix.schema(registry)
    const row = fill.intoRow(schema)
    assert.ok(row.at(schema.indexOf('execunix')).equals(SENDING))
    assert.equal(fix.FixMsg.fromRow(schema, row, registry).event().execunix, SENDING_NS)

    // A trade's own TransactTime is its execution clock.
    const traded = codec.parseFixLine(
      Buffer.from('8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|60=20240102-10:15:30.5|10=0|'),
    )
    assert.equal(traded.event().execunix, SENDING_NS + 500_000_000n)

    // An acknowledgement and an order report no execution.
    const acknowledged = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=8|37=O1|17=E0|150=0|39=0|10=0|'))
    assert.equal(acknowledged.event().execunix, null)
    const order = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|10=0|'))
    assert.equal(order.event().execunix, null)
  })

  test('the lifecycle redirects categories snapshots dedup and normalized rows', () => {
    const intrinsic = new fix.FixRegistry()
    assert.throws(() => intrinsic.setCodeset('msgcatcodeset', []), /fixed MsgCat operation identifiers/)
    assert.throws(
      () => intrinsic.mergeCodeset('msgcatcodeset', [{ value: '99', name: 'ORDR' }]),
      /fixed MsgCat operation identifiers/,
    )
    assert.throws(() => intrinsic.removeCodeset('msgcatcodeset'), /fixed MsgCat operation identifiers/)

    const registry = seed()
    assert.equal(new fix.FixCodec(registry).snapshotNs, null)
    assert.equal(new fix.FixCodec(registry, { snapshotNs: undefined }).snapshotNs, null)
    assert.equal(new fix.FixCodec(registry, { snapshotNs: null }).snapshotNs, null)
    assert.equal(new fix.FixCodec(registry, { snapshotNs: 0n }).snapshotNs, null)
    assert.equal(new fix.FixCodec(registry, { snapshotNs: -1n }).snapshotNs, null)
    assert.throws(() => new fix.FixCodec(registry, { snapshotNs: 1 }), /bigint/i)
    assert.throws(() => new fix.FixCodec(registry, { snapshotNs: 9_223_372_036_854_775_808n }), /signed 64-bit/i)
    const codec = fixedCodec(registry)
    const snapshots = fixedCodec(registry, { snapshotNs: 1_000_000_000n })
    assert.equal(snapshots.snapshotNs, 1_000_000_000n)
    assert.equal(registry.msgtype('D').msgcat, 'ORDR')

    const original = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|'))
    const replay = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=7|43=Y|52=20260102-10:15:31|122=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|'))
    const distinct = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=8|52=20260102-10:15:32|11=REPLAY-1|55=AAPL|10=0|'))
    assert.equal(original.msgcat, 10)
    assert.equal(original.marketoperationid, 10)
    const deduplicated = [...codec.lifecycle([original, original.clone(), replay, distinct])]
    assert.deepEqual(deduplicated.map((message) => message.header().msgseqnum), [7, 8])
    assert.deepEqual(deduplicated.map((message) => message.seqnum), [0, 1])

    const later = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:31|11=LATER|isincode=US0378331005|10=0|'))
    const earlier = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=EARLIER|isincode=US0378331005|bloombergcode=AAPL US Equity|10=0|'))
    const learned = [...codec.lifecycle([later, earlier])]
    assert.equal(learned[1].securityids.BLOOMBERG, 'AAPL US Equity')
    assert.equal('BLOOMBERG' in later.securityids, false, 'the input is never enriched in place')
    const schema = fix.schema(registry)
    const rebuilt = fix.FixMsg.fromRow(schema, learned[1].intoRow(schema), registry)
    assert.equal(rebuilt.securityids.BLOOMBERG, 'AAPL US Equity')

    const expiring = snapshots.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|'))
    const entries = expiring.entries()
    const deadline = expiring.event().exprtime
    const walked = [...snapshots.lifecycle([expiring])]
    assert.deepEqual(expiring.entries(), entries)
    assert.equal(expiring.event().snapunix, null)
    assert.ok(walked.some((message) => message.currunix === deadline && message.state === '95EXPIRED'))
    const emittedSnapshots = walked.filter((message) => message.event().snapunix !== null)
    assert.ok(emittedSnapshots.length > 0)
    assert.ok(emittedSnapshots.every((message) => message.currunix <= message.event().snapunix && message.event().snapunix < deadline))
  })

  test('the fixed schema places category beside message type and normalized codes once', () => {
    const registry = seed()
    const schema = fix.schema(registry)
    assert.equal(CRATE.length, 28)
    assert.equal(CRATE_SCALARS.length, 27)
    assert.equal(new fix.FixRegistry().size, 30)
    assert.equal(scalars(new fix.FixRegistry()).length, 29)
    assert.equal(schema.fieldLen, 124)
    assert.equal(fix.schemaTags().length, 119)
    const at = schema.indexOf('msgtype')
    assert.deepEqual(
      [schema.fieldAt(at - 1).name, schema.fieldAt(at).name, schema.fieldAt(at + 1).name, schema.fieldAt(at + 2).name],
      ['beginstring', 'msgtype', 'msgcat', 'msgseqnum'],
    )
    // The three crate views of the security identifiers keep their tags;
    // the retired identifiers, cusipcode and sedolcode columns are gone and
    // their tags are never reused.
    for (const [name, tag] of [['isincode', 65055], ['bloombergcode', 65059], ['miccode', 65060], ['figicode', 65061]]) {
      assert.equal(schema.fieldAt(schema.indexOf(name)).fix.tag, tag, name)
    }
    assert.equal(schema.fieldAt(schema.indexOf('cficode')).fix.tag, 461)
    for (const retired of [65020, 65056, 65057, 65058]) {
      assert.equal(fix.schemaTags().includes(retired), false, String(retired))
    }
    for (const name of ['identifiers', 'cusipcode', 'sedolcode']) {
      assert.equal(schema.indexOf(name), null, name)
    }
  })

  test('security source S identifies FIGI while A remains Bloomberg', () => {
    const registry = seed()
    const codec = fixedCodec(registry)
    const figi = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=D|52=20240102-10:15:30|22=S|48=BBG000BLNQ16|454=1|455=BBG000BLNQ16|456=S|10=0|',
    ))
    assert.equal(figi.securityids.FIGI, 'BBG000BLNQ16')
    assert.equal(figi.event().securityids.FIGI, 'BBG000BLNQ16')
    assert.equal('BLOOMBERG' in figi.securityids, false)

    const schema = fix.schema(registry)
    const rebuilt = fix.FixMsg.fromRow(schema, figi.intoRow(schema), registry)
    assert.equal(rebuilt.securityids.FIGI, 'BBG000BLNQ16')
    assert.ok(rebuilt.intoRow(schema).equals(figi.intoRow(schema)))

    const bloomberg = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=D|52=20240102-10:15:30|22=A|48=AAPL US Equity|10=0|',
    ))
    assert.equal(bloomberg.securityids.BLOOMBERG, 'AAPL US Equity')
    assert.equal('FIGI' in bloomberg.securityids, false)

    const stated = figi.clone()
    stated.set(65061, 'BBG000BLNQ16')
    assert.equal(stated.securityids.FIGI, 'BBG000BLNQ16')
  })

  test('a parse restates deprecated fields to their latest aliases', () => {
    // A FIX 4.2 execution report: a transaction type, a partial fill, a Rule80A
    // capacity and two identities the specification later moved into `Parties`.
    const line =
      '8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|'
    const latest = fixedCodec(seed()).parseLine(Buffer.from(line)).next().value
    assert.equal(latest.header().beginstring, 'FIX.4.2')
    // The state the report reached is the event's, ranked; the row keeps the
    // code the wire spelled, restated to the latest alias where one applies.
    assert.equal(latest.state, '40PARTFILL')
    assert.equal(latest.byTag(150).asJs(), 'F')
    assert.equal(latest.byTag(39).asJs(), '1')

    // Rule80A A is an agency order.
    assert.equal(latest.byTag(528).toJSON(), 'A')
    // ExecBroker and ClientID are two parties, in tag order, counted.
    assert.equal(latest.byTag(453).asJs(), 2)
    assert.equal(latest.byPath('parties[0].partyid').asJs(), 'BRKR')
    assert.equal(latest.byPath('parties[0].partyrole').asJs(), 1)
    assert.equal(latest.byPath('parties[1].partyid').asJs(), 'CLIENT1')
    assert.equal(latest.byPath('parties[1].partyrole').asJs(), 3)
    // The fill under its newest spelling, reachable by the old one too, and
    // reachable is all it is: an alias is a way of asking rather than a child
    // to store.
    assert.ok(latest.byTag(32).equals(Scalar.decimal(100n)))
    assert.ok(latest.byName('LastShares').equals(Scalar.decimal(100n)))
    assert.equal(latest.lastqty, '100')
    const names = [...latest.field.dtype.keys()]
    assert.equal(names.filter((name) => name === 'lastqty').length, 0, 'the event holds it')
    assert.ok(!names.includes('lastshares'))
    // The event reads the report: the last executed price as `lastpx` and
    // no price stated, the venue's order identifier as the cross code, the
    // names the report goes by.
    assert.equal(latest.price, null)
    assert.equal(latest.lastpx, '10.5')
    assert.equal(latest.crosscode, 'O1')
    assert.deepEqual(latest.altids, { EXECID: 'E1', ORDERID: 'O1' })
    // One pass, and the filling read the restated row: a report stating no time
    // in force is a day order, one fill's average is that fill's price, and what
    // it was worth is the quantity times the price.
    assert.equal(latest.byTag(59).toJSON(), '0')
    assert.ok(latest.byTag(6).equals(Scalar.decimal(105n, 1)))
    assert.ok(latest.byTag(381).equals(Scalar.decimal(1050n)))
  })

  // A Jolokia answer as a bridge log line writes it: a timestamp and a reader
  // in front of the document, the duration the call took behind it.
  const DOCUMENT = Buffer.from(
    '2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":' +
      '"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting",' +
      '"Version":"4.7.0","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD",' +
      '"BeginString":"FIX.4.4","CurrentPort":9726,"State":"logged"},"status":200} (12 ms)',
  )

  test('a JSON document is one unknown message carrying only what the row stated', () => {
    const codec = fixedCodec(new fix.FixRegistry(), { captureNames: ['msgpluginid'] })
    const messages = codec.parseTextLine(new TextLine(0n, DOCUMENT, ['Router_OrderRouting']))
    const message = messages.next().value
    assert.equal(messages.next().done, true)
    // The codec reads no document: the row is a message that stated no type
    // and no content, so nothing inside the document reaches a fact - the
    // comp ids it spells included - and the wire re-emits the header alone.
    assert.equal(message.field.name, 'unknown')
    assert.deepEqual(message.entries(), [])
    assert.equal(message.size, 0)
    assert.equal(message.header().msgtype, '')
    assert.equal(message.header().sendercompid, null)
    assert.equal(message.header().targetcompid, null)
    assert.equal(message.intoText('|'), '8=FIX.4.4|')
    assert.equal(message.getByName('Name'), null)
    // What the row stated beside the document is carried: the direction the
    // prose in front of it spells, and the capture its header declared.
    assert.equal(message.header().msgdirection, 'R')
    assert.equal(message.byTag(385).asJs(), 'R')
    assert.equal(message.capture().msgpluginid, 'Router_OrderRouting')
    assert.equal(message.byTag(65009).asJs(), 'Router_OrderRouting')
  })

  test('a row reads back into a message stating the same facts', () => {
    const registry = seed()
    const schema = fix.schema(registry)
    const reader = fixedCodec(registry)
    const message = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20240102-10:15:30|54=1|11=A1|55=AAPL|9999=x|10=0|'))
    const row = message.intoRow(schema)
    const held = fix.FixMsg.fromRow(schema, row, registry)

    // Projected columns and residual entries are one semantic message. Their
    // internal child order may differ, while the row and every named fact stay
    // fixed and the row-provided identity remains exact.
    assert.ok(held.field.equals(schema) === false, 'the root is the content, not the schema')
    assert.equal(held.currhashcode, message.currhashcode)
    assert.equal(held.curruuid, message.curruuid)
    assert.equal(held.crossuuid, message.crossuuid)
    assert.deepEqual(held.header(), message.header())
    assert.equal(held.side, 'BUY')
    for (const tag of [8, 35, 11, 55, 54, 52]) assert.ok(held.byTag(tag).equals(message.byTag(tag)), `tag ${tag}`)
    assert.ok(held.intoRow(schema).equals(row))
    // The process default is the registry when none is named.
    assert.notEqual(fix.FixMsg.fromRow(schema, row).registry, null)
    // A row that does not fit the schema is refused.
    assert.throws(() => fix.FixMsg.fromRow(schema, { nosuchcolumn: 1 }, registry))
  })

  test("a capture's own columns lead the row", () => {
    const registry = seed()
    // Nullable, because a message parsed on its own states none of them and a
    // row cell is typed by its column (`rust/tests/fix/msg.rs`).
    const carrier = fields.struct(
      'line',
      [fields.utf8('url', { nullable: true }), fields.binary('body', { nullable: true })],
      { nullable: false },
    )
    const plain = fix.schema(registry, 'FixMessage')
    const carried = fix.schemaCarrying(carrier, plain)

    assert.equal(carried.fieldAt(0).name, 'url')
    assert.equal(carried.fieldAt(1).name, 'body')
    assert.equal(carried.fieldLen, plain.fieldLen + 2)
    assert.equal(carried.indexOf('msgtype'), plain.indexOf('msgtype') + 2)

    // A column no tag names is the capture's, so a row answers null there: the
    // capture fills it, and nothing in the message says what it held.
    const row = reading(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10=0|')).next().value.intoRow(carried).toJSON()
    assert.equal(row[0], null)
    assert.equal(row[carried.indexOf('msgtype')], 'D')

    // A capture column whose folded name a FIX column takes is not carried in
    // front: `msgSessionId` and `msgsessionid` are one name, and the FIX
    // column is the one a reader spelling it means - so the bridge's session
    // instance reaches that column instead of riding in front. `msgthreadid`
    // names no FIX column, so it is carried and leads the row.
    const stamped = fields.struct(
      'line',
      [
        fields.utf8('url', { nullable: false }),
        fields.binary('body', { nullable: false }),
        fields.utf8('msgthreadid'),
        fields.utf8('msgSessionId'),
      ],
      { nullable: false },
    )
    const folded = fix.schemaCarrying(stamped, plain)
    assert.equal(folded.fieldLen, plain.fieldLen + 3)
    assert.equal(folded.indexOf('msgSessionId'), null)
    assert.equal(folded.indexOf('msgthreadid'), 2)
    assert.equal(folded.indexOf('msgsessionid'), plain.indexOf('msgsessionid') + 3)
  })

  test('a CBlock read under a dialect stamps membership on everything it produced', (t) => {
    const root = scratch()
    t.after(() => fs.rmSync(root, { recursive: true, force: true }))
    const file = path.join(root, 'bloomberg.cfb')
    fs.writeFileSync(
      file,
      [
        '<?xml version="1.0"?>',
        '<cplugin-configuration fix-version="4.4">',
        '<vocabulary>',
        '<vocabulary-tag name="10001" alt="ExcludedDealers" type="string" />',
        '<vocabulary-tag name="55" alt="Symbol" type="string" />',
        '</vocabulary>',
        '</cplugin-configuration>',
      ].join('\n'),
      'utf8',
    )
    const [registry, roots] = fix.FixRegistry.fromCfbFile(file, 'Bloomberg')
    assert.equal(roots.length, 0)
    assert.equal(registry.size, 2 + SEEDED)

    // Every field the file produced - a standard tag included, since membership
    // means the dictionary speaks it - carries the dialect, folded once.
    assert.deepEqual(registry.fieldByTag(10001).fix.branches, ['bloomberg'])
    assert.deepEqual(registry.fieldByTag(55).fix.branches, ['bloomberg'])
    assert.ok(registry.fieldByTag(10001).fix.hasBranch('BLOOMBERG'))
    assert.deepEqual(registry.dialects(), ['bloomberg'])
    // The crate's own are nobody's: a definition is filed by the shape it
    // has, so each is reached through the door its shape put it behind.
    assert.ok(fix.crateFields().every((field) => {
      const declared = field.fix.counter !== null
        ? registry.fieldByCounter(field.fix.counter)
        : registry.getFieldByTag(field.fix.tag)
      return declared === null || !declared.fix.hasBranch('bloomberg')
    }))

    // Membership is provenance: the codec reads the one namespace with no pin
    // and the venue's field resolves like any other.
    const message = reading(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10001=DEALER-A|10=0|')).next().value
    assert.equal(message.byName('ExcludedDealers').asJs(), 'DEALER-A')
    assert.equal(message.byTag(10001).asJs(), 'DEALER-A')
    assert.ok(flat(message).every(([tag]) => Number.isInteger(tag)))
    const excluded = reading(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10001=NONE|10=0|')).next().value
    assert.equal(excluded.getByName('ExcludedDealers'), null)
    assert.equal(excluded.getByTag(10001), null)

    // With no dialect named nothing is stamped, and a name that is empty or
    // carries the separator is refused before anything is read.
    const [unstamped] = fix.FixRegistry.fromCfbFile(file)
    assert.deepEqual(unstamped.fieldByTag(10001).fix.branches, [])
    assert.deepEqual(unstamped.dialects(), [])
    assert.equal(fix.FixRegistry.fromCfbFile(file, null)[0].dialects().length, 0)
    assert.throws(() => fix.FixRegistry.fromCfbFile(file, 'a,b'), /FIX:branches/)
    assert.throws(() => fix.FixRegistry.fromCfbFile(file, ''), /FIX:branches/)
  })

  test('a message type keeps its complete wire code and immutable schema', () => {
    const registry = new fix.FixRegistry()
    registry.insert(fixField('msgtype', 'utf8', 35))
    const value = registry.registerMsgtype('P Report Ack', 'AllocationReportAck', 'Allocation Report ACK')
    assert.equal(value.asStr(), 'P Report Ack')
    assert.equal(value.name, 'allocationreportack')
    // The vocabulary is the dictionary's, never a copy on the field: tag 35
    // names the set derived from its own name, and the code registered is a
    // member of that set, its full wire spelling kept as an alias.
    assert.equal(registry.fieldByTag(35).fix.codeset, 'msgtypecodeset')
    assert.deepEqual(registry.codeset('msgtypecodeset').codes, [
      {
        value: 'P Report Ack',
        name: 'AllocationReportAck',
        aliases: ['P Report Ack'],
        doc: 'Allocation Report ACK',
      },
    ])
    assert.equal(registry.codeValue('msgtypecodeset', 'AllocationReportAck'), 'P Report Ack')
    assert.equal(registry.codeName('msgtypecodeset', 'P Report Ack'), 'AllocationReportAck')
    assert.equal(value.asField().fix.msgtype, 'P Report Ack')
    // The registered definition is a component of the registry, reached
    // through the field doors like every definition.
    assert.equal(registry.fieldByName('allocationreportack').fix.msgtype, 'P Report Ack')
    assert.equal(registry.size, SEEDED + 2)
    // A message type is a copy: the dictionary is not shared with it.
    registry.insert(fixField('Symbol', 'utf8', 55))
    assert.ok(registry.getMsgtype('P Report Ack').equals(value))
    const copy = registry.clone()
    assert.ok(copy.registerMsgtype('P Report Ack').equals(value))
  })

  test('a CBlock is read for what it says, and a truncated one is refused', () => {
    const root = scratch()

    // A declaration this reader cannot make a field of is dropped and the rest
    // of the file is still a dictionary: what went is a warning the host reads
    // through a logger, never a failed read.
    const dropped = path.join(root, 'dropped.cfb')
    fs.writeFileSync(
      dropped,
      [
        '<?xml version="1.0"?>',
        '<cplugin-configuration fix-version="4.4">',
        '<vocabulary>',
        '<vocabulary-tag name="35" alt="MsgType" type="decimal" />',
        '<vocabulary-tag name="55" alt="Symbol" type="string" />',
        '</vocabulary>',
        '</cplugin-configuration>',
      ].join('\n'),
      'utf8',
    )
    const [registry] = fix.FixRegistry.fromCfbFile(dropped, 'bloomberg')
    assert.equal(registry.fieldByTag(55).name, 'symbol')
    assert.throws(() => registry.fieldByTag(35))

    // A document that stops with an element open leaves nothing to keep, and
    // the native sentence crosses whole rather than as a bare "invalid file":
    // the byte the reader stopped at, and the element it left open.
    const truncated = path.join(root, 'broken.cfb')
    fs.writeFileSync(
      truncated,
      [
        '<?xml version="1.0"?>',
        '<cplugin-configuration fix-version="4.4">',
        '<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" />',
        '</cplugin-configuration>',
      ].join('\n'),
      'utf8',
    )
    assert.throws(
      () => fix.FixRegistry.fromCfbFile(truncated, 'bloomberg'),
      (error) => {
        assert.match(error.message, /invalid cfb expression at byte \d+/)
        assert.match(error.message, /vocabulary/)
        return true
      },
    )
  })

  // One content record, with tag 0 reserved for what the dictionary did not
  // resolve. Parity with `rust/tests/fix/entry.rs`.

  test('unresolved counters keep their members in arrival order under tag 0', () => {
    const codec = fixedCodec(seed())
    // At the top: an unregistered numeric counter heads what arrived under it.
    const top = codec.parsePairs([
      ['999999', '2'],
      ['999999[0].OwnThing', 'a'],
      ['999999[1].999998', 'b'],
    ])
    assert.deepEqual(flat(top), [
      [0, '999999', null],
      [0, '999999', null],
      [0, '999998', 'b'],
      [0, '999999', null],
      [0, 'ownthing', 'a'],
    ])
    assert.equal(top.intoText('|'), '8=FIX.4.4|999998=b|ownthing=a|')

    // A declared group keeps its resolved member and the unknown children
    // beside it under the counter it stated.
    const registry = new fix.FixRegistry()
    registry.insert(fixField('norows', 'int32', 90_001))
    const group = fields.serie('rows', fields.struct('row', [fixField('scopedvalue', 'utf8', 90_002)], { nullable: false }))
    group.fix.counter = 90_001
    assert.equal(registry.insert(group), null)
    assert.equal(registry.getFieldByTag(90_002), null)
    assert.equal(registry.fieldByCounter(90_001).name, 'rows')
    const rows = fixedCodec(registry).parsePairs([
      ['NoRows', '1'],
      ['Rows[0].ScopedValue', 'known'],
      ['Rows[0].999999', 'numeric'],
      ['Rows[0].OwnThing', 'named'],
    ])
    assert.deepEqual(flat(rows), [
      [90_001, 'rows', '1'],
      [0, 'row', null],
      [90_002, 'scopedvalue', 'known'],
      [0, '999999', 'numeric'],
      [0, 'ownthing', 'named'],
    ])
    assert.equal(rows.byPath('rows[0].scopedvalue').asJs(), 'known')
    assert.equal(rows.byPath('rows[0]."999999"').asJs(), 'numeric')
    assert.equal(rows.byPath('rows[0].ownthing').asJs(), 'named')
    assert.equal(rows.intoText('|'), '8=FIX.4.4|90001=1|90002=known|999999=numeric|ownthing=named|')
  })
}

// The `batch` suite, in its own block: it brings its own
// fixtures, and `const` is block-scoped.
{
  // The codec's streams and Arrow twins, and the message holder's setters.
  //
  // Every rule here is the core's, pinned in `rust/tests/fix/batch.rs` and
  // `rust/tests/fix/msg.rs`; what these check is the crossing - that a
  // JavaScript iterable is pulled one item at a time, that batch readers cross
  // both ways, that raw-byte batching is observable through `batchByteSize`, and
  // that a write to a message is typed by the dictionary and leaves the wire
  // alone.

  const assert = require('node:assert/strict')
  const path = require('node:path')
  const test = require('node:test')

  const arrow = require('apache-arrow')

  const { BatchReader, DataType, Field, Scalar, TextLine, TextOptions, fields, fix } = require('yggdryl')

  const SEED = path.join(__dirname, '..', '..', 'config', 'fix')


  // A codec that reads every message type. The corpora below are captures, and
  // a capture holds the session traffic and the bridge rows stating no type
  // that `DEFAULT_REFUSED_MSGTYPES` drop; a case about the refusals says so for
  // itself.
  function reading(registry, options) {
    return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
  }

  let seedRegistry
  function seed() {
    seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
    return seedRegistry.clone()
  }

  const encoder = new TextEncoder()
  const PIPE = '|'.charCodeAt(0)
  // The one intake clock the Rust suites build undated messages under
  // (`fixed_codec` in `rust/tests/fix.rs`), stated here as a message's own
  // SendingTime so two builds settle the same identity.
  const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)
  // The columns a row must carry a value at: the settled identity, and the
  // version every message opens with.
  const REQUIRED = ['currunix', 'creaunix', 'curruuid', 'crossuuid', 'currhashcode', 'crosshashcode', 'beginstring']

  // Two frames on one row: a line is none, one or many messages, and this
  // one is two.
  const TWO_FRAMES = '8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|'

  // Every shape a real capture holds, the corpus the readers are tested on.
  const CAPTURE = [
    'sending >> 8=FIX.4.2|9=176|35=D|11=ORDER-1|55=AAPL|54=1|10=203| << queued seq=1092',
    'raw 8=FIX.4.4|9=224|35=8|17=E1|37=O9|31=12.75|32=50|10=118|',
    '8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=1|',
    'sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|',
    '8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000',
    'toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1',
    'ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1',
    'After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR',
    'Referential(dbi|equity|dbi;GB00BN7SWP63_XLON_GBX|[quantity-type=])',
    "<Order ClOrdID='XML-1'>body</Order>",
    "Receiving XmlApi: <Execution ExecID='E1'></Execution>",
    'Message rejected because : ignoring OMSSales expiry message',
    'no level printed by this logger',
    'heartbeat emitted seq=7',
  ]

  // The capture lines that carry a message. A line that opens no frame, states
  // no bridge pair and carries no document carries nothing to read
  //: `After Enrichment ->` and `heartbeat emitted seq=7` write
  // their pairs into a sentence, which names no separator for them, so they are
  // prose that happens to hold an `=`, and the other three hold no pair at all.
  const CARRYING = [0, 1, 2, 3, 4, 5, 6, 9, 10].map((at) => CAPTURE[at])

  const ORDER = '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|VenueThing=7|9999=x|10=0|'
  const REPORT = '8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|'

  // The capture as the batches a text reader hands the codec, `rows` lines to a batch.
  function capture(lines, rows) {
    const batches = []
    for (let at = 0; at < lines.length; at += Math.max(rows, 1)) {
      const chunk = lines.slice(at, at + Math.max(rows, 1)).map((line) => encoder.encode(line))
      batches.push(new arrow.Table({ body: arrow.vectorFromArray(chunk, new arrow.Binary()) }))
    }
    if (batches.length === 0) {
      return BatchReader.from(new arrow.Table({ body: arrow.vectorFromArray([], new arrow.Binary()) }))
    }
    return BatchReader.from(batches.flatMap((table) => table.batches))
  }

  // Two hundred wide orders, about 450 bytes each.
  function wide() {
    return Array.from(
      { length: 200 },
      (_, index) => `8=FIX.4.4|35=D|11=ORDER-${String(index).padStart(6, '0')}|58=${'x'.repeat(400)}|10=0|`,
    )
  }

  function one(codec, line) {
    const messages = codec.parseLine(Buffer.from(line))
    const message = messages.next().value
    assert.equal(messages.next().done, true, 'one message')
    return message
  }

  function column(table, name) {
    return table.getChild(name).toJSON()
  }

  /** One exact column, as the unscaled coefficients Arrow JS renders it. */
  function exactColumn(table, name) {
    return column(table, name).map((held) => (held === null ? null : BigInt(JSON.parse(held))))
  }

  function rowCounts(reader) {
    return [...reader].map((batch) => batch.numRows)
  }

  // Every tag a message's children carry, beside the value each holds.
  function stated(message) {
    const held = new Map()
    for (let at = 0; at < message.field.fieldLen; at += 1) {
      const tag = message.field.fieldAt(at).fix.tag
      if (tag !== null) held.set(tag, message.byTag(tag))
    }
    return held
  }

  test('parseLines pulls one line at a time and continues past a refused one', () => {
    const codec = reading(seed(), { threads: 1 })
    let pulled = 0
    function* lines() {
      for (const line of [TWO_FRAMES, '']) {
        pulled += 1
        yield line
      }
    }
    const messages = codec.parseLines(lines())
    assert.ok(messages instanceof fix.FixMessages)
    assert.equal(messages[Symbol.iterator](), messages)
    // Nothing is pulled until the stream is asked.
    assert.equal(pulled, 0)
    assert.equal(messages.next().value.byTag(11).asJs(), 'A')
    assert.equal(pulled, 1)
    // The second frame comes out of the same line, without the next pull.
    assert.equal(messages.next().value.byTag(11).asJs(), 'B')
    assert.equal(pulled, 1)
    // An empty line is not a row at all: thrown where it is met, and the
    // stream goes on to say it is done.
    assert.throws(() => messages.next(), /captured row/)
    assert.equal(pulled, 2)
    assert.equal(messages.next().done, true)
    assert.equal(messages.next().done, true)
  })

  test('an item that is not bytes is refused where it is met', () => {
    const codec = reading(seed())
    const mixed = codec.parseLines([Buffer.from('8=FIX.4.4|35=D|11=A|10=0|'), 42])
    assert.equal(mixed.next().value.byTag(11).asJs(), 'A')
    assert.throws(() => mixed.next(), TypeError)
    assert.equal(mixed.next().done, true)
    // A generator that throws throws as itself.
    function* failing() {
      yield '8=FIX.4.4|35=D|11=A|10=0|'
      throw new RangeError('the source broke')
    }
    const broken = codec.parseLines(failing())
    assert.equal(broken.next().value.byTag(11).asJs(), 'A')
    assert.throws(() => broken.next(), RangeError)
    // Something that is not iterable is refused before anything is pulled.
    assert.throws(() => codec.parseLines(42), TypeError)
  })

  test('parseTextLines pulls one line at a time', () => {
    const codec = reading(seed(), { captureNames: ['beginstring'], threads: 1 })
    let pulled = 0
    function* lines() {
      for (const body of ['8=FIX.4.4|35=D|11=A|10=0|', '8=FIX.4.4|35=D|11=B|10=0|']) {
        pulled += 1
        yield new TextLine(BigInt(pulled - 1), Buffer.from(body))
      }
    }
    const messages = codec.parseTextLines(lines())
    assert.equal(pulled, 0)
    assert.equal(messages.next().value.byTag(11).asJs(), 'A')
    assert.equal(pulled, 1)
    assert.equal(messages.next().value.byTag(11).asJs(), 'B')
    assert.equal(messages.next().done, true)
    // A capture speaks per row: `beginstring` reads the frame at 4.2, where tag
    // 32 is `lastshares`, and what it restates to is the quantity the event
    // last traded rather than a column beside it.
    const [old] = codec.parseTextLines([
      new TextLine(0n, Buffer.from('8=FIX.4.4|35=8|32=100|10=0|'), ['FIX.4.2']),
    ])
    assert.equal(old.field.indexOf('lastqty'), null, 'the event holds it')
    assert.equal(old.lastqty, '100')
    assert.notEqual(old.getByName('lastshares'), null)
    // A row of two frames is two messages, and the stream door yields each.
    assert.equal([...codec.parseTextLines([new TextLine(0n, Buffer.from(TWO_FRAMES))])].length, 2)
  })

  test("a row's msgpluginid fills its own column and selects nothing", () => {
    // A venue field beside the specification's: one namespace, so `VENUETAG`
    // resolves whatever the row's plugin is called, and the membership the
    // field carries is provenance a caller filters on.
    const registry = seed()
    for (const [name, tag, dialect] of [['VenueTag', 5001, 'venue'], ['OtherTag', 5002, 'elsewhere']]) {
      const field = Field.from(`${name}: utf8`)
      field.fix.tag = tag
      field.fix.branches = [dialect]
      registry.insert(field)
    }
    assert.deepEqual(registry.dialects(), ['elsewhere', 'venue'])
    const captureNames = ['msgpluginid', 'prevmsgpluginid']
    const codec = reading(registry, { captureNames })
    const body = Buffer.from('MSGTYPE=D|CLORDID=A|VENUETAG=dark')
    // A line and the captures its header declared, in that order.
    const lined = (plugin, previous = null, held = body) => new TextLine(0n, held, [plugin, previous])

    // A `msgpluginid` capture - a plugin named like a dictionary, one no
    // dictionary is named after, a null, an empty string - fills the crate's
    // own field exactly as it was spelled and selects no dialect: the venue's
    // field resolves under every one of them.
    for (const spelled of ['venue', 'VENUE', 'OMS_X1_TradeCapture', null, '']) {
      const [message] = codec.parseTextLine(lined(spelled))
      assert.equal(message.byTag(5001).asJs(), 'dark', `${spelled}`)
      assert.equal(message.byName('venuetag').asJs(), 'dark', `${spelled}`)
      // A capture is the message's own, typed: an empty spelling states
      // nothing, as an absent one does.
      assert.equal(message.capture().msgpluginid, spelled === '' ? null : spelled, `${spelled}`)
      // A message root is not a dictionary member.
      assert.deepEqual(message.field.fix.branches, [])
    }

    // Nothing fills `prevmsgpluginid` but a capture of that name, and the two
    // session names are only ever what the line itself spells, through the
    // aliases a bridge row writes them under.
    const [carried] = codec.parseTextLine(lined('venue', 'ULFilter'))
    assert.equal(carried.capture().msgpluginid, 'venue')
    const spoken = '|#SYMBOL=TTF|#TECH.CLIENTID=MCFP2|'
    const [stated] = codec.parseTextLine(lined('venue', null, Buffer.from(spoken)))
    // A bridge's own namespaced key is the message's metadata, folded once,
    // and the event states the same map under the market's name for it.
    assert.deepEqual(stated.metadata, { 'tech.clientid': 'MCFP2' })
    assert.deepEqual(stated.event().metadata, { 'tech.clientid': 'MCFP2' })
    assert.equal(stated.capture().msgsessionid, null)
  })

  test('the codec answers the pins it was given', () => {
    const registry = seed()
    const bare = reading(registry)
    // A codec pins no version: a row states one, or the line implies it.
    assert.equal(bare.version, undefined)
    assert.equal(bare.separator, null)
    assert.equal(bare.payloadColumn, 'body')
    assert.deepEqual(bare.nullValues, ['', 'null', '<null>', 'none', 'n/a', '[n/a]'])
    assert.equal(bare.direction, 'S')
    // The default target, stated once in the core and read here.
    assert.equal(bare.batchByteSize, 128 * 1024 * 1024)

    const pinned = reading(registry, {
      separator: PIPE,
      payloadColumn: 'line',
      nullValues: ['<none>'],
      direction: 'Receive',
      batchByteSize: 4096,
    })
    // No pin names a dialect: the dictionary is one namespace.
    assert.equal('branch' in pinned, false)
    assert.equal(pinned.separator, PIPE)
    assert.equal(pinned.payloadColumn, 'line')
    assert.deepEqual(pinned.nullValues, ['<none>'])
    assert.equal(pinned.direction, 'R')
    assert.equal(pinned.batchByteSize, 4096)
    // The empty text is no pin; a spelling outside tag 385's set is refused
    // naming the set.
    assert.equal(reading(registry, { direction: '' }).direction, null)
    assert.throws(() => reading(registry, { direction: 'sideways' }), /R, S/)
    assert.throws(() => reading(registry, { batchByteSize: 1.5 }))
    // The payload column names a batch column; a line's body is its own, so
    // the line door reads the same frame without naming anything.
    const [read] = pinned.parseTextLines([new TextLine(0n, Buffer.from('8=FIX.4.2|35=D|11=A|10=0|'))])
    assert.equal(read.byTag(11).asJs(), 'A')
  })

  // A row header dating each line by an `mtime` capture, the line's own clock.
  const DATED = String.raw`^(?<mtime>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) `
  const RECORDED = '2024-03-05 10:15:30.250'
  const RECORDED_NS = 1_709_633_730_250_000_000n
  const SENDING_NS = 1_704_190_530_000_000_000n

  /** `body` as a line the header dated at `RECORDED`, in UTC. */
  function datedLine(body) {
    const options = new TextOptions()
    options.rowheader = DATED
    options.timezone = 'UTC'
    return new TextLine(0n, body, [RECORDED], options)
  }

  test("a line's own clock dates a message stating no sending time", () => {
    // The line was recorded as its message went by: nearer the send than
    // any pin.
    const registry = seed()
    const schema = fix.schema(registry)
    const line = datedLine('8=FIX.4.4|35=8|10=0|')
    assert.equal(line.mtime, RECORDED_NS)
    assert.equal(line.currunix, RECORDED_NS)

    // Unpinned and pinned alike, the line's clock is the sending clock an
    // undated frame on it is read against - ahead of the default and of now -
    // and so the instant, the creation and the recording.
    for (const codec of [reading(registry), reading(registry, { defaultSendingTime: SENDING })]) {
      const [message] = codec.parseTextLine(line)
      assert.equal(message.header().sendingtime, RECORDED_NS)
      assert.equal(message.currunix, RECORDED_NS)
      assert.equal(message.event().creaunix, RECORDED_NS)
      assert.equal(message.event().recdunix, RECORDED_NS)
      // Supplied, never stated: neither the wire nor the row's own column
      // says what the frame did not.
      assert.ok(!message.intoText('|').includes('52='))
      assert.equal(message.intoRow(schema).asJs()[schema.indexOf('sendingtime')], null)
    }

    // A frame stating its own SendingTime keeps it; the line still says when
    // it was recorded.
    const [stated] = reading(registry).parseTextLine(datedLine('8=FIX.4.4|35=8|52=20240102-10:15:30|10=0|'))
    assert.equal(stated.header().sendingtime, SENDING_NS)
    assert.equal(stated.currunix, SENDING_NS)
    assert.equal(stated.event().recdunix, RECORDED_NS)
    assert.ok(stated.intoText('|').includes('|52=20240102-10:15:30|'))

    // An execution report on it executed at that instant.
    const [filled] = reading(registry, { defaultSendingTime: SENDING })
      .parseTextLine(datedLine('8=FIX.4.4|35=8|150=F|39=2|10=0|'))
    assert.equal(filled.event().execunix, RECORDED_NS)

    // The Arrow door reads the same clock off a row's `currunix` cell.
    const source = new arrow.Table({
      currunix: arrow.makeVector(arrow.makeData({
        type: new arrow.TimestampNanosecond('UTC'),
        length: 1,
        data: BigInt64Array.from([RECORDED_NS]),
      })),
      body: arrow.vectorFromArray([encoder.encode('8=FIX.4.4|35=8|10=0|')], new arrow.Binary()),
    })
    const parsed = reading(registry, { defaultSendingTime: SENDING })
      .parseTextArrowReader(BatchReader.from(source))
      .intoTable()
    assert.deepEqual([...parsed.getChild('currunix').toArray()], [RECORDED_NS])
    assert.deepEqual([...parsed.getChild('recdunix').toArray()], [RECORDED_NS])
    assert.deepEqual(column(parsed, 'sendingtime'), [null])

    // A raw-byte door holds no line, so the same frame there takes the pin.
    const raw = reading(registry, { defaultSendingTime: SENDING }).parseFixLine(Buffer.from('8=FIX.4.4|35=8|10=0|'))
    assert.equal(raw.currunix, SENDING_NS)
    assert.equal(raw.event().recdunix, null)
  })

  test("threads preserve ordered multi-batch rows and a message carries its row's cells", () => {
    const defaultCodec = reading(seed())
    const one = reading(seed(), { threads: 1, batchRowSize: 1 })
    const four = reading(seed(), { threads: 4, batchRowSize: 1 })
    assert.ok(defaultCodec.threads >= 1)
    assert.equal(one.threads, 1)
    assert.equal(four.threads, 4)
    assert.equal(reading(seed(), { threads: 0 }).threads, 1)
    // The line doors answer on four threads what an explicit one answers:
    // the same messages in the same order. One physical line holds two
    // frames and each source batch holds one line, so the pool receives more
    // batches than its four workers and the output closes one row at a time.
    // By content code and wire: the lines state no clock, so each parse dates
    // them by its own now, and the identity the instant derives differs.
    const stated = (messages) => [...messages].map((held) => [held.currhashcode, held.intoText('|')])
    const lines = [TWO_FRAMES, ...CAPTURE]
    const bytes = lines.map((line) => Buffer.from(line))
    assert.deepEqual(stated(four.parseLines(bytes)), stated(one.parseLines(bytes)))
    const source = () => capture(lines, 1)
    const parsed = four.parseTextArrowReader(source()).intoTable()
    assert.deepEqual(
      rowCounts(four.parseTextArrowReader(source())),
      Array(CARRYING.length + 2).fill(1),
    )
    assert.deepEqual(
      stated(four.messages(parsed)),
      stated(one.messages(one.parseTextArrowReader(source()).intoTable())),
    )
    // A message read back out of a row carries the row's own cells - the
    // body its line was cut from - and one parsed from bytes carries none.
    const [held] = one.messages(parsed)
    assert.deepEqual(Object.keys(held.carried), ['body'])
    assert.equal(Buffer.from(held.carried.body.asJs()).toString(), TWO_FRAMES)
    assert.deepEqual(one.parseLine(bytes[0]).next().value.carried, {})
  })

  test('the schema is decided before the first row is read', () => {
    const reader = reading(seed()).parseTextArrowReader(capture([], 1))
    const names = []
    for (let at = 0; at < reader.field.fieldLen; at += 1) names.push(reader.field.fieldAt(at).name)
    // The capture's own column leads; the fixed columns follow, opening on
    // when the event happened, each named by its folded name and carrying
    // its tag.
    assert.equal(names[0], 'body')
    assert.equal(names[1], 'currunix')
    const header = names.indexOf('beginstring')
    assert.ok(header > 0)
    assert.equal(reader.field.fieldAt(header).fix.tag, 8)
    // One list closes the row: the whole content record, unresolved keys at
    // tag 0, under the counter that counts it.
    assert.deepEqual(names.slice(-2), ['nofixentries', 'fixentries'])
    assert.equal(reader.field.fieldAt(names.indexOf('msgtype')).fix.tag, 35)
    // And an empty capture yields no batch at all.
    assert.equal(reader.intoTable().numRows, 0)
  })

  test('a capture answers one row per message, not one per line', () => {
    const parsed = reading(seed()).parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoTable()
    assert.equal(parsed.numRows, CARRYING.length, 'one row a message; the text reader answers one a line')
    assert.deepEqual(
      column(parsed, 'body').map((body) => Buffer.from(body).toString()),
      CARRYING,
    )
    const msgtype = column(parsed, 'msgtype')
    assert.equal(msgtype[0], 'D', 'a framed row states its type')
    assert.equal(msgtype.at(-1), null, 'a document that states no type is `unknown`')
  })

  test('several small input batches accumulate and one large batch splits by rows', () => {
    const registry = seed()
    const lines = wide()
    // Twenty input batches of ten rows, far under the default target.
    const whole = rowCounts(reading(registry).parseTextArrowReader(capture(lines, 10)))
    assert.deepEqual(whole, [200], 'one batch under the byte target')

    // Under a target holding about five input batches, the output batches are
    // fewer than the input ones and no row is lost.
    const bounded = rowCounts(
      reading(registry, { batchByteSize: 5 * 10 * 470 }).parseTextArrowReader(capture(lines, 10)),
    )
    assert.ok(bounded.length >= 2 && bounded.length < 20, `${bounded.length} batches`)
    assert.equal(bounded.reduce((sum, rows) => sum + rows, 0), 200)
    for (const rows of bounded.slice(0, -1)) assert.ok(rows > 10, 'an input batch did not close an output batch alone')

    // One input batch charges every row the same share of its bytes, so the
    // cut is even, and that many rows of raw capture is about the target.
    const target = 4096
    const split = rowCounts(
      reading(registry, { batchByteSize: target }).parseTextArrowReader(capture(lines, lines.length)),
    )
    assert.ok(split.length > 1)
    assert.equal(split.reduce((sum, rows) => sum + rows, 0), 200, 'the bound shapes batches, it does not drop rows')
    // The charge is what each row lands as - the leaves of every column and
    // a per-row width - so the cut is even and the count is the target's,
    // not the raw line's.
    const closed = split.slice(0, -1)
    assert.ok(closed.every((rows) => rows === closed[0]))
    assert.ok(closed[0] >= 1)

    // A target no row fits under closes a batch after every row, so one
    // enormous line can never produce an empty batch.
    const each = rowCounts(reading(registry, { batchByteSize: 1 }).parseTextArrowReader(capture(lines, lines.length)))
    assert.equal(each.length, 200)
    assert.ok(each.every((rows) => rows === 1))
  })

  test('messages to batches close on the arrival records raw bytes', () => {
    const registry = seed()
    const codec = reading(registry)
    const schema = fix.schema(registry)
    const lines = wide()
    assert.deepEqual(rowCounts(codec.arrowReader(schema, codec.parseLines(lines))), [200])

    // A bound of about ten lines of pairs cuts the stream into batches of
    // about ten, and every row survives the cut.
    const many = rowCounts(reading(registry, { batchByteSize: 10 * 450 }).arrowReader(schema, codec.parseLines(lines)))
    assert.ok(many.length > 1 && many.length < 40, `${many.length} batches`)
    assert.equal(many.reduce((sum, rows) => sum + rows, 0), 200)
    assert.ok(many.slice(0, -1).every((rows) => rows > 0))

    // A target of one byte is a batch a message.
    assert.equal(rowCounts(reading(registry, { batchByteSize: 1 }).arrowReader(schema, codec.parseLines(lines))).length, 200)
  })

  test('bookArrowReader streams messages through native books into nested batches', () => {
    const codec = reading(seed(), { batchRowSize: 1 })
    const snapshot = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
    ))
    const update = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|',
    ))
    const reader = codec.bookArrowReader([snapshot, update])
    const names = Array.from({ length: reader.field.fieldLen }, (_, at) => reader.field.fieldAt(at).name)
    // The book row: the event and market columns, the two sides, the
    // executions, and the partitions its last snapshot replaced.
    assert.deepEqual(names.slice(-4), ['bid', 'ask', 'executions', 'snapshotpartitions'])
    assert.ok(names.includes('price') && names.includes('quantity'))
    assert.ok(!names.includes('px') && !names.includes('qty'))
    const books = reader.intoTable()
    assert.equal(books.numRows, 2)
    assert.deepEqual(exactColumn(books, 'price'), [101n * 10n ** 18n, 1015n * 10n ** 17n])
  })

  test('a lifecycled two-sided trade streams executions without book depth', () => {
    const codec = reading(seed(), { batchRowSize: 1 })
    const trade = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=AE|49=SELL|56=BUY|34=7|52=20260921-10:00:00|' +
      '571=T1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|' +
      '54=1|1427=BUY-EXEC|1009=4|37=BUY-ORDER|11=BUY-CLIENT|' +
      '54=2|1427=SELL-EXEC|1009=6|37=SELL-ORDER|11=SELL-CLIENT|10=0|',
    ))
    const books = codec.bookArrowReader(codec.lifecycle([trade])).intoTable()
    const executions = books.getChild('executions').get(0)
    const bid = books.getChild('bid').get(0)
    const ask = books.getChild('ask').get(0)
    const bySide = new Map(Array.from(executions, (execution) => [execution.side, execution]))
    const buy = bySide.get('BUY')
    const sell = bySide.get('SELL')

    assert.equal(books.numRows, 1)
    assert.deepEqual([...bySide.keys()].sort(), ['BUY', 'SELL'])
    assert.equal(buy.marketoperationid, 21)
    assert.equal(sell.marketoperationid, 21)
    // A trade-capture side states no price and no quantity: its last
    // executed price and quantity are `lastpx` and `lastqty`, and the two
    // nullable columns carry the null.
    assert.equal(buy.price, null)
    assert.equal(sell.price, null)
    assert.equal(buy.quantity, null)
    assert.equal(sell.quantity, null)
    assert.equal(BigInt(buy.lastpx.toString()), 10125n * 10n ** 16n)
    assert.equal(BigInt(sell.lastpx.toString()), 10125n * 10n ** 16n)
    assert.equal(BigInt(buy.lastqty.toString()), 4n * 10n ** 18n)
    assert.equal(BigInt(sell.lastqty.toString()), 6n * 10n ** 18n)
    assert.equal(new Map(buy.altids).get('SIDEEXECID'), 'BUY-EXEC')
    assert.equal(new Map(sell.altids).get('SIDEEXECID'), 'SELL-EXEC')
    assert.notDeepEqual(buy.curruuid, sell.curruuid)
    assert.notDeepEqual(buy.crossuuid, sell.crossuuid)
    assert.notEqual(buy.crosscode, sell.crosscode)
    assert.equal(bid.live.length, 0)
    assert.equal(ask.live.length, 0)
    assert.equal(bid.deltas.length, 0)
    assert.equal(ask.deltas.length, 0)
  })

  test('a trade side without Side refuses at its exact occurrence through the book reader', () => {
    const codec = reading(seed(), { batchRowSize: 1 })
    const missing = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|' +
      '32=4|31=101.25|60=20260921-10:00:00|552=1|' +
      '1427=NO-SIDE|1009=4|37=ORDER-1|11=CLIENT-1|10=0|',
    ))

    assert.throws(
      () => codec.bookArrowReader([missing]).intoTable(),
      /\$\.NoSides\(552\)\[0\]\.Side\(54\)/,
    )
  })

  test('a parse fills what the dictionary derives, through both doors', () => {
    const codec = reading(seed())
    const rows = codec.parseTextArrowReader(capture([REPORT], 1)).intoTable()
    const [message] = codec.parseLines([REPORT])
    // There is no enriching pass: what a message implies is filled where it
    // is parsed, so the row a capture lands in states it already.
    // An exact column crosses as its unscaled coefficient at the one scale
    // this crate keeps a number at.
    assert.deepEqual(exactColumn(rows, 'leavesqty'), [60n * 10n ** 18n])
    assert.ok(message.byTag(151).equals(Scalar.decimal(60n)))
    assert.deepEqual(exactColumn(rows, 'avgpx'), [105n * 10n ** 17n])
    assert.ok(message.byTag(6).equals(Scalar.decimal(105n, 1)))
    // A derived tag the fixed row does not carry is filled on the message and
    // has no column to appear in: the row is a projection of the message.
    assert.ok(message.byTag(381).equals(Scalar.decimal(420n)))
    assert.equal(rows.schema.fields.some((field) => field.name === 'grosstradeamt'), false)
    // The carried column still leads the row.
    assert.equal(rows.schema.fields[0].name, 'body')
  })

  test('messages and arrowReader invert each other', () => {
    const registry = seed()
    const codec = reading(registry, { nullValues: [] })
    const schema = fix.schema(registry)
    const parsed = [...codec.parseLines(CAPTURE)]
    const again = [...codec.messages(codec.arrowReader(schema, parsed))]
    assert.equal(again.length, parsed.length)
    for (const [at, held] of again.entries()) {
      const message = parsed[at]
      // Projected columns and residual entries may rebuild in another child
      // order; the semantic values, supplied identity and fixed row stay exact.
      assert.equal(held.currhashcode, message.currhashcode)
      assert.equal(held.curruuid, message.curruuid)
      for (const tag of [35, 11, 55, 54, 17, 37, 65017, 65039]) {
        const value = message.getByTag(tag)
        if (value !== null && value.kind !== 'null') assert.ok(held.byTag(tag).equals(value), `tag ${tag}`)
      }
      assert.ok(held.intoRow(schema).equals(message.intoRow(schema)))
    }
    // And the batches the second pass makes are the batches the first made.
    const first = codec.arrowReader(schema, parsed).intoIpc()
    const second = codec.arrowReader(schema, again).intoIpc()
    assert.deepEqual(first, second)
  })

  test('the names a message goes by are built from their source fields and have no column', () => {
    const registry = seed()
    const codec = reading(registry, { defaultSendingTime: SENDING })
    const schema = fix.schema(registry)
    // A message states the names it goes by as a typed fact: the map the
    // event holds, read back as a plain object with upper-cased keys in the
    // core's key order, built at every settle from `OrderID(37)`,
    // `ClOrdID(11)`, `ExecID(17)` and the other source fields.
    const lines = [
      '8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|',
      '8=FIX.4.4|35=D|10=0|',
    ]
    const messages = lines.map((line) => one(codec, line))
    assert.deepEqual(messages[0].altids, { CLORDID: 'C-001', EXECID: 'E-09', ORDERID: 'O-01' })
    assert.deepEqual(Object.keys(messages[0].altids), ['CLORDID', 'EXECID', 'ORDERID'])
    assert.deepEqual(messages[1].altids, {})
    assert.deepEqual(messages[0].accountids, {})
    assert.deepEqual(messages[0].userids, {})

    // No column of its own: the row states the source fields, and a row
    // read back builds the same map from them.
    const table = codec.arrowReader(schema, messages).intoTable()
    for (const gone of ['identifiers', 'altids', 'accountids', 'userids', 'securityids']) {
      assert.equal(table.schema.fields.some((field) => field.name === gone), false, gone)
    }
    const restored = [...codec.messages(table)]
    assert.equal(restored.length, messages.length)
    for (const [at, held] of restored.entries()) {
      assert.deepEqual(held.altids, messages[at].altids, `message ${at}`)
      assert.ok(held.intoRow(schema).equals(messages[at].intoRow(schema)), `message ${at}`)
    }

    // The bridge's own namespaced keys cross as `metadata`, which does have
    // a column.
    const bridged = one(codec, 'MSGTYPE=D|CLORDID=A|TECH.CLIENTID=X1|')
    assert.deepEqual(bridged.metadata, { 'tech.clientid': 'X1' })
    const held = [...codec.messages(codec.arrowReader(schema, [bridged]).intoTable())]
    assert.deepEqual(held[0].metadata, bridged.metadata)
  })

  test('a scalar alias never takes the metadata Map name', () => {
    const scalar = fields.utf8('venueid')
    scalar.fix.tag = 9001
    scalar.fix.names = ['Metadata']
    const registry = fix.FixRegistry.fromFields([scalar])
    // The scalar answers the folded name a lookup asks for; the group is
    // reached by its counter and by the path grammar.
    assert.equal(registry.fieldByName('metadata').name, 'venueid')
    assert.equal(registry.fieldByCounter(65049).name, 'metadata')
    assert.equal(registry.fieldByPath('metadata').name, 'metadata')
    // A message carries the scalar in its row and the map as its own fact.
    const schema = fields.struct('row', [scalar, registry.fieldByTag(52)], { nullable: false })
    const value = new fix.FixMsg(schema, { venueid: 'scalar', sendingtime: SENDING }, registry)
    assert.equal(value.byTag(9001).asJs(), 'scalar')
    assert.deepEqual(value.metadata, {})
    value.set(65049, new Map([['tech.clientid', 'X1']]))
    assert.deepEqual(value.metadata, { 'tech.clientid': 'X1' })
    assert.equal(value.byTag(9001).asJs(), 'scalar')
    assert.equal(value.size, 1, 'the map is a fact, never a row child')
  })

  test('messages pull from the reader one batch at a time', () => {
    const codec = reading(seed(), { threads: 1 })
    const source = codec.parseTextArrowReader(capture(CAPTURE, 3))
    const messages = codec.messages(source)
    assert.ok(messages instanceof fix.FixMessages)
    assert.ok(source.consumed, 'the stream is taken, not copied')
    const first = messages.next().value
    assert.equal(first.byTag(11).asJs(), 'ORDER-1')
    assert.equal(first.getByName('body'), null, "a carried column stays the row's, never a child of the message")
    assert.equal([...messages].length, CARRYING.length - 1)
  })

  test('a failure behind a batch stream arrives with the batch', () => {
    const registry = seed()
    const codec = reading(registry)
    const message = one(codec, ORDER)
    const reader = codec.arrowReader(fix.schema(registry), [message, 'not a message'])
    // The batch reader pulls the messages, so the failure is that pull's, and
    // it names the item that was met.
    assert.throws(() => reader.intoTable(), /FixMsg/)
  })

  test('byte in, byte out over the whole corpus', () => {
    const registry = seed()
    // The convention that drops a stated absence is deliberately not
    // byte-preserving, so it is turned off to measure the reader rather than
    // the convention.
    const codec = reading(registry, { nullValues: [], separator: PIPE, defaultSendingTime: SENDING })
    const chunks = []
    const rows = codec.parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoIpc()
    const written = codec.writeArrowReader(BatchReader.fromIpc(rows), {
      write(chunk) {
        chunks.push(Buffer.from(chunk))
      },
    })
    // One line out per message, so the lines that carried none are not there.
    assert.equal(written, CARRYING.length)
    const back = Buffer.concat(chunks).toString().split('\n')
    assert.equal(back.pop(), '')
    assert.equal(back.length, CARRYING.length)
    // Each written line is what the row's own message emits: the wire is
    // rebuilt from the record, never from the columns.
    const messages = [...codec.messages(BatchReader.fromIpc(rows))]
    assert.equal(messages.length, CARRYING.length)
    for (const [at, line] of back.entries()) {
      assert.equal(line, messages[at].intoBytes(PIPE).toString(), CARRYING[at])
    }
    // An Arrow JS table is a source too, the sink is whatever writes chunks,
    // and a batch without the content record is refused before a row is read.
    const facets = codec.parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoTable().select(['symbol', 'side'])
    const refused = []
    assert.throws(() => codec.writeArrowReader(facets, { write: (chunk) => refused.push(chunk) }), /arrival record|content record|fixentries/)
    assert.deepEqual(refused, [])
    // A sink that throws throws as itself.
    assert.throws(
      () =>
        codec.writeArrowReader(codec.parseTextArrowReader(capture(CAPTURE, 1)), {
          write() {
            throw new RangeError('disk full')
          },
        }),
      /disk full/,
    )
    assert.throws(() => codec.writeArrowReader(capture(CAPTURE, 1), {}), /write\(chunk/)
  })

  test('a set value is typed by the registry field and appended when absent', () => {
    const registry = seed()
    const message = one(reading(registry), ORDER)
    const before = message.size
    const declared = registry.fieldByTag(1)

    message.set(1, 'ACC-1')

    assert.equal(message.size, before + 1, 'appended, not inserted')
    const child = message.field.fieldAt(before)
    assert.equal(child.name, declared.name, "the dictionary's spelling")
    assert.ok(child.dtype.equals(declared.dtype), "the dictionary's type")
    assert.equal(child.fix.tag, 1)
    assert.equal(child.nullable, false, 'a stated value is non-null')
    assert.equal(message.byTag(1).asJs(), 'ACC-1')
    assert.equal(message.byName('Account').asJs(), 'ACC-1', 'reached by name through the registry')

    // A header tag is a typed fact: it fills the holder and the row is
    // exactly as long as it was. So is `Price(44)`, which the message lifts
    // and holds exact, so it takes an exact value and not a float.
    const held = message.size
    message.set(34, 7)
    message.set(44, '10.5')
    assert.equal(message.size, held)
    assert.equal(message.header().msgseqnum, 7)
    assert.equal(message.byTag(34).asJs(), 7)
    assert.equal(message.price, '10.5')
    assert.ok(message.byTag(44).equals(Scalar.decimal(105n, 1)))
  })

  test('a set value replaces an existing child in place and keeps the tag index', () => {
    const message = one(reading(seed()), ORDER)
    const before = stated(message)
    const at = message.field.indexOf('symbol')

    message.set(55, 'MSFT')
    message.set('Side', '2')

    assert.equal(message.field.indexOf('symbol'), at, 'same position')
    assert.equal(message.size, before.size + 2, 'two unknown children beside the tagged ones')
    assert.equal(message.byTag(55).asJs(), 'MSFT')
    // A side is stored as the explicit value the wire code names.
    assert.equal(message.byTag(54).asJs(), 'SELL')
    // Content identity changes; every other tag still reaches its previous
    // value, the settled clocks and the chain identity included.
    for (const [tag, value] of before) {
      if (tag === 55 || tag === 54) continue
      if (tag === 65017 || tag === 65039) {
        assert.equal(message.byTag(tag).equals(value), false, 'the identity follows the content')
        continue
      }
      assert.ok(message.byTag(tag).equals(value), `tag ${tag}`)
    }
    assert.equal(message.byTag(65017).asJs(), message.currhashcode)
  })

  test('a set leaves the entries, the wire and the digest untouched', () => {
    const parsed = one(reading(seed()), ORDER)
    const message = parsed.clone()
    message.set(55, 'MSFT')
    message.set(1, 'ACC-1')
    // The written `Account(1)` is an entry and the removed `Side(54)` was
    // one, so the two cancel out: the side is an ordinary child now.
    assert.notEqual(message.remove(54), null)
    assert.deepEqual(message.entries().length, parsed.entries().length, 'the written child is an entry')
    // `OrderQty(38)` is a fact the message lifts, so writing it moves no
    // child and adds no entry.
    message.set(38, '100')
    assert.deepEqual(message.entries().length, parsed.entries().length)
    assert.equal(message.quantity, '100')
    assert.ok(message.byTag(38).equals(Scalar.decimal(100n)))
    // A null is stored as a stated null.
    message.set(55, null)
    assert.equal(message.field.fieldAt(message.field.indexOf('symbol')).nullable, true)
    assert.equal(message.getByTag(55).kind, 'null')
  })

  test('an unknown name is refused and the message stands', () => {
    const message = one(reading(seed()), ORDER)
    const before = message.clone()
    assert.throws(() => message.set('nosuchfield', 'y'), /nosuchfield/)
    assert.ok(message.equals(before))
    // A value the field refuses is refused the same way.
    assert.throws(() => message.set(34, 'not a number'))
    assert.ok(message.equals(before))
    // A key is a tag or a name.
    assert.throws(() => message.set(55n, 'y'), TypeError)
    // An unknown name still reaches the child spelled that way, which keeps
    // its own field.
    const at = message.field.indexOf('venuething')
    message.set('Venue_Thing', '8')
    assert.equal(message.field.fieldAt(at).name, 'venuething')
    assert.ok(message.field.fieldAt(at).dtype.equals(DataType.from('utf8')))
    assert.equal(message.byName('venuething').asJs(), '8')
  })

  test('a bare unknown tag is appended under its decimal spelling', () => {
    const message = one(reading(seed()), ORDER)
    message.set(7777, 'custom')
    const child = message.field.fieldAt(message.size - 1)
    assert.equal(child.name, '7777')
    assert.equal(child.nullable, true)
    assert.equal(message.byTag(7777).asJs(), 'custom')
    // A second write reaches the same child rather than a second one.
    message.set(7777, 'again')
    assert.equal(message.byTag(7777).asJs(), 'again')
    assert.equal(message.field.indexOf('7777'), message.size - 1)
    // The one the line already carried is replaced where it stands.
    message.set(9999, 'y')
    assert.equal(message.byTag(9999).asJs(), 'y')
  })

  test('remove answers the value and the other tags still reach their children', () => {
    const message = one(reading(seed()), ORDER)
    const before = stated(message)
    const count = message.size
    const wire = message.clone().intoBytes(PIPE).toString()

    assert.equal(message.remove(55).asJs(), 'AAPL')
    assert.equal(message.getByTag(55), null)
    assert.equal(message.size, count - 1)
    for (const [tag, value] of before) {
      if (tag === 55) continue
      if (tag === 65017 || tag === 65039) {
        assert.equal(message.byTag(tag).equals(value), false, 'the identity follows the content')
        continue
      }
      assert.ok(message.byTag(tag).equals(value), `tag ${tag}`)
    }
    // By name, by decimal, and a miss.
    assert.equal(message.remove('VenueThing').asJs(), '7')
    assert.equal(message.remove(9999).asJs(), 'x')
    assert.equal(message.remove(55), null)
    assert.equal(message.remove('nosuchfield'), null)
    assert.equal(message.size, count - 3)
    // The entries follow the row, so the wire no longer states what left it.
    const emitted = message.intoBytes(PIPE).toString()
    assert.ok(wire.includes('55=AAPL'))
    assert.equal(emitted.includes('55=AAPL'), false)
    assert.equal(emitted.includes('9999=x'), false)
  })

  test('a row is refused where it cannot state the settled identity', () => {
    const registry = seed()
    const codec = reading(registry, { defaultSendingTime: SENDING })
    const schema = fix.schema(registry)
    const parsed = one(codec, ORDER)
    const row = parsed.intoRow(schema)

    // Every required column is one the message settles, so a row nulling one
    // is refused at that column, located, and nothing is built.
    for (const name of REQUIRED) {
      const at = schema.indexOf(name)
      assert.notEqual(at, null, name)
      const cells = Array.from({ length: schema.fieldLen }, (_, index) => row.at(index))
      cells[at] = null
      assert.throws(() => fix.FixMsg.fromRow(schema, cells, registry), new RegExp(name), name)
    }

    // Removing the side takes its child out of the row, and the trait falls
    // back to the unknown it reads off a message that states none.
    const message = parsed.clone()
    const size = message.size
    assert.notEqual(message.remove(54), null)
    assert.equal(message.side, 'UNKNOWN')
    assert.equal(message.size, size - 1)
    // An ordinary child leaves, and the content identity follows it.
    const identity = message.currhashcode
    assert.equal(message.remove('VenueThing').asJs(), '7')
    assert.notEqual(message.currhashcode, identity)
    const settled = message.clone()
    assert.equal(message.remove('absent'), null)
    assert.ok(message.equals(settled))
  })

  test('a row reads back into the message that made it', () => {
    const registry = seed()
    // A whole-millisecond default SendingTime keeps every replay clock exact, so
    // `asJs` states each one as a `Date` below rather than as wider text.
    const codec = reading(registry, { defaultSendingTime: new Date(1_704_190_530_000) })
    const schema = fix.schema(registry)
    const parsed = one(codec, ORDER)
    const row = parsed.intoRow(schema)

    const held = fix.FixMsg.fromRow(schema, row, registry)

    // Rebuilding combines projected fields with residual entries. Semantic row
    // equality, rather than arrival entry order or wire spelling, is the contract.
    assert.equal(held.currhashcode, parsed.currhashcode)
    assert.equal(held.curruuid, parsed.curruuid)
    for (const tag of [8, 35, 11, 55, 54]) assert.ok(held.byTag(tag).equals(parsed.byTag(tag)), `tag ${tag}`)
    // And it makes the row it came from, whole.
    assert.ok(held.intoRow(schema).equals(row))
    // A row stated as plain JavaScript crosses the same gate the constructor
    // does, but the settled identity keeps its exact layouts: `asJs` states
    // the nanosecond clocks as millisecond `Date`s, which a read back refuses
    // at the first required column rather than restating it.
    // Every cell crosses the gate the constructor crosses, so a row stated as
    // plain JavaScript - its clocks as `Date`s, its UUIDs as text - reads back
    // into the same message, and the process default is the registry when none
    // is named.
    const named = fix.FixMsg.fromRow(schema, row.asJs(), registry)
    assert.ok(named.byTag(55).equals(parsed.byTag(55)))
    assert.ok(named.intoRow(schema).equals(row))
    assert.equal(named.currhashcode, held.currhashcode)
    assert.notEqual(fix.FixMsg.fromRow(schema, row).registry, null)
  })

  test('rows prune projected scalars and complete groups from residual entries', () => {
    const registry = seed()
    const codec = reading(registry)
    const schema = fix.schema(registry)
    const message = one(
      codec,
      Buffer.from('8=FIX.4.4|35=D|11=A1|55=AAPL|453=1|448=BRK|447=D|452=1|9999=x|10=0|'),
    )
    const row = message.intoRow(schema)
    const residual = row.asJs()[schema.indexOf('fixentries')]
    assert.ok(residual.every((entry) => entry[0] !== 55 && entry[0] !== 453))
    assert.ok(residual.some((entry) => entry[0] === 0))

    const rebuilt = fix.FixMsg.fromRow(schema, row, registry)
    assert.equal(rebuilt.byTag(55).asJs(), 'AAPL')
    assert.equal(rebuilt.byTag(453).asJs(), 1)
    assert.ok(rebuilt.intoRow(schema).equals(row))
  })

  test("a capture's own columns are carried and never become facts", () => {
    const registry = seed()
    const codec = reading(registry)
    // The object the line came out of is one of the capture's own columns:
    // no column of the fixed row states it, so a capture that knows it
    // declares it beside the body and the row number.
    const line = fields.struct(
      'line',
      [fields.utf8('url'), fields.int64('rownum'), fields.binary('body'), fields.url('sourceurl')],
      { nullable: false },
    )
    const schema = fix.schemaCarrying(line, fix.schema(registry))
    // A carried column is nullable whatever the capture declared: only a pass
    // holding the source row can state one.
    assert.ok(schema.field('url').nullable)
    const parsed = one(codec, ORDER)

    // A message parsed out of a line carries nothing: the capture's own
    // columns are null in its row, the one the crate tags among them.
    assert.deepEqual(parsed.carried, {})
    const row = parsed.intoRow(schema).asJs()
    for (const carrier of ['url', 'rownum', 'body', 'sourceurl']) {
      assert.equal(row[schema.indexOf(carrier)], null, carrier)
    }

    // A row a reader stated them on reads back carrying them, each under its
    // column's name: no child, nothing to answer by name, the content
    // identity untouched, and a write to the crate's own column refused
    // rather than silently kept.
    const stated = [...row]
    stated[schema.indexOf('url')] = 'file:///capture.log'
    stated[schema.indexOf('rownum')] = 42n
    const again = fix.FixMsg.fromRow(schema, stated, registry)
    assert.ok(again.intoRow(schema).equals(schema.scalar(stated)))
    assert.equal(again.currhashcode, parsed.currhashcode)
    assert.deepEqual(Object.keys(again.carried).sort(), ['rownum', 'url'])
    assert.equal(again.carried.url.asJs(), 'file:///capture.log')
    assert.equal(Number(again.carried.rownum.asJs()), 42)
    for (const carrier of ['url', 'rownum', 'body']) {
      assert.equal(again.getByName(carrier), null, carrier)
    }
    assert.throws(() => again.set('sourceurl', 'file:///capture.log'), /sourceurl/)

    // So a message states them again at their columns, and nowhere else.
    const written = again.intoRow(schema).asJs()
    assert.equal(written[schema.indexOf('url')], 'file:///capture.log')
    assert.equal(Number(written[schema.indexOf('rownum')]), 42)
    for (const carrier of ['body', 'sourceurl']) {
      assert.equal(written[schema.indexOf(carrier)], null, carrier)
    }
    assert.equal(written[schema.indexOf('symbol')], 'AAPL')
    assert.ok(!again.intoText('|').includes('65026='))
  })

  test('a row without the entries column keeps projected content', () => {
    const registry = seed()
    const codec = reading(registry)
    const wideSchema = fix.schema(registry)
    const columns = []
    for (let at = 0; at < wideSchema.fieldLen; at += 1) {
      const held = wideSchema.fieldAt(at)
      if (held.name !== 'fixentries' && held.name !== 'nofixentries') columns.push(held)
    }
    const narrow = fields.struct('fix', columns, { nullable: false })
    const parsed = one(codec, ORDER)
    const row = parsed.intoRow(narrow)
    const held = fix.FixMsg.fromRow(narrow, row, registry)
    // A row without residual entries still reconstructs projected content.
    assert.equal(held.byTag(55).asJs(), 'AAPL')
    assert.equal(held.header().msgtype, parsed.header().msgtype)
    assert.equal(held.currunix, parsed.currunix)
    const again = held.intoRow(narrow).asJs()
    assert.equal(again[narrow.indexOf('symbol')], 'AAPL')
    assert.deepEqual(again[narrow.indexOf('currunix')], row.asJs()[narrow.indexOf('currunix')])
    // A row that does not fit the schema is refused.
    assert.throws(() => fix.FixMsg.fromRow(narrow, { nosuchcolumn: 1 }, registry))
  })

  test('format answers the rows one message field holds, both doors', () => {
    // Pinned by `format_messages_answers_one_row_per_message_under_the_field`
    // and `format_arrow_reader_answers_the_batches_format_messages_answers_rows`
    // in `rust/tests/fix/messages.rs`.
    const registry = seed()
    const codec = reading(registry)
    // The fixed row itself as the target, so a formatted row keeps every column
    // the capture landed in.
    const schema = fix.schema(registry)
    const names = (held) => Array.from({ length: held.fieldLen }, (_, at) => held.fieldAt(at).name)

    const messages = [...codec.parseLine(Buffer.from(ORDER))]
    const rows = codec.formatMessages(messages, schema)
    assert.equal(rows.length, 1)
    const held = rows[0].asJs()
    assert.equal(held[schema.indexOf('symbol')], 'AAPL')
    // The record closes a formatted row exactly as it closes a parsed one.
    assert.ok(held[schema.indexOf('fixentries')].length > 0)

    // The Arrow twin answers the same row, one batch at a time, and decides its
    // schema before a row is read.
    const source = codec.arrowReader(schema, messages)
    const formatted = codec.formatArrowReader(source, schema)
    const table = formatted.intoTable()
    assert.deepEqual(
      table.schema.fields.map((field) => field.name),
      names(schema),
    )
    assert.equal(table.numRows, 1)
    assert.deepEqual(column(table, 'symbol'), ['AAPL'])
  })
}

// The `lifecycle` suite, in its own block: it brings its own
// fixtures, and `const` is block-scoped.
{
  // The one walk over events: `FixCodec.lifecycle` states each message as the
  // one after the live message it follows, and `lifecycleArrowReader` is the
  // same walk over batches of rows.
  //
  // Every rule is the core's, pinned in `rust/tests/fix/`; what these check is
  // the crossing - the stream a JavaScript iterable feeds one message at a
  // time, the facts each walked message carries, and the batch twin.

  const assert = require('node:assert/strict')
  const path = require('node:path')
  const test = require('node:test')

  const { BatchReader, DataType, IOBase, Scalar, TextLine, TextOptions, fields, fix } = require('yggdryl')

  const SEED = path.join(__dirname, '..', '..', 'config', 'fix')
  // A second of a ULBridge's own capture, anonymized: the corpus

  // A codec that reads every message type. The corpora below are captures, and
  // a capture holds the session traffic and the bridge rows stating no type
  // that `DEFAULT_REFUSED_MSGTYPES` drop; a case about the refusals says so for
  // itself.
  function reading(registry, options) {
    return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
  }

  // `rust/tests/fix/ulbridge.rs` reads.
  const CAPTURE = path.join(__dirname, '..', '..', 'rust', 'tests', 'fix', 'ulbridge.log')
  // The bridge's own row header, as the core spells it: what a line states
  // about itself in front of the payload. The core exports the text, so this
  // suite reads a bridge log under the same expression the crate ships rather
  // than a second copy of it.
  const ROWHEADER = fix.ULBRIDGE_ROWHEADER
  const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)

  let seedRegistry
  function seed() {
    seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
    return seedRegistry.clone()
  }

  /** The bridge capture as the messages a text read answers. */
  function captured(codec) {
    const options = new TextOptions()
    options.rowheader = ROWHEADER
    const messages = []
    for (const line of new IOBase(CAPTURE).readTextLines(options)) {
      for (const message of codec.parseTextLine(line)) messages.push(message)
    }
    return messages
  }

  /** One order's life, as a venue and its client tell it. */
  const LIFE = [
    // The order, sent under the client's own identifier.
    '8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|',
    // Acknowledged under the venue's, which now names the same chain.
    '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|',
    // Half of it done.
    '8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|',
    // Filled: the chain ends here.
    '8=FIX.4.4|35=8|11=A1|37=O1|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=100|14=100|151=0|32=50|31=12.6|60=20260102-10:15:33.000|10=0|',
  ]

  test('a message no live one precedes is answered as it came', () => {
    const registry = seed()
    const codec = reading(registry, { defaultSendingTime: SENDING })
    const heartbeat = codec.parseLine(Buffer.from('8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|')).next().value
    const [walked] = codec.lifecycle([heartbeat])
    // No cross code names no chain, so nothing precedes it and its own
    // identity is the chain's.
    assert.equal(walked.crosscode, '')
    assert.equal(walked.crossuuid, walked.curruuid)
    assert.equal(walked.prevuuid, null)
    assert.equal(walked.seqnum, 0)
    assert.ok(walked.equals(heartbeat))

    // Two chains are walked apart: each message follows the live one of its
    // own chain.
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|60=20260102-10:15:30.000|10=0|',
      '8=FIX.4.4|35=D|11=B1|55=MSFT|54=1|60=20260102-10:15:31.000|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=A1|17=E1|150=0|39=0|60=20260102-10:15:32.000|10=0|',
      '8=FIX.4.4|35=8|11=B1|37=B1|17=E2|150=0|39=0|60=20260102-10:15:33.000|10=0|',
    ]
    const walkedPair = [...codec.lifecycle(lines.map((line) => codec.parseLine(Buffer.from(line)).next().value))]
    assert.equal(walkedPair.length, 4)
    assert.equal(walkedPair[0].prevuuid, null)
    assert.equal(walkedPair[1].prevuuid, null, 'another chain, another first message')
    assert.equal(walkedPair[2].prevuuid, walkedPair[0].curruuid)
    assert.equal(walkedPair[3].prevuuid, walkedPair[1].curruuid)

    // A message read from a line states the line as its one source, and the
    // source is no part of the code: the same bytes are the same content,
    // whether the walk dated the message by its transaction or not.
    const line = new TextLine(0n, lines[0])
    const [sourced] = codec.parseTextLine(line)
    assert.deepEqual(sourced.srcuuids, [line.curruuid])
    assert.deepEqual(sourced.event().srcuuids, [line.curruuid])
    assert.equal(sourced.currhashcode, walkedPair[0].currhashcode)
    assert.deepEqual(walkedPair[0].srcuuids, [])
  })

  test('the stream is lazy, pulls one message at a time and throws what its source throws', () => {
    const registry = seed()
    const codec = reading(registry, { defaultSendingTime: SENDING })
    const parsed = LIFE.map((line) => codec.parseLine(Buffer.from(line)).next().value)

    // What is not iterable is refused before anything is pulled.
    assert.throws(() => codec.lifecycle(42), TypeError)
    // The walk reads its source in order, so an item that is not a message
    // ends the pull and throws in place of the stream's end.
    const mixed = codec.lifecycle([parsed[0], '8=FIX.4.4|35=0|10=0|'])
    assert.equal(mixed.next().done, false)
    assert.throws(() => mixed.next(), TypeError)
    assert.equal(mixed.next().done, true)

    // A failure of the iterable throws as itself, once, and ends the stream.
    function* failing() {
      yield parsed[0]
      throw new RangeError('the source broke')
    }
    const broken = codec.lifecycle(failing())
    assert.equal(broken.next().done, false)
    assert.throws(() => broken.next(), RangeError)
    assert.equal(broken.next().done, true)

    // The stream is its own iterator, and exhaustion is fused.
    const stream = codec.lifecycle(parsed)
    assert.equal(stream[Symbol.iterator](), stream)
    assert.equal([...stream].length, LIFE.length)
    assert.equal(stream.next().done, true)
  })

  test('the walk crosses Arrow both ways without a second parse', () => {
    const registry = seed()
    const codec = reading(registry, { defaultSendingTime: SENDING })
    const schema = fix.schema(registry)
    const parsed = LIFE.map((line) => codec.parseLine(Buffer.from(line)).next().value)

    const walked = codec.lifecycleArrowReader(codec.arrowReader(schema, parsed))
    assert.ok(walked instanceof BatchReader)
    const back = [...codec.messages(walked)]
    assert.equal(back.length, LIFE.length)
    // The same walk the message stream answers, through the rows: each
    // message states its place in the chain, the one before it, and the
    // semantic row of the corresponding stream message. Projected columns and
    // residual entries may rebuild in another child order; the row identity
    // and the chain it names are what the walk states.
    const expected = [...codec.lifecycle(parsed)]
    for (const [at, message] of back.entries()) {
      assert.equal(message.seqnum, expected[at].seqnum, `message ${at}`)
      assert.equal(message.crosscode, expected[at].crosscode, `message ${at}`)
      assert.ok(message.intoRow(schema).equals(expected[at].intoRow(schema)), `message ${at}`)
      assert.equal(message.prevuuid, at === 0 ? null : back[at - 1].curruuid, `message ${at}`)
      assert.deepEqual(message.srcuuids, [], `message ${at} was read from bytes`)
      assert.equal(message.crossuuid, back[0].crossuuid, `message ${at}`)
    }
    // The source is consumed, as every batch door consumes one.
    const source = codec.arrowReader(schema, parsed)
    codec.lifecycleArrowReader(source)
    assert.ok(source.consumed)
  })

  test('a bridge capture parses whole and walks its chains', () => {
    const registry = seed()
    const codec = reading(registry, {
      captureNames: ['timestamp', 'msgthreadid', 'msgsessionid', 'msgctxid', 'msgseqnum', 'msgpluginid', 'level'],
      defaultSendingTime: SENDING,
    })
    const messages = captured(codec)
    // Every line that carries a message is one message, the JSON documents
    // among them (`rust/tests/fix/ulbridge.rs`).
    assert.equal(messages.length, 94)

    // What a bridge's row header states reaches the capture, and what its own
    // namespaces state reaches the metadata.
    const report = messages.find((message) => message.header().msgtype === '8')
    assert.equal(report.capture().msgpluginid, 'ULBridge')
    assert.match(report.capture().msgctxid, /^[0-9a-f]{10}$/)
    assert.match(report.capture().msgsessionid, /^[0-9a-f]{8}$/)
    assert.ok(Object.keys(report.metadata).some((key) => key.startsWith('ullink.')))
    assert.ok(Object.keys(report.metadata).some((key) => key.startsWith('firm.')))
    assert.ok(Object.keys(report.metadata).every((key) => key === key.toLowerCase()))
    // The event reads the report: the instrument, the side, the price and the
    // quantity, and the names the message goes by.
    assert.equal(report.event().securityids.ISIN, 'CH0012214059')
    assert.equal(report.event().miccode, 'XSWX')
    assert.equal(report.side, 'BUY')
    assert.ok(Object.keys(report.altids).includes('CLORDID'))
    // The parties merge to one group with the counter synced.
    const parties = report.entries().find((entry) => entry.tag === 453)
    assert.equal(parties.value, '8')
    assert.equal(parties.entries.length, 8)

    // The finite capture fully merges observations carrying one forced
    // delivery key: message type, session, context and delivery sequence. The
    // latest recording is the reference and earlier observations fill it.
    // That coalesces 54 reports, one order, six cancel rejects and the six
    // rows that state no FIX type at all. One additional output expires a
    // live order at its stated deadline.
    //
    // It was 32 while the row header's clock admitted three fractional digits
    // and no more: the capture's last fifteen lines write grouped
    // microseconds, so those lines arrived with no session, context or
    // sequence and could not be folded onto the deliveries they repeat.
    const walked = [...codec.lifecycle(messages)]
    const expired = walked.filter((message) => message.state === '95EXPIRED')
    const retained = walked.filter((message) => message.state !== '95EXPIRED')
    assert.equal(retained.length, 27)
    assert.equal(expired.length, 1)
    assert.equal(walked.length, 28)

    const counts = (held) => {
      const found = new Map()
      for (const message of held) {
        const type = message.header().msgtype
        found.set(type, (found.get(type) ?? 0) + 1)
      }
      return found
    }
    const inputCounts = counts(messages)
    const retainedCounts = counts(retained)
    const removed = Object.fromEntries(
      [...inputCounts].map(([type, count]) => [type, count - (retainedCounts.get(type) ?? 0)]).filter(([, count]) => count > 0),
    )
    assert.deepEqual(removed, { 8: 54, '': 6, D: 1, cancelreject: 6 })

    // The default cross-code chains six retained bridge messages.
    // Every non-root message states both its predecessor and a positive sequence.
    assert.equal(walked.filter((message) => message.prevuuid !== null).length, 6)
    assert.equal(walked.filter((message) => message.seqnum > 0).length, 6)
    // A walked message descends from the whole chain before it. A fully merged
    // delivery keeps every observation's source, with each source belonging to
    // one output; only the six rows that state no FIX type lose their
    // provenance. Two cancel rejects lost theirs as well until the row
    // header's clock admitted the bridge's grouped microseconds - unread,
    // those lines carried no delivery key to be folded onto. The synthetic
    // expiry keeps its predecessor's provenance and lands at the stated
    // deadline.
    assert.ok(messages.every((message) => message.srcuuids.length === 1))
    const inputSources = new Set(messages.flatMap((message) => message.srcuuids))
    assert.ok(retained.every((message) =>
      message.srcuuids.length > 0 && message.srcuuids.every((source) => inputSources.has(source))))
    const retainedSources = retained.flatMap((message) => message.srcuuids)
    assert.equal(new Set(retainedSources).size, retainedSources.length)
    assert.equal(retainedSources.length, inputSources.size - 6)

    const [expiry] = expired
    const predecessor = retained.find((message) => message.curruuid === expiry.prevuuid)
    assert.ok(predecessor)
    assert.equal(expiry.currunix, predecessor.event().exprtime)
    assert.equal(expiry.seqnum, predecessor.seqnum + 1)
    assert.deepEqual(expiry.srcuuids, predecessor.srcuuids)

    // And the Arrow twin answers the same walk over the same corpus.
    const schema = fix.schema(registry)
    const rows = codec.lifecycleArrowReader(codec.arrowReader(schema, messages))
    const chained = [...codec.messages(rows)]
    assert.equal(chained.length, walked.length)
    assert.equal(chained.filter((message) => message.prevuuid !== null).length, 6)

    // Row intake preserves recorded identity; the lifecycle event clock,
    // facts and chain topology agree on both doors.
    const signature = (message) => {
      const header = message.header()
      const event = message.event()
      return {
        header: {
          beginstring: header.beginstring,
          msgtype: header.msgtype,
          sendercompid: header.sendercompid,
          targetcompid: header.targetcompid,
          msgseqnum: header.msgseqnum,
          possdupflag: header.possdupflag,
          msgdirection: header.msgdirection,
        },
        capture: message.capture(),
        metadata: message.metadata,
        event: {
          curruuid: event.curruuid,
          currhashcode: event.currhashcode,
          prevuuid: event.prevuuid,
          crossuuid: event.crossuuid,
          crosscode: event.crosscode,
          crosshashcode: event.crosshashcode,
          altids: event.altids,
          securityids: event.securityids,
          srcuuids: event.srcuuids,
          currunix: event.currunix,
          state: event.state,
          seqnum: event.seqnum,
          creaunix: event.creaunix,
          exprtime: event.exprtime,
          prevunix: event.prevunix,
          snapunix: event.snapunix,
        },
      }
    }
    assert.deepEqual(chained.map(signature), walked.map(signature))
    const topology = (held) => {
      const indices = new Map(held.map((message, at) => [message.curruuid, at]))
      const indexOf = (uuid) => {
        const at = indices.get(uuid)
        assert.notEqual(at, undefined, `${uuid} names a row in the finite walk`)
        return at
      }
      return held.map((message) => ({
        previous: message.prevuuid === null ? null : indexOf(message.prevuuid),
      }))
    }
    assert.deepEqual(topology(chained), topology(walked))
  })

  test('a transaction time stating only a day leaves the sending clock standing', () => {
    const codec = reading(seed(), { defaultSendingTime: SENDING })
    // `60=20260814` states a day and no clock, and a transaction time dates
    // nothing at the parse anyway: the event is the sending time
    // (`rust/tests/fix/`).
    const day = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|60=20260814|10=0|'))
    assert.equal(day.currunix, 1_704_190_530_000_000_000n)
    assert.equal(day.header().sendingtime, day.currunix)
    const [walked] = codec.lifecycle([day])
    assert.equal(walked.currunix, day.currunix)
  })

  test('the bridge row header crosses whole and names its own captures', () => {
    const options = new TextOptions()
    options.rowheader = fix.ULBRIDGE_ROWHEADER
    const captures = options.sourceField()
    const names = []
    for (let at = 0; at < captures.fieldLen; at += 1) {
      names.push(captures.fieldAt(at).name)
    }
    assert.deepEqual(names.slice(-7), [
      'timestamp',
      'msgthreadid',
      'msgsessionid',
      'msgctxid',
      'msgseqnum',
      'msgpluginid',
      'level',
    ])
    assert.equal(String(captures.field('msgseqnum').dtype), 'int64')
    // The clock is the capture's own column rather than `currunix`, so this
    // header dates no line: it is typed by its own syntax, where an `mtime`
    // capture would be consumed into `currunix` and read at nanoseconds UTC.
    assert.equal(String(captures.field('timestamp').dtype), 'datetime64(us)')
    assert.ok(!names.slice(-7).includes('mtime'))
  })
}
