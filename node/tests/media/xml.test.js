'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { Field, Scalar, xml } = require('yggdryl')

const ORDER = '<Order id="7"><symbol>AAPL</symbol><qty>100</qty></Order>'
const BOOK_SCHEMA = `<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="urn:book" xmlns="urn:book"
           elementFormDefault="qualified">
  <xs:element name="book">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="title" type="xs:string"/>
        <xs:element name="pages" type="xs:int"/>
      </xs:sequence>
      <xs:attribute name="isbn" type="xs:string" use="required"/>
    </xs:complexType>
  </xs:element>
  <xs:element name="note" type="xs:string"/>
</xs:schema>`

test('a document is one namespace of names, and every leaf is text', () => {
  assert.ok(Object.isFrozen(xml))
  assert.deepEqual(xml.loads(ORDER), {
    id: '7',
    qty: '100',
    symbol: 'AAPL',
  })
  // Attributes and child elements share the one namespace, so characters
  // beside attributes need a name of their own rather than being dropped.
  assert.deepEqual(xml.loads('<Amt Ccy="EUR">9.50</Amt>'), {
    Ccy: 'EUR',
    value: '9.50',
  })
  // A repeated child is the sequence it proves, whatever its prefix.
  assert.deepEqual(
    xml.loads(
      '<Trades xmlns:t="urn:t">' +
        '<t:Trade side="buy">1</t:Trade>' +
        '<t:Trade side="sell">2</t:Trade>' +
        '</Trades>',
    ),
    { Trade: [{ side: 'buy', value: '1' }, { side: 'sell', value: '2' }] },
  )
  assert.equal(xml.loads('<a/>'), '')
  assert.equal(
    xml.loads(
      '<a xsi:nil="true" ' +
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"/>',
    ),
    null,
  )
})

test('every documented input spelling decodes the same document', () => {
  const bytes = Buffer.from(ORDER, 'utf8')
  const expected = xml.loads(ORDER)

  assert.deepEqual(xml.loads(bytes), expected)
  assert.deepEqual(xml.loads(new Uint8Array(bytes)), expected)
  assert.deepEqual(
    xml.loads(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)),
    expected,
  )
})

test('a write names its document element and chooses its layout', () => {
  const value = { symbol: 'AAPL', qty: 100 }
  const pretty = xml.dumps(value, 'Order').toString('utf8')
  const flat = xml.dumps(value, 'Order', { indent: 0 }).toString('utf8')
  const wide = xml.dumps(value, 'Order', { indent: 4 }).toString('utf8')

  assert.ok(Buffer.isBuffer(xml.dumps(value, 'Order')))
  assert.equal(
    pretty,
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
      '<Order>\n  <qty>100</qty>\n  <symbol>AAPL</symbol>\n</Order>\n',
  )
  assert.equal(
    flat,
    '<?xml version="1.0" encoding="UTF-8"?>' +
      '<Order><qty>100</qty><symbol>AAPL</symbol></Order>',
  )
  assert.ok(wide.includes('\n    <qty>100</qty>\n'))
  // Text that would change the parse is escaped, never emitted raw.
  assert.ok(
    xml
      .dumps({ note: 'a < b & c' }, 'Doc', { indent: 0 })
      .toString('utf8')
      .includes('<note>a &lt; b &amp; c</note>'),
  )
})

test('a field types the leaves a document only proves the shape of', () => {
  const field = xml.schema(BOOK_SCHEMA, { root: 'book' })
  const typed = xml.loadsWithField(
    '<book isbn="1-84-1"><title>Dune</title><pages>412</pages></book>',
    field,
  )

  assert.ok(field instanceof Field)
  assert.ok(typed instanceof Scalar)
  assert.deepEqual(typed.asJs(), ['1-84-1', 'Dune', 412])
  assert.equal(String(field.fieldAt(2).dtype), 'int32')
  // Without a field the same document is all text.
  assert.equal(
    xml.loads('<book isbn="1-84-1"><pages>412</pages></book>').pages,
    '412',
  )
})

test('an XML Schema is the field it declares, and writes back as itself', () => {
  const book = xml.schema(BOOK_SCHEMA, { root: 'book' })
  const note = xml.schema(BOOK_SCHEMA, { root: 'note' })
  const written = xml.schemaDumps(book).toString('utf8')

  assert.equal(book.name, 'book')
  assert.equal(book.protocol('xml').get('namespace'), 'urn:book')
  assert.equal(book.field('isbn').protocol('xml').get('kind'), 'attribute')
  assert.equal(String(note.dtype), 'utf8')
  assert.ok(written.includes('<xs:element name="title" type="xs:string"/>'))
  assert.ok(written.includes('<xs:attribute name="isbn" type="xs:string"'))
  assert.ok(xml.schemaDumps(book, { indent: 0 }).toString('utf8').includes('?><xs:schema'))
  // The written schema declares the same field it was read from.
  assert.ok(xml.schema(written).equals(book))
})

test('a decode spends the budget it is given and no more', () => {
  assert.throws(
    () => xml.loads('<a><b><c/></b></a>', { maxDepth: 2 }),
    /nesting depth limit exceeded/,
  )
  assert.throws(
    () => xml.loads('<a><b/><c/></a>', { maxNodes: 2 }),
    /decoded node limit exceeded/,
  )
  assert.throws(() => xml.loads(ORDER, { maxInputBytes: 4 }), /input/)
  assert.throws(
    () => xml.loads('<a>'),
    /expected every element to close, got the end of the document/,
  )
  assert.throws(() => xml.loads('<a>&unknown;</a>'), /invalid xml data at byte/)
})

test('the XML surface names what it will not take', () => {
  assert.throws(() => xml.loads(1), TypeError)
  assert.throws(() => xml.loads('<a/>', { indent: 2 }), {
    name: 'TypeError',
    message: 'unknown XML decode option indent',
  })
  assert.throws(() => xml.loads('<a/>', [1]), {
    name: 'TypeError',
    message: 'XML decode options must be a plain object',
  })
  assert.throws(() => xml.loads('<a/>', { maxNodes: -1 }), RangeError)
  assert.throws(() => xml.loads('<a/>', { maxNodes: 1.5 }), RangeError)
  assert.throws(() => xml.dumps({}, 1), TypeError)
  assert.throws(() => xml.dumps({}, 'a', { indent: 256 }), RangeError)
  assert.throws(() => xml.schema(BOOK_SCHEMA, { root: 7 }), TypeError)
  assert.throws(() => xml.schema(BOOK_SCHEMA), /expected one global element/)
  assert.throws(() => xml.schemaDumps({}), {
    name: 'TypeError',
    message: 'the declared field must be a Field',
  })
  assert.throws(() => xml.loadsWithField(ORDER, {}), {
    name: 'TypeError',
    message: 'the declared field must be a Field',
  })
})
