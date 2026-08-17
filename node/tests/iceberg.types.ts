import { Table as ArrowTable } from 'apache-arrow'

import {
  BatchReader,
  DataType,
  Field,
  IOBase,
  Url,
  Value,
  iceberg,
  type Catalog,
  type Compaction,
  type PartitionInput,
  type DataFile,
  type FieldBound,
  type FieldCount,
  type ManifestFileView,
  type PartitionFieldView,
  type PartitionSpec,
  type SchemaUpdate,
  type SnapshotView,
  type Table,
} from '..'

declare const schema: Field
declare const arrowTable: ArrowTable

const numbered: Field = iceberg.assignFieldIds(schema)
const renumbered: Field = iceberg.assignFieldIds(schema, 10)
const document: Value = iceberg.schemaToJson(numbered)
const parsed: Field = iceberg.schemaFromJson('row', document)

const unpartitioned: PartitionSpec = iceberg.PartitionSpec.unpartitioned()
const spec: PartitionSpec = iceberg.PartitionSpec.identity(numbered, ['venue'], 1)
const byName: PartitionInput = ['venue']
const specId: number = spec.specId
const specFields: PartitionFieldView[] = spec.fields
const flat: boolean = spec.isUnpartitioned()

const created: Table = iceberg.Table.create('file:///lake/trades', numbered, spec)
const opened: Table = iceberg.Table.open(new IOBase('file:///lake/trades'))
const either: Table = iceberg.Table.openOrCreate(
  Url.fromString('file:///lake/trades'),
  numbered,
  byName,
  3,
)

const location: string = created.location
const uuid: string = created.tableUuid
const formatVersion: number = created.formatVersion
const version: number = created.version
const properties: Record<string, string> = created.properties
const metadataFileName: string = created.metadataFileName
const metadataLocation: string = created.metadataLocation
const current: Field = created.schema
const every: Field[] = created.schemas
const currentSpec: PartitionSpec = created.spec
const container: IOBase = created.root
const snapshot: SnapshotView | null = created.currentSnapshot
const snapshots: SnapshotView[] = created.snapshots
const manifests: ManifestFileView[] = created.manifests()
const files: DataFile[] = created.dataFiles()
const printed: string = created.toString()

const scan: BatchReader = created.scan()
const projected: BatchReader = created.scan(numbered)
created.append(arrowTable)
created.overwrite(BatchReader.from(arrowTable))
const schemaId: number = created.evolveSchema(numbered)

declare const file: DataFile
const filePath: string = file.filePath
const fileFormat: string = file.fileFormat
const content: number = file.content
const partition: Value[] = file.partition
const partitionNames: string[] = file.partitionNames
const recordCount: number = file.recordCount
const fileSize: number = file.fileSizeInBytes
const valueCounts: FieldCount[] = file.valueCounts
const nullCounts: FieldCount[] = file.nullValueCounts
const columnSizes: FieldCount[] = file.columnSizes
const lower: FieldBound[] = file.lowerBounds
const upper: FieldBound[] = file.upperBounds
const sortOrderId: number | null = file.sortOrderId

declare const view: SnapshotView
const snapshotId: bigint = view.snapshotId
const parentSnapshotId: bigint | undefined = view.parentSnapshotId
const timestampMs: number = view.timestampMs
const operation: string = view.operation
const summary: Record<string, string> = view.summary
const manifestList: string = view.manifestList

declare const manifest: ManifestFileView
const manifestPath: string = manifest.manifestPath
const addedSnapshotId: bigint = manifest.addedSnapshotId
const addedFilesCount: number = manifest.addedFilesCount
const manifestContent: string = manifest.content

const catalog: Catalog = new iceberg.Catalog('file:///lake/warehouse')
const fromHandle: Catalog = new iceberg.Catalog(new IOBase('file:///lake/warehouse'))
const fromField: Table = catalog.createTable('nyc.taxis', numbered)
const fromExpression: Table = catalog.createTable(
  'nyc.taxis',
  'row: struct<id int64, venue utf8> not null',
)
const fromChildren: Table = catalog.createTable('nyc.taxis', [schema, schema])
const openedByName: Table = catalog.table('nyc.taxis')
const present: boolean = catalog.hasTable('nyc.taxis')
const openedOrCreated: Table = catalog.openOrCreateTable('nyc.taxis', numbered)
const appended: Table = catalog.append('nyc.taxis', arrowTable)
const replaced: Table = catalog.overwrite('nyc.taxis', BatchReader.from(arrowTable))
const namespaces: string[] = catalog.listNamespaces()
const nested: string[] = catalog.listNamespaces('nyc')
const tables: string[] = catalog.listTables('nyc')

const atBigint: BatchReader = created.scanAt(1n)
const atNumber: BatchReader = created.scanAt(1)
const atFiltered: BatchReader = created.scanAt(1n, { venue: 'XNAS' })
const atMapped: BatchReader = created.scanAt(1n, new Map([['venue', 'XNAS']]), numbered)
const atProjected: BatchReader = created.scanAt(1n, [['venue', 'XNAS']], 'id: int64')

const byRef: SnapshotView = created.snapshotByRef('main')
const target: number = created.targetFileSize
const compaction: Compaction = created.compact()
const filesBefore: number = compaction.filesBefore
const filesAfter: number = compaction.filesAfter
const bytesRewritten: number = compaction.bytesRewritten
const history: BatchReader = created.inspectHistory()
const snapshotsReader: BatchReader = created.inspectSnapshots()
const filesReader: BatchReader = created.inspectFiles()

created.updateProperties({ 'commit.retry.num-retries': '4' })
created.updateProperties(new Map([['a', 'b']]), ['c'])
created.updateProperties(undefined, ['a'])
created.updateProperties()

const builder: SchemaUpdate = created.updateSchema()
const chained: SchemaUpdate = builder
  .addColumn('', 'price: float64')
  .addColumn('quote', schema)
  .dropColumn('venue')
  .renameColumn('id', 'tradeId')
  .updateDoc('tradeId', 'the identifier')
  .makeNullable('tradeId')
  .updateType('tradeId', DataType.from('int64'))
  .updateType('price', 'float64')
const committed: number = chained.commit()

iceberg.canPromote('int32', 'int64')
iceberg.canPromote(DataType.from('float32'), DataType.from('float64'))
