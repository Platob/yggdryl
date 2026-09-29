'use strict'

// `node/src/excel.rs` and the loader's Excel section: the workbook, its
// sheets and cells, and the record options a `.xlsx` handle takes.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const {
  Cell,
  CellRange,
  CellRef,
  DataType,
  IOBase,
  RecordOptions,
  Scalar,
  Serie,
  Sheet,
  Workbook,
  excel,
} = require('yggdryl')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-excel-'))
}

function trades() {
  return new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL', null, 'MSFT'], new arrow.Utf8()),
    price: arrow.vectorFromArray([187.5, 410.25, -0.5], new arrow.Float64()),
    live: arrow.vectorFromArray([true, false, true], new arrow.Bool()),
  })
}

test('the namespace names the classes and the grid', () => {
  assert.ok(Object.isFrozen(excel))
  assert.equal(excel.Workbook, Workbook)
  assert.equal(excel.Sheet, Sheet)
  assert.equal(excel.Cell, Cell)
  assert.equal(excel.CellRef, CellRef)
  assert.equal(excel.CellRange, CellRange)
  assert.equal(typeof excel.Row, 'function')
  assert.equal(excel.MAX_ROWS, 1_048_576)
  assert.equal(excel.MAX_COLUMNS, 16_384)
  assert.equal(excel.MAX_CELL_TEXT, 32_767)
  assert.equal(excel.MAX_SHEET_NAME, 31)
  assert.equal(excel.DEFAULT_SHEET_NAME, 'Sheet1')
})

test('record options address a sheet, a header and a range, and nothing else does', () => {
  const options = RecordOptions.from('trades.xlsx')
  assert.equal(options.sheet, null)
  assert.equal(options.header, true)
  assert.equal(options.range, null)
  options.sheet = 'Trades'
  options.header = false
  options.range = 'A3:F'
  assert.equal(options.sheet, 'Trades')
  assert.equal(options.header, false)
  assert.ok(options.range instanceof CellRange)
  assert.equal(String(options.range), 'A3:F')
  options.range = new CellRange(['B2', 'C3'])
  assert.equal(String(options.range), 'B2:C3')
  options.range = null
  options.sheet = null
  assert.equal(options.range, null)
  assert.equal(options.sheet, null)
  assert.throws(() => {
    options.sheet = 'a/b'
  }, /sheet name/)

  const chained = options.withSheet('Quotes').withHeader(false).withRange([[1, 0], [9, 3]])
  assert.equal(chained.sheet, 'Quotes')
  assert.equal(chained.header, false)
  assert.equal(String(chained.range), 'A2:D10')
  assert.equal(options.sheet, null, 'with* answers a copy')

  const ipc = RecordOptions.from('trades.arrows')
  assert.equal(ipc.sheet, null)
  assert.equal(ipc.header, null)
  assert.equal(ipc.range, null)
  assert.throws(() => {
    ipc.sheet = 'Trades'
  }, /expected Excel options/)
  // The header is one knob a CSV and a workbook share.
  assert.throws(() => ipc.withHeader(false), /expected CSV or Excel options/)
})

test('records round-trip through a workbook file', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const file = new IOBase(path.join(root, 'trades.xlsx'))
  assert.equal(file.recordOptions().mimeType.toString(), 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')
  file.overwriteArrowTable(trades())
  // A number cell is a float64: the file knows no other number.
  const inferred = file.readArrowReader().intoTable()
  assert.deepEqual(inferred.getChild('id').toArray(), Float64Array.from([1, 2, 3]))
  assert.deepEqual([...inferred.getChild('symbol')], ['AAPL', null, 'MSFT'])
  assert.deepEqual([...inferred.getChild('live')], [true, false, true])
  assert.equal(file.rowSize, 3)
  assert.equal(file.columnSize, 4)

  // Declared, the ids come back as the int64 they were.
  const declared = file.recordOptions().withField(Serie.fromArrowBatch(trades()).field)
  const typed = file.readArrowReader(declared).intoTable()
  assert.deepEqual([...typed.getChild('id')], [1n, 2n, 3n])

  // A second sheet, addressed by name; a header-less read names the
  // columns by their letters, the header row left out by the range.
  file.overwriteArrowTable(
    new arrow.Table({ note: arrow.vectorFromArray(['a', 'b'], new arrow.Utf8()) }),
    file.recordOptions().withSheet('Notes'),
  )
  assert.deepEqual(Workbook.open(file).sheetNames, ['Sheet1', 'Notes'])
  assert.deepEqual([...file.readArrowReader(file.recordOptions().withSheet('Notes')).intoTable().getChild('note')], ['a', 'b'])
  const lettered = file
    .readArrowReader(file.recordOptions().withHeader(false).withRange('A2:D'))
    .intoTable()
  assert.deepEqual(lettered.schema.fields.map((field) => field.name), ['A', 'B', 'C', 'D'])
  assert.equal(lettered.numRows, 3)
  // A sheet the workbook lacks reads as nothing.
  assert.equal(file.readArrowReader(file.recordOptions().withSheet('Missing')).intoTable().numRows, 0)
})

