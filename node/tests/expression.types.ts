import {
  BatchReader,
  Bound,
  BoundStatement,
  Expression,
  Field,
  IOBase,
  Statement,
  Value,
  iceberg,
  type Table,
} from '..'
import { type ScanPlanCounts } from '../index'
import {
  tableFromArrays,
  type RecordBatch as ArrowRecordBatch,
  type Table as ArrowTable,
} from 'apache-arrow'

const filter: Expression = new Expression("ccy = 'EUR' and price > 100")
const parsed: Expression = Expression.parse("ccy = 'EUR'")
const restored: Expression = Expression.fromJson(filter.intoJson())
const named: Expression = Expression.column('ccy')
const constant: Expression = Expression.literal(Value.fromJs('EUR'))
const held: Expression = Expression.attribute('partition', 'year')
const stat: Expression = Expression.attribute('size')
const late: Expression = Expression.parameter('floor')
const always: Expression = Expression.alwaysTrue()
const never: Expression = Expression.alwaysFalse()

const columns: Array<string> = filter.columns
const attributes: Array<string> = filter.attributes
const parameters: Array<string> = filter.parameters
const conjuncts: Array<Expression> = filter.conjuncts()
const depth: number = filter.depth
const document: string = filter.intoJson()
// @ts-expect-error project conversions use into*, with no legacy alias
filter.toJson()
const text: string = filter.toString()
const same: boolean = filter.equals("ccy = 'EUR' and price > 100")
const expressionClone: Expression = filter.clone()
const expressionOrder: number = filter.compare(expressionClone)
const expressionHash: bigint = filter.stableHash()
const expressionJson: unknown = filter.toJSON()
const both: Expression = named.and(constant)
const either: Expression = named.or("size > 1")
const negated: Expression = named.not()
const sumExpression: Expression = Expression.column('price').add('1')
const inferredSumExpression: Expression = Expression.column('price').add(1)
const nativeSumExpression: Expression = Expression.column('price').add(Value.fromJs(1))
const differenceExpression: Expression = Expression.column('price').subtract('1')
const productExpression: Expression = Expression.column('price').multiply('2')
const quotientExpression: Expression = Expression.column('price').divide('2')
const remainderExpression: Expression = Expression.column('price').remainder('2')
const negativeExpression: Expression = Expression.column('price').negate()

const schema = new Field('trades', 'struct(field("ccy",utf8,nullable=true,metadata={}))', false)
const output: Field = named.field(schema)
const bound: Bound = named.bind(schema)
const boundExpression: Expression = bound.expression
const boundField: Field = bound.field
const isPredicate: boolean = bound.isPredicate
const boundColumns: Array<string> = bound.columns
const readsRows: boolean = bound.readsRows
const answered: Value = bound.eval(Value.fromJs(['EUR']))
const kept: boolean = bound.matches(Value.fromJs(['EUR']))
const boundText: string = bound.toString()

const statement: Statement = new Statement("select ccy where ccy = 'EUR' limit 10")
const clonedStatement: Statement = new Statement(statement).clone()
const sameStatement: boolean = statement.equals(clonedStatement)
const statementOrder: number = statement.compare(clonedStatement)
const statementHash: bigint = statement.stableHash()
const statementJson: unknown = statement.toJSON()
const fromDocument: Statement = Statement.fromJson(statement.intoJson())
const projections: Array<string> = statement.projections
const predicate: Expression | null = statement.predicate
const limit: number | null = statement.limit
const statementIsAll: boolean = statement.isAll
const orderedStatement = new Statement('select * order by ccy desc nulls first')
const orderingExpression: Expression = orderedStatement.ordering[0].expression
const orderingDirection: 'ascending' | 'descending' = orderedStatement.ordering[0].direction
const orderingNulls: 'first' | 'last' | null | undefined = orderedStatement.ordering[0].nulls
const boundStatement: BoundStatement = statement.bind(schema)
const boundStatementSchema: Field = boundStatement.schema
const boundStatementOutput: Field = boundStatement.output
const boundStatementProjections: Array<Bound> = boundStatement.projections
const boundStatementPredicate: Bound | null = boundStatement.predicate
const boundStatementIsAll: boolean = boundStatement.isAll
const arrowTable: ArrowTable = tableFromArrays({ ccy: ['EUR'] })
const arrowBatch: ArrowRecordBatch = arrowTable.batches[0]
const projectedBatch: ArrowRecordBatch = boundStatement.projectArrowRecordBatch(arrowBatch)
const projectedBatchInferred: ArrowRecordBatch = boundStatement.projectArrow(arrowBatch)
const projectedTable: ArrowTable = boundStatement.projectArrowTable(arrowTable)
const projectedTableInferred: ArrowTable = boundStatement.projectArrow(arrowTable)
const projectedReader: BatchReader = boundStatement.projectArrowReader(BatchReader.from(arrowTable))
const projectedReaderInferred: BatchReader = boundStatement.projectArrow(BatchReader.from(arrowTable))
const sortedBatch: ArrowRecordBatch = orderedStatement
  .bind(schema)
  .sortArrowRecordBatch(arrowBatch)
const statementText: string = statement.toString()

const handle = new IOBase('file:///lake')
const matching: Array<IOBase> = [...handle.childrenMatching(filter)]
const matchingText: Array<IOBase> = [...handle.childrenMatching("&holder.size > 0", true)]

const table: Table = iceberg.Table.create('file:///lake/trades', schema, ['ccy'])
const rows: BatchReader = table.scanMatching(filter)
const projectedRows: BatchReader = table.scanMatching("ccy = 'EUR'", schema)
const counts: ScanPlanCounts = table.planMatching(filter)
const tasks: number = counts.tasks
const manifestsSkipped: number = counts.manifestsSkipped

export {
  always,
  answered,
  attributes,
  both,
  bound,
  boundColumns,
  boundExpression,
  boundField,
  boundText,
  boundStatement,
  boundStatementIsAll,
  boundStatementOutput,
  boundStatementPredicate,
  boundStatementProjections,
  boundStatementSchema,
  columns,
  conjuncts,
  constant,
  counts,
  depth,
  document,
  either,
  filter,
  fromDocument,
  held,
  isPredicate,
  kept,
  late,
  limit,
  manifestsSkipped,
  matching,
  matchingText,
  named,
  negated,
  sumExpression,
  inferredSumExpression,
  nativeSumExpression,
  differenceExpression,
  productExpression,
  quotientExpression,
  remainderExpression,
  negativeExpression,
  never,
  output,
  orderedStatement,
  orderingDirection,
  orderingExpression,
  orderingNulls,
  parameters,
  parsed,
  predicate,
  projectedRows,
  projectedBatch,
  projectedBatchInferred,
  projectedReader,
  projectedReaderInferred,
  projectedTable,
  projectedTableInferred,
  projections,
  readsRows,
  restored,
  rows,
  same,
  expressionClone,
  expressionOrder,
  expressionHash,
  expressionJson,
  schema,
  statement,
  clonedStatement,
  sameStatement,
  statementOrder,
  statementHash,
  statementJson,
  statementIsAll,
  statementText,
  stat,
  table,
  tasks,
  text,
  sortedBatch,
}
