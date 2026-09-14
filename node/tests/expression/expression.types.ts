import {
  BatchReader,
  Bound,
  BoundSelector,
  Expression,
  Field,
  Filter,
  IOBase,
  Plan,
  Records,
  Selector,
  Scalar,
  Term,
  iceberg,
  type Table,
} from '../..'
import { type ScanPlanCounts } from '../../index'
import {
  tableFromArrays,
  type RecordBatch as ArrowRecordBatch,
  type Table as ArrowTable,
} from 'apache-arrow'

const term: Term = new Term("ccy = 'EUR' and price > 100")
const parsed: Term = Term.parse("ccy = 'EUR'")
const restored: Term = Term.fromJson(term.intoJson())
const named: Term = Term.column('ccy')
const constant: Term = Term.literal(Scalar.from('EUR'))
const typed: Term = Term.typedLiteral('int32', Scalar.from(5))
const held: Term = Term.attribute('partition', 'year')
const stat: Term = Term.attribute('size')
const late: Term = Term.parameter('floor')
const always: Term = Term.alwaysTrue()
const never: Term = Term.alwaysFalse()
const conjoined: Term = Term.all([named, "price > 1"])
const disjoined: Term = Term.any([named, "price > 1"])
const called: Term = Term.call('year', ['event'])

const columns: Array<string> = term.columns
const attributes: Array<string> = term.attributes
const parameters: Array<string> = term.parameters
const conjuncts: Array<Term> = term.conjuncts()
const depth: number = term.depth
const nodeCount: number = term.nodeCount
const isLiteral: boolean = constant.isLiteral
const asColumn: string | null = named.asColumn
const hasAttributes: boolean = held.hasAttributes
const document: string = term.intoJson()
const text: string = term.toString()
const same: boolean = term.equals("ccy = 'EUR' and price > 100")
const termClone: Term = term.clone()
const termOrder: number = term.compare(termClone)
const termHash: bigint = term.stableHash()
const termJson: unknown = term.toJSON()
const simplified: Term = term.simplify()
const explained: string = term.explain()
const both: Term = named.and(constant)
const either: Term = named.or("size > 1")
const negated: Term = named.not()
const compared: Term = named.comparison('is distinct from', "'EUR'")
const equal: Term = named.eq("'EUR'")
const unequal: Term = named.ne("'EUR'")
const below: Term = named.lt('1')
const atMost: Term = named.le('1')
const above: Term = named.gt('1')
const atLeast: Term = named.ge('1')
const within: Term = named.isIn(['1', '2'])
const between: Term = named.between('1', '2')
const isNull: Term = named.isNull()
const isNotNull: Term = named.isNotNull()
const like: Term = named.like("'A%'")
const ilike: Term = named.ilike("'a%'")
const glob: Term = named.glob("'*.parquet'")
const cast: Term = named.cast('int64')
const tryCast: Term = named.tryCast('int64')
const child: Term = named.child('leg')
const at: Term = named.at(0)
const sliced: Term = named.slice(1, null)
const keyed: Term = named.key(Scalar.from('k'))
const sumTerm: Term = Term.column('price').add('1')
const inferredSumTerm: Term = Term.column('price').add(1)
const nativeSumTerm: Term = Term.column('price').add(Scalar.from(1))
const differenceTerm: Term = Term.column('price').subtract('1')
const productTerm: Term = Term.column('price').multiply('2')
const quotientTerm: Term = Term.column('price').divide('2')
const remainderTerm: Term = Term.column('price').remainder('2')
const negativeTerm: Term = Term.column('price').negate()

const schema = new Field('trades', 'struct(field("ccy",utf8,nullable=true,metadata={}))', false)
const output: Field = named.field(schema)
const bound: Bound = named.bind(schema)
const boundLate: Bound = late.bind(schema, { floor: 2 })
const boundTerm: Term = bound.term
const boundField: Field = bound.field
const boundSchema: Field = bound.schema
const isPredicate: boolean = bound.isPredicate
const boundColumns: Array<string> = bound.columns
const boundIndices: Array<number> = bound.columnIndices
const readsRows: boolean = bound.readsRows
const answered: Scalar = bound.eval(Scalar.from(['EUR']))
const kept: boolean = bound.matches(Scalar.from(['EUR']))
const boundText: string = bound.toString()
const boundExplained: string = bound.explain()
const split: { answerable: Filter; remaining: Filter } = bound.partitionSplit()