test('a reference reads every spelling and is a value', () => {
  for (const value of ['B3', '$B$3', [2, 1], new CellRef('B3')]) {
    const reference = new CellRef(value)
    assert.equal(reference.row, 2)
    assert.equal(reference.column, 1)
    assert.equal(String(reference), 'B3')
    assert.equal(reference.toJSON(), 'B3')
  }
  assert.ok(new CellRef('A1').isInGrid())
  assert.equal(CellRef.columnName(0), 'A')
  assert.equal(CellRef.columnName(27), 'AB')
  assert.equal(CellRef.columnIndex('XFD'), 16_383)
  assert.equal(CellRef.columnIndex('XFE'), null)
  assert.ok(new CellRef('A1').equals(new CellRef([0, 0])))
  assert.equal(new CellRef('A1').compare(new CellRef('B2')), -1)
  assert.ok(new CellRef('B2').clone().equals(new CellRef('B2')))
  assert.throws(() => new CellRef('Sheet1!A1'), /sheet-qualified/)
  assert.throws(() => new CellRef('A0'), /row number/)
  assert.throws(() => new CellRef([1_048_576, 0]), /row/)
})

test('a range reads every spelling, contains and iterates', () => {
  const closed = new CellRange('C3:A1')
  assert.equal(String(closed), 'A1:C3')
  assert.equal(String(closed.start), 'A1')
  assert.equal(String(closed.end), 'C3')
  assert.equal(closed.rowSize, 3)
  assert.equal(closed.columnSize, 3)
  assert.ok(!closed.isRowOpen() && !closed.isColumnOpen())
  assert.ok(new CellRange([new CellRef('C3'), 'A1']).equals(closed))
  assert.equal(new CellRange('A:C').rowSize, excel.MAX_ROWS)
  assert.ok(new CellRange('3:5').isColumnOpen())
  assert.equal(String(new CellRange('A3:F')), 'A3:F')
  assert.equal(CellRange.all().columnSize, excel.MAX_COLUMNS)
  const span = new CellRange('B2:C3')
  assert.ok(span.contains('C3') && span.contains([1, 1]) && !span.contains('A1'))
  assert.ok(span.containsRow(2) && !span.containsRow(3))
  assert.ok(span.containsColumn(1) && !span.containsColumn(3))
  assert.deepEqual([...span].map(String), ['B2', 'C2', 'B3', 'C3'])
  assert.equal(closed.compare(new CellRange('A2:C3')), -1)
  assert.throws(() => new CellRange('A:3'), /expected a cell range/)
})

