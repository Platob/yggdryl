import {
  Cell,
  CellRange,
  CellRef,
  DataType,
  ExcelRow,
  Field,
  IOBase,
  RecordOptions,
  Scalar,
  Serie,
  Sheet,
  Workbook,
  excel,
  type CellDocument,
  type CellRangeInput,
  type CellRefInput,
  type DateSystem,
  type Excel,
  type SheetState,
} from '..'
import { Buffer } from 'node:buffer'

const referenceInput: CellRefInput = 'B2'
const pairInput: CellRefInput = [1, 1]
const reference: CellRef = new CellRef(referenceInput)
const fromPair: CellRef = new CellRef(pairInput)
const row: number = reference.row
const column: number = reference.column
const inGrid: boolean = reference.isInGrid()
const letters: string = CellRef.columnName(27)
const index: number | null = CellRef.columnIndex('AB')
const same: boolean = reference.equals(fromPair)
const order: number = reference.compare(fromPair)
const referenceText: string = reference.toString()
const referenceJson: string = reference.toJSON()
const referenceClone: CellRef = reference.clone()

const rangeInput: CellRangeInput = 'A1:C3'
const range: CellRange = new CellRange(rangeInput)
const fromPairs: CellRange = new CellRange([referenceInput, pairInput])
const everything: CellRange = CellRange.all()
const start: CellRef = range.start
const end: CellRef = range.end
const rows: number = range.rowSize()
const columns: number = range.columnSize()
const rowOpen: boolean = range.isRowOpen()
const columnOpen: boolean = range.isColumnOpen()
const inside: boolean = range.contains(referenceInput)
const insideRow: boolean = range.containsRow(1)
const insideColumn: boolean = range.containsColumn(1)
const rangeOrder: number = range.compare(fromPairs)
const rangeText: string = range.toString()
const cellsOfRange: CellRef[] = [...range]
const rangeClone: CellRange = everything.clone()

const system: DateSystem = '1900'
const cell: Cell = new Cell(referenceInput, 2.5, { dateSystem: system })
const dated: Cell = new Cell('C1', new DataType('date32').scalar('2024-01-02'))
const parts: Cell = Cell.fromParts('A1', 'number', 'general', Scalar.from(1), null, null)
const cellReference: CellRef = cell.reference
const cellRow: number = cell.row
const cellColumn: number = cell.column
const kind: string = cell.kind
const format: string = dated.format
const value: Scalar = cell.value
const formula: string | null = cell.formula
const error: string | null = cell.error
const isNull: boolean = cell.isNull()
const text: string = cell.text()
const withFormula: Cell = cell.withFormula('A1*2')
const withError: Cell = cell.withError('#DIV/0!')
const moved: Cell = cell.at('D4')
const cellEquals: boolean = cell.equals(parts)
const cellClone: Cell = cell.clone()
const document: CellDocument = cell.toJSON()
const documentReference: string = document.reference

const state: SheetState = 'hidden'
const sheet: Sheet = new Sheet('Trades', { dateSystem: system, state })
const defaultSheet: Sheet = new Sheet()
const name: string = sheet.name
sheet.name = 'Renamed'
const sheetState: string = sheet.state
sheet.state = 'visible'
const sheetSystem: string = sheet.dateSystem
const size: number = sheet.size
const empty: boolean = sheet.isEmpty()
const dimension: CellRange | null = sheet.dimension
const held: Cell | null = sheet.cell(referenceInput)
const scalar: Scalar = sheet.scalar(referenceInput)
const has: boolean = sheet.has(referenceInput)
const replaced: Cell | null = sheet.setCell(referenceInput, 'AAPL')
const inserted: Cell | null = sheet.insertCell(cell)
const removed: Cell | null = sheet.removeCell(referenceInput)
const sheetRows: ExcelRow[] = sheet.rows()
const oneRow: ExcelRow | null = sheet.row(0)
const allCells: Cell[] = sheet.cells()
const someCells: Cell[] = sheet.cellsIn(rangeInput)
const oneColumn: Cell[] = sheet.column(0)
const window: Sheet = sheet.slice(rangeInput)
sheet.insertRows(0, 2)
sheet.removeRows(0, 2)
const iterated: ExcelRow[] = [...sheet]
const sheetEquals: boolean = sheet.equals(window)
const sheetClone: Sheet = sheet.clone()
const sheetText: string = sheet.toString()