const filter: Filter = new Filter("ccy = 'EUR'")
const filterFromTerm: Filter = new Filter(named.eq("'EUR'"))
const filterParsed: Filter = Filter.parse("ccy = 'EUR'")
const filterRestored: Filter = Filter.fromJson(filter.intoJson())
const filterTrue: Filter = Filter.alwaysTrue()
const filterFalse: Filter = Filter.alwaysFalse()
const filterAll: Filter = Filter.all([filter, 'price > 1'])
const filterAny: Filter = Filter.any([filter, 'price > 1'])
const filterTerm: Term = filter.term
const filterIsTrue: boolean = filter.isAlwaysTrue
const filterConjuncts: Array<Filter> = filter.conjuncts()
const filterBoth: Filter = filter.and('price > 1')
const filterNegated: Filter = filter.not()
const filterBound: Bound = filter.bind(schema)
const filterField: Field = filter.applyField(schema)
const filterExplained: string = filter.explain()
const filterJson: unknown = filter.toJSON()
const filterOrder: number = filter.compare(filter.clone())
const filterHash: bigint = filter.stableHash()

const selector: Selector = new Selector('ccy, price * 2 as doubled')
const selectorFromParts: Selector = new Selector(['ccy', 'price as amount'])
const selectorFromTerm: Selector = new Selector(named)
const selectorAll: Selector = Selector.all()
const selectorExcept: Selector = Selector.allExcept(['price'])
const selectorColumns: Selector = Selector.fromColumns(['ccy'])
const selectorFromField: Selector = Selector.fromField(schema)
const selectorNames: Array<string> = selector.names
const selectorProjections: Array<string> = selector.projections
const selectorExcluded: Array<string> = selectorExcept.excluded
const selectorIsAll: boolean = selectorAll.isAll
const selectorIsColumns: boolean = selectorColumns.isColumns
const selectorLength: number = selector.length
const selectorWith: Selector = selector.withProjection('size')
const selectorWithout: Selector = selector.withoutColumns(['ccy'])
const selectorField: Field = selector.applyField(schema)
const selectorStored: Field = selector.intoField(schema)
const boundSelector: BoundSelector = selector.bind(schema)
const boundSelectorSchema: Field = boundSelector.schema
const boundSelectorOutput: Field = boundSelector.output
const boundSelectorProjections: Array<Bound> = boundSelector.projections
const boundSelectorIdentity: boolean = boundSelector.isIdentity
const boundSelectorRow: Scalar = boundSelector.applyRow(Scalar.from(['EUR', 1]))
const selectorExplained: string = selector.explain()
const selectorJson: unknown = selector.toJSON()

const plan: Plan = new Plan("select ccy from t where ccy = 'EUR' order by ccy desc limit 10")
const emptyPlan: Plan = new Plan()
const planRestored: Plan = Plan.fromJson(plan.intoJson())
const planFromField: Plan = Plan.fromField(schema)
const planBuilt: Plan = new Plan()
  .withCreate('id int64 not null', 'trades')
  .withWrite('upsert into', "'file:///lake/trades.parquet'", ['id'])
  .withSelect(selector)
  .withSource('raw')
  .withFilter(filter)
  .withOrdering(['ccy desc nulls last', 'price'])
  .withLimit(10)
  .withOffset(1)