test('a cell states its kind, format and value for every native', () => {
  const text = new Cell('A1', 'AAPL')
  assert.equal(text.kind, 's')
  assert.equal(text.format, 'general')
  assert.ok(text.value instanceof Scalar)
  assert.equal(text.value.asJs(), 'AAPL')
  assert.equal(text.text(), 'AAPL')
  assert.equal(String(text), 'A1=AAPL')
  const number = new Cell([0, 1], 187.5)
  assert.equal(number.kind, 'n')
  assert.equal(number.value.asJs(), 187.5)
  const flag = new Cell('C1', true)
  assert.equal(flag.kind, 'b')
  const day = new Cell('D1', new DataType('date32').scalar('2024-01-02'))
  assert.equal(day.format, 'date')
  const empty = new Cell('F1', null)
  assert.ok(empty.isNull())
  assert.equal(String(empty.reference), 'F1')
  assert.equal(empty.row, 0)
  assert.equal(empty.column, 5)

  const formula = new Cell('A1', 3).withFormula('1+2')
  assert.equal(formula.formula, '1+2')
  assert.equal(formula.error, null)
  assert.equal(String(formula.at('B2').reference), 'B2')
  const failed = new Cell('A1', null).withError('#DIV/0!')
  assert.equal(failed.error, '#DIV/0!')
  assert.deepEqual(JSON.parse(JSON.stringify(text)), {
    reference: 'A1',
    kind: 's',
    format: 'general',
    formula: null,
    error: null,
    value: 'AAPL',
  })
  assert.ok(text.equals(new Cell('A1', 'AAPL')))
  assert.ok(!text.equals(new Cell('A2', 'AAPL')))
  assert.ok(text.clone().equals(text))
  assert.ok(Cell.fromParts('A1', 's', 'general', Scalar.from('AAPL'), null, null).equals(text))
  assert.throws(() => new Cell('A1', new DataType('date32').scalar('1903-12-31'), { dateSystem: '1904' }), /1904/)
  assert.throws(() => new Cell('A1', 1, { dateSystem: '1930' }), /date system/)
})

test('a sheet is built cell by cell and read back by reference', () => {
  const sheet = new Sheet('Trades')
  assert.equal(sheet.name, 'Trades')
  assert.equal(sheet.state, 'visible')
  assert.equal(sheet.dateSystem, '1900')
  assert.equal(sheet.size, 0)
  assert.ok(sheet.isEmpty())
  assert.equal(sheet.dimension, null)
  assert.equal(sheet.cell('A1'), null)
  assert.equal(sheet.setCell('A1', 'symbol'), null)
  sheet.setCell([0, 1], 'price')
  sheet.setCell('A2', 'AAPL')
  sheet.setCell('B2', 187.5)
  assert.equal(sheet.setCell('B2', 190).value.asJs(), 187.5)
  assert.equal(sheet.cell('B2').value.asJs(), 190)
  assert.equal(sheet.scalar('B2').asJs(), 190)
  assert.equal(sheet.scalar('Z9').asJs(), null)
  assert.ok(sheet.has('A1') && !sheet.has('Z9'))
  assert.equal(sheet.size, 2)
  assert.equal(String(sheet.dimension), 'A1:B2')
  assert.deepEqual(sheet.cells().map((cell) => cell.value.asJs()), ['symbol', 'price', 'AAPL', 190])
  assert.deepEqual(sheet.column(0).map((cell) => cell.value.asJs()), ['symbol', 'AAPL'])
  assert.deepEqual(sheet.rows().map((row) => row.index), [0, 1])
  assert.deepEqual([...sheet].map((row) => row.size), [2, 2])
  const row = sheet.row(1)
  assert.equal(row.size, 2)
  assert.equal(row.cell(1).value.asJs(), 190)
  assert.deepEqual(row.cells().map((cell) => String(cell.reference)), ['A2', 'B2'])
  assert.equal(String(sheet), "Sheet('Trades', 2 rows)")

  const sliced = sheet.slice('A2:B2')
  assert.deepEqual(sliced.cells().map((cell) => String(cell.reference)), ['A2', 'B2'])
  assert.deepEqual(sheet.cellsIn('B1:B2').map((cell) => cell.value.asJs()), ['price', 190])
  assert.equal(sheet.removeCell('A1').value.asJs(), 'symbol')
  assert.equal(sheet.removeCell('A1'), null)
  sheet.insertCell(new Cell('A1', 'id'))
  assert.equal(sheet.cell('A1').value.asJs(), 'id')
  sheet.insertRows(1, 2)
  assert.equal(sheet.cell('A4').value.asJs(), 'AAPL')
  sheet.removeRows(1, 3)
  assert.equal(sheet.cell('A2').value.asJs(), 'AAPL')

  sheet.state = 'hidden'
  assert.equal(sheet.state, 'hidden')
  sheet.name = 'Fills'
  assert.equal(sheet.name, 'Fills')
  assert.throws(() => {
    sheet.name = 'a'.repeat(32)
  }, /sheet name/)
  assert.throws(() => new Sheet('History'), /sheet name/)
  assert.equal(new Sheet().name, 'Sheet1')
  assert.equal(new Sheet('Dates', { dateSystem: '1904', state: 'veryHidden' }).dateSystem, '1904')
})