const field: Field = new Field('row', 'struct<id: int64 not null, symbol: utf8>', false)
const serie: Serie = Serie.fromScalars(field, [
  Scalar.from([1n, 'AAPL']),
  Scalar.from([2n, null]),
])
const laid: Sheet = Sheet.fromSerie('Trades', serie, { header: true })
laid.writeSerie('D1', serie, { header: false })
laid.extendFromSerie(serie)
const back: Serie = laid.intoSerie(field, { header: true, safe: false, representation: 'value' })
const inferred: Serie = laid.intoSerie()
const named: Serie = laid.intoSerie('struct<id: float64>')

if (oneRow !== null) {
  const rowIndex: number = oneRow.index
  const rowCells: Cell[] = oneRow.cells()
  const rowCell: Cell | null = oneRow.cell(0)
  const rowSize: number = oneRow.size
  void rowIndex
  void rowCells
  void rowCell
  void rowSize
}

const workbook: Workbook = new Workbook()
const opened: Workbook = Workbook.open('trades.xlsx')
const fromHandle: Workbook = Workbook.open(new IOBase('trades.xlsx'))
const fromBytes: Workbook = Workbook.fromBytes(Buffer.alloc(0))
const workbookSystem: string = workbook.dateSystem
workbook.dateSystem = '1904'
const names: string[] = workbook.sheetNames
const sheetKind: string | null = workbook.sheetKind('Trades')
const count: number = workbook.size
const workbookEmpty: boolean = workbook.isEmpty()
const hasSheet: boolean = workbook.has('Trades')
const views: Sheet[] = workbook.sheets()
const added: Sheet = workbook.addSheet('Trades')
const view: Sheet = workbook.sheet('Trades')
const maybe: Sheet | null = workbook.getSheet('Missing')
const at: Sheet | null = workbook.sheetAt(0)
const displaced: Sheet | null = workbook.insertSheet(laid)
workbook.renameSheet('Trades', 'Orders')
const taken: Sheet | null = workbook.removeSheet('Orders')
const iteratedSheets: Sheet[] = [...workbook]
const bytes: Buffer = workbook.intoBytes()
workbook.writeInto('trades.xlsx')
const reads: number = workbook.handleReads
const workbookText: string = workbook.toString()

const options: RecordOptions = RecordOptions.from('trades.xlsx')
const sheetOption: string | null = options.sheet
const headerOption: boolean | null = options.header
const rangeOption: CellRange | null = options.range
options.sheet = 'Trades'
options.header = false
options.range = 'A2:D'
const withSheet: RecordOptions = options.withSheet('Notes').withHeader(true).withRange(range)

const namespace: Excel = excel
const maxRows: number = excel.MAX_ROWS
const maxColumns: number = excel.MAX_COLUMNS
const maxText: number = excel.MAX_CELL_TEXT
const maxName: number = excel.MAX_SHEET_NAME
const defaultName: string = excel.DEFAULT_SHEET_NAME
const viaNamespace: Workbook = new excel.Workbook()
const rowClass: typeof ExcelRow = excel.Row

void row
void column
void inGrid
void letters
void index
void same
void order
void referenceText
void referenceJson
void referenceClone
void start
void end
void rows
void columns
void rowOpen
void columnOpen
void inside
void insideRow
void insideColumn
void rangeOrder
void rangeText
void cellsOfRange
void rangeClone
void cellReference
void cellRow
void cellColumn
void kind
void format
void value
void formula
void error
void isNull
void text
void withFormula
void withError
void moved
void cellEquals
void cellClone
void documentReference
void defaultSheet
void name
void sheetState
void sheetSystem
void size
void empty
void dimension
void held
void scalar
void has
void replaced
void inserted
void removed
void sheetRows
void allCells
void someCells
void oneColumn
void iterated
void sheetEquals
void sheetClone
void sheetText
void back
void inferred
void named
void opened
void fromHandle
void fromBytes
void workbookSystem
void names
void sheetKind
void count
void workbookEmpty
void hasSheet
void views
void added
void view
void maybe
void at
void displaced
void taken
void iteratedSheets
void bytes
void reads
void workbookText
void sheetOption
void headerOption
void rangeOption
void withSheet
void namespace
void maxRows
void maxColumns
void maxText
void maxName
void defaultName
void viaNamespace
void rowClass