const planSelector: Selector = plan.selector
const planFilter: Filter = plan.filter
const planSource: string | null = plan.source
const planSourcePlan: Plan | null = plan.sourcePlan
const planOrdering = plan.ordering
const planDirection: 'asc' | 'desc' = planOrdering[0].direction
const planNulls: 'first' | 'last' = planOrdering[0].nulls
const planOrderTerm: Term = planOrdering[0].term
const planLimit: number | null = plan.limit
const planOffset: number | null = plan.offset
const planVerb: string | null = planBuilt.verb
const planTarget: string | null = planBuilt.writeTarget
const planSchema: Selector | null = planBuilt.schema
const planMergeBy: Selector = planBuilt.mergeBy
const planField: Field | null = planBuilt.field()
const planFieldFrom: Field = plan.readSections().fieldFrom(schema)
const planColumns: Array<string> = plan.columns
const planReadColumns: Array<string> | null = plan.readColumns
const planIsIdentity: boolean = plan.isIdentity
const planRead: Plan = plan.readSections()
const planExpression: Expression = plan.intoExpression()
const planExplained: string = plan.explain()
const planJson: unknown = plan.toJSON()
const planOrder: number = plan.compare(plan.clone())
const planHash: bigint = plan.stableHash()
const arrowTable: ArrowTable = tableFromArrays({ ccy: ['EUR'] })
const arrowBatch: ArrowRecordBatch = arrowTable.batches[0]
const appliedBatch: ArrowRecordBatch = planRead.applyArrowBatch(arrowBatch)
const appliedBatchInferred: ArrowRecordBatch = planRead.applyArrow(arrowBatch)
const appliedTable: ArrowTable = planRead.applyArrowTable(arrowTable)
const appliedTableInferred: ArrowTable = planRead.applyArrow(arrowTable)
const appliedReader: BatchReader = planRead.applyArrowReader(BatchReader.from(arrowTable))
const appliedReaderInferred: BatchReader = planRead.applyArrow(BatchReader.from(arrowTable))
const filteredBatch: ArrowRecordBatch = filter.applyArrowBatch(arrowBatch)
const selectedTable: ArrowTable = selector.applyArrowTable(arrowTable)
const boundSelected: BatchReader = boundSelector.applyArrowReader(BatchReader.from(arrowTable))
const boundFiltered: ArrowRecordBatch = filterBound.filterArrowBatch(arrowBatch)
const planRecords: Records = planRead.applyRecords([{ ccy: 'EUR' }])
const planRows: Scalar[] = planRecords.collect()
const recordsField: Field = planRecords.field
const recordsReader: BatchReader = filter.applyRecords([{ ccy: 'EUR' }], schema).intoArrowReader()
const recordsBack: Records = Records.fromArrowReader(BatchReader.from(arrowTable))
const executed: BatchReader = new Plan("select * from 'file:///lake/trades.arrow'").execute()

const expression: Expression = new Expression("select ccy where ccy = 'EUR'")
const expressionParsed: Expression = Expression.parse('select ccy')
const expressionRestored: Expression = Expression.fromJson(expression.intoJson())
const expressionSelect: Expression = Expression.select(selector)
const expressionWhere: Expression = Expression.filter(filter)
const expressionPlan: Expression = Expression.plan(plan)
const expressionSequence: Expression = Expression.sequence([expressionSelect, 'where ccy = 1'])
const expressionKind: 'select' | 'where' | 'plan' | 'sequence' = expression.kind
const expressionSteps: Array<Expression> = expressionSequence.steps
const expressionSelector: Selector | null = expressionSelect.asSelector()
const expressionFilter: Filter | null = expressionWhere.asFilter()
const expressionAsPlan: Plan | null = expression.asPlan()
const expressionColumns: Array<string> = expression.columns
const expressionField: Field = expressionSelect.applyField(schema)
const expressionBatch: ArrowRecordBatch = expression.applyArrowBatch(arrowBatch)
const expressionReader: BatchReader = expression.applyArrowReader(BatchReader.from(arrowTable))
const expressionRecords: Records = expression.applyRecords([{ ccy: 'EUR' }])
const expressionExplained: string = expression.explain()
const expressionJson: unknown = expression.toJSON()
const expressionOrder: number = expression.compare(expression.clone())
const expressionHash: bigint = expression.stableHash()

const handle = new IOBase('file:///lake')
const matching: Array<IOBase> = [...handle.childrenMatching(filter)]
const matchingTerm: Array<IOBase> = [...handle.childrenMatching(term)]
const matchingText: Array<IOBase> = [...handle.childrenMatching("&holder.size > 0", true)]