test('a sheet lays a table out and reads it back as a serie', () => {
  const sheet = Sheet.fromSerie('Trades', trades())
  assert.equal(sheet.size, 4)
  assert.deepEqual(sheet.row(0).cells().map((cell) => cell.value.asJs()), ['id', 'symbol', 'price', 'live'])
  assert.equal(sheet.cell('B3'), null, 'the null symbol writes no cell')
  const serie = sheet.intoSerie()
  assert.ok(serie instanceof Serie)
  const rows = serie.asJs()
  assert.equal(rows.length, 3)
  assert.deepEqual(rows[0], { id: 1, symbol: 'AAPL', price: 187.5, live: true })
  // A sheet laid out in memory keeps each value as it was, so inference
  // reads the int64 the cells hold - a parsed part holds the float64 a
  // number cell is - and a safe integer crosses as a number, as every
  // int64 does through asJs.
  const dtypes = (field) => [0, 1, 2, 3].map((index) => String(field.dtype.fieldAt(index).dtype))
  assert.deepEqual(dtypes(serie.field), ['int64', 'utf8', 'float64', 'boolean'])
  const declared = sheet.intoSerie(Serie.fromArrowBatch(trades()).field)
  assert.deepEqual(dtypes(declared.field), ['int64', 'utf8', 'float64', 'boolean'])
  assert.deepEqual(declared.asJs()[0], { id: 1, symbol: 'AAPL', price: 187.5, live: true })
  const lettered = sheet.slice('A2:D4').intoSerie(null, { header: false })
  assert.deepEqual([0, 1, 2, 3].map((index) => lettered.field.dtype.fieldAt(index).name), ['A', 'B', 'C', 'D'])
  assert.throws(() => sheet.intoSerie(null, { header: false }), /Trades!A2.*declare a field/)

  const anchored = new Sheet('Anchored')
  anchored.writeSerie('B3', trades(), { header: false })
  assert.equal(anchored.cell('B3').value.asJs(), 1)
  assert.equal(anchored.cell('E5').value.asJs(), true)
  anchored.extendFromSerie(Serie.fromArrowBatch(new arrow.Table({
    id: arrow.vectorFromArray([4n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['GOOG'], new arrow.Utf8()),
    price: arrow.vectorFromArray([1], new arrow.Float64()),
    live: arrow.vectorFromArray([false], new arrow.Bool()),
  })))
  assert.equal(anchored.cell('C6').value.asJs(), 'GOOG')
  assert.ok(sheet.equals(Sheet.fromSerie('Trades', trades())))
  assert.ok(!sheet.equals(anchored))
  assert.ok(sheet.clone().equals(sheet))
})

test('a workbook opens from a handle, a path or bytes and hands out live views', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const file = new IOBase(path.join(root, 'trades.xlsx'))
  file.overwriteArrowTable(trades())

  for (const source of [file, path.join(root, 'trades.xlsx'), file.readBytes()]) {
    const workbook = Workbook.open(source)
    assert.deepEqual(workbook.sheetNames, ['Sheet1'])
    assert.equal(workbook.size, 1)
    assert.ok(workbook.has('Sheet1') && workbook.has('sheet1') && !workbook.has('Missing'))
    assert.equal(workbook.sheetKind('Sheet1'), 'worksheet')
    assert.equal(workbook.sheetKind('Missing'), null)
  }
  const workbook = Workbook.fromBytes(file.readBytes())
  assert.equal(String(workbook), "Workbook(['Sheet1'])")
  assert.equal(workbook.dateSystem, '1900')
  assert.ok(workbook.handleReads >= 1)

  const sheet = workbook.sheet('Sheet1')
  assert.equal(sheet.cell('A1').value.asJs(), 'id')
  assert.equal(sheet.cell('B2').value.asJs(), 'AAPL')
  assert.equal(sheet.cell('B3'), null)
  assert.equal(workbook.sheet('sheet1').cell('C4').value.asJs(), -0.5)
  assert.equal(workbook.sheetAt(0).name, 'Sheet1')
  assert.equal(workbook.sheetAt(1), null)
  assert.equal(workbook.getSheet('Missing'), null)
  assert.throws(() => workbook.sheet('Missing'), /Missing.*Sheet1/)
  assert.deepEqual([...workbook].map((view) => view.name), ['Sheet1'])

  // The view is live: a cell set through it is what the workbook writes.
  sheet.setCell('E1', 'note')
  sheet.setCell([3, 4], 2.5)
  const reopened = Workbook.fromBytes(workbook.intoBytes())
  assert.equal(reopened.sheet('Sheet1').cell('E1').value.asJs(), 'note')
  assert.equal(reopened.sheet('Sheet1').cell('E4').value.asJs(), 2.5)
  assert.equal(reopened.sheet('Sheet1').cell('B2').value.asJs(), 'AAPL')

  // In memory as well: an IOBase over bytes opens without a location.
  const memory = IOBase.fromBytes(file.readBytes())
  memory.mediaType = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
  assert.deepEqual(Workbook.open(memory).sheetNames, ['Sheet1'])
})

