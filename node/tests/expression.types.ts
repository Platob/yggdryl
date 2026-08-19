import {
  BatchReader,
  Bound,
  Expression,
  Field,
  IOBase,
  Statement,
  Value,
  iceberg,
  type Table,
} from '..'
import { type ScanPlanCounts } from '../index'

const filter: Expression = new Expression("ccy = 'EUR' and price > 100")
const parsed: Expression = Expression.parse("ccy = 'EUR'")
const restored: Expression = Expression.fromJson(filter.toJson())
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
const document: string = filter.toJson()
const text: string = filter.toString()
const same: boolean = filter.equals("ccy = 'EUR' and price > 100")
const both: Expression = named.and(constant)
const either: Expression = named.or("size > 1")
const negated: Expression = named.not()

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
const fromDocument: Statement = Statement.fromJson(statement.toJson())
const projections: Array<string> = statement.projections
const predicate: Expression | null = statement.predicate
const limit: number | null = statement.limit
const statementText: string = statement.toString()

const handle = new IOBase('file:///lake')
const matching: Array<IOBase> = handle.childrenMatching(filter)
const matchingText: Array<IOBase> = handle.childrenMatching("&holder.size > 0", true)

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
  never,
  output,
  parameters,
  parsed,
  predicate,
  projectedRows,
  projections,
  readsRows,
  restored,
  rows,
  same,
  schema,
  statement,
  statementText,
  stat,
  table,
  tasks,
  text,
}