const table: Table = iceberg.Table.create('file:///lake/trades', schema, ['ccy'])
const rows: BatchReader = table.scanMatching(filter)
const projectedRows: BatchReader = table.scanMatching("ccy = 'EUR'", schema)
const counts: ScanPlanCounts = table.planMatching(term)
const tasks: number = counts.tasks
const manifestsSkipped: number = counts.manifestsSkipped

export {
  always,
  answered,
  appliedBatch,
  appliedBatchInferred,
  appliedReader,
  appliedReaderInferred,
  appliedTable,
  appliedTableInferred,
  asColumn,
  at,
  atLeast,
  atMost,
  attributes,
  above,
  below,
  between,
  both,
  bound,
  boundColumns,
  boundExplained,
  boundField,
  boundFiltered,
  boundIndices,
  boundLate,
  boundSchema,
  boundSelected,
  boundSelector,
  boundSelectorIdentity,
  boundSelectorOutput,
  boundSelectorProjections,
  boundSelectorRow,
  boundSelectorSchema,
  boundTerm,
  boundText,
  called,
  cast,
  child,
  columns,
  compared,
  conjoined,
  conjuncts,
  constant,
  counts,
  depth,
  disjoined,
  document,
  either,
  emptyPlan,
  equal,
  executed,
  explained,
  expression,
  expressionAsPlan,
  expressionBatch,
  expressionColumns,
  expressionExplained,
  expressionField,
  expressionFilter,
  expressionHash,
  expressionJson,
  expressionKind,
  expressionOrder,
  expressionParsed,
  expressionPlan,
  expressionReader,
  expressionRecords,
  expressionRestored,
  expressionSelect,
  expressionSelector,
  expressionSequence,
  expressionSteps,
  expressionWhere,
  filter,
  filterAll,
  filterAny,
  filterBoth,
  filterBound,
  filterConjuncts,
  filterExplained,
  filterFalse,
  filterField,
  filterFromTerm,
  filterHash,
  filterIsTrue,
  filterJson,
  filterNegated,
  filterOrder,
  filterParsed,
  filterRestored,
  filterTerm,
  filterTrue,
  filteredBatch,
  glob,
  hasAttributes,
  held,
  ilike,
  inferredSumTerm,
  isLiteral,
  isNotNull,
  isNull,
  isPredicate,
  kept,
  keyed,
  late,
  like,
  manifestsSkipped,
  matching,
  matchingTerm,
  matchingText,
  named,
  nativeSumTerm,
  negated,
  negativeTerm,
  never,
  nodeCount,
  output,
  parameters,
  parsed,
  plan,
  planBuilt,
  planColumns,
  planDirection,
  planExplained,
  planExpression,
  planField,
  planFieldFrom,
  planFilter,
  planFromField,
  planHash,
  planIsIdentity,
  planJson,
  planLimit,
  planMergeBy,
  planNulls,
  planOffset,
  planOrder,
  planOrderTerm,
  planOrdering,
  planRead,
  planReadColumns,
  planRecords,
  planRestored,
  planRows,
  planSchema,
  planSelector,
  planSource,
  planSourcePlan,
  planTarget,
  planVerb,
  productTerm,
  projectedRows,
  quotientTerm,
  readsRows,
  recordsBack,
  recordsField,
  recordsReader,
  remainderTerm,
  restored,
  rows,
  same,
  schema,
  selectedTable,
  selector,
  selectorAll,
  selectorColumns,
  selectorExcept,
  selectorExcluded,
  selectorExplained,
  selectorField,
  selectorFromField,
  selectorFromParts,
  selectorFromTerm,
  selectorIsAll,
  selectorIsColumns,
  selectorJson,
  selectorLength,
  selectorNames,
  selectorProjections,
  selectorStored,
  selectorWith,
  selectorWithout,
  simplified,
  sliced,
  split,
  stat,
  sumTerm,
  table,
  tasks,
  term,
  termClone,
  termHash,
  termJson,
  termOrder,
  text,
  tryCast,
  typed,
  unequal,
  within,
  differenceTerm,
}