test('sheets are added, inserted, renamed and removed', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const workbook = new Workbook()
  assert.ok(workbook.isEmpty())
  const tradesSheet = workbook.addSheet('Trades')
  tradesSheet.setCell('A1', 'symbol')
  tradesSheet.setCell('A2', 'AAPL')
  assert.equal(workbook.sheet('Trades').cell('A2').value.asJs(), 'AAPL')
  assert.throws(() => workbook.addSheet('trades'), /no other sheet has/)

  const notes = Sheet.fromSerie('Notes', new arrow.Table({ note: arrow.vectorFromArray(['a', 'b'], new arrow.Utf8()) }))
  assert.equal(workbook.insertSheet(notes), null)
  // The object now views the sheet inside the workbook.
  notes.setCell('C1', 'seen')
  assert.equal(workbook.sheet('Notes').cell('C1').value.asJs(), 'seen')
  assert.deepEqual(workbook.sheetNames, ['Trades', 'Notes'])
  notes.name = 'Remarks'
  assert.deepEqual(workbook.sheetNames, ['Trades', 'Remarks'])
  assert.throws(() => {
    notes.name = 'TRADES'
  }, /no other sheet has/)
  workbook.renameSheet('Remarks', 'Notes')
  assert.deepEqual(workbook.sheetNames, ['Trades', 'Notes'])

  const replaced = workbook.insertSheet(new Sheet('Notes'))
  assert.equal(replaced.cell('C1').value.asJs(), 'seen')
  assert.ok(workbook.sheet('Notes').isEmpty())
  assert.equal(workbook.removeSheet('Notes').name, 'Notes')
  assert.equal(workbook.removeSheet('Notes'), null)
  workbook.removeSheet('Trades')
  assert.ok(workbook.isEmpty())

  workbook.addSheet('Only').setCell('A1', 1)
  const target = path.join(root, 'written.xlsx')
  workbook.writeInto(target)
  assert.equal(Workbook.open(target).sheet('Only').cell('A1').value.asJs(), 1)
  const handle = new IOBase(path.join(root, 'second.xlsx'))
  workbook.writeInto(handle)
  assert.deepEqual(Workbook.open(handle).sheetNames, ['Only'])
  assert.equal(handle.readArrowReader().intoTable().numRows, 0)

  workbook.dateSystem = '1904'
  const dates = workbook.addSheet('Dates')
  dates.setCell('A1', new DataType('date32').scalar('2024-02-29'))
  const reopened = Workbook.fromBytes(workbook.intoBytes())
  assert.equal(reopened.dateSystem, '1904')
  assert.equal(reopened.sheet('Dates').cell('A1').value.toJSON(), '2024-02-29')
  assert.throws(() => {
    workbook.dateSystem = '1930'
  }, /date system/)
})

test('bytes that are not a package are refused by name', () => {
  assert.throws(() => Workbook.fromBytes(Buffer.concat([Buffer.from([0xd0, 0xcf, 0x11, 0xe0]), Buffer.alloc(12)])), /BIFF/)
  assert.throws(() => Workbook.fromBytes(Buffer.from('not a package at all')), /ZIP package/)
  assert.ok(Workbook.fromBytes(Buffer.alloc(0)).isEmpty())
})
