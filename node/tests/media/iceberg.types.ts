import { Table as ArrowTable } from 'apache-arrow'

import {
  BatchReader,
  DataType,
  Field,
  IOBase,
  MimeType,
  Url,
  Scalar,
  iceberg,
  type Catalog,
  type Compaction,
  type PartitionInput,
  type DataFile,
  type FieldBound,
  type FieldCount,
  type FieldSummaryView,
  type ManifestFile,
  type PartitionField,
  type PartitionSpec,
  type PartitionSpecDocument,
  type SchemaUpdate,
  type Snapshot,
  type SnapshotRef,
  type Table,
} from '../..'
import {
  IcebergOptions,
  type ScanPlan,
} from '../../index'

declare const schema: Field
declare const arrowTable: ArrowTable

const numbered: Field = iceberg.assignFieldIds(schema)
const renumbered: Field = iceberg.assignFieldIds(schema, 10)
const document: Scalar = iceberg.schemaIntoJson(numbered)
const parsed: Field = iceberg.schemaFromJson('row', document)

const unpartitioned: PartitionSpec = iceberg.PartitionSpec.unpartitioned()
const spec: PartitionSpec = iceberg.PartitionSpec.identity(numbered, ['venue'], 1)
const byName: PartitionInput = ['venue']
const specId: number = spec.specId
const specFields: PartitionField[] = spec.fields
const compactionClass = iceberg.Compaction
const manifestClass = iceberg.ManifestFile
const partitionFieldClass = iceberg.PartitionField
const snapshotClass = iceberg.Snapshot
const snapshotRefClass = iceberg.SnapshotRef
const flat: boolean = spec.isUnpartitioned()
const clonedSpec: PartitionSpec = spec.clone()
const equalSpec: boolean = spec.equals(clonedSpec)
const specOrder: number = spec.compare(clonedSpec)
const specHash: bigint = spec.stableHash()
const specDocument: PartitionSpecDocument = spec.intoJSON()
const parsedSpec: PartitionSpec = iceberg.PartitionSpec.fromJSON(specDocument)
const serializedSpec: PartitionSpecDocument = spec.toJSON()

void compactionClass
void manifestClass
void partitionFieldClass
void snapshotClass
void snapshotRefClass

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
const snapshot: Snapshot | null = created.currentSnapshot
const snapshots: Snapshot[] = created.snapshots
const manifests: ManifestFile[] = created.manifests()
const files: DataFile[] = created.dataFiles()
const printed: string = created.toString()

const scan: BatchReader = created.scan()
const projected: BatchReader = created.scan(numbered)
created.append(arrowTable)
created.overwrite(BatchReader.from(arrowTable))
const schemaId: number = created.evolveSchema(numbered)

declare const file: DataFile
const filePath: string = file.filePath
const fileMimeType: MimeType = file.mimeType
const content: number = file.content
const partition: Scalar[] = file.partition
const partitionNames: string[] = file.partitionNames
const recordCount: number = file.recordCount
const fileSize: number = file.fileSizeInBytes
const valueCounts: FieldCount[] = file.valueCounts
const nullCounts: FieldCount[] = file.nullValueCounts
const nanCounts: FieldCount[] = file.nanValueCounts
const columnSizes: FieldCount[] = file.columnSizes
const lower: FieldBound[] = file.lowerBounds
const upper: FieldBound[] = file.upperBounds
const keyMetadata: Buffer | null = file.keyMetadata
const splitOffsets: number[] = file.splitOffsets
const equalityIds: number[] | null = file.equalityIds
const sortOrderId: number | null = file.sortOrderId
const fileFirstRowId: bigint | null = file.firstRowId
const referencedDataFile: string | null = file.referencedDataFile
const contentOffset: number | null = file.contentOffset
const contentSizeInBytes: number | null = file.contentSizeInBytes
const clonedFile: DataFile = file.clone()
const equalFile: boolean = file.equals(clonedFile)
const fileOrder: number = file.compare(clonedFile)
const fileHash: bigint = file.stableHash()

declare const view: Snapshot
const snapshotId: bigint = view.snapshotId
const parentSnapshotId: bigint | null = view.parentSnapshotId
const timestampMs: number = view.timestampMs
const operation: string = view.operation
const summary: Record<string, string> = view.summary
const manifestList: string = view.manifestList
const v1Manifests: string[] | null = view.manifests
const snapshotSequence: number | null = view.sequenceNumber
const snapshotSchemaId: number | null = view.schemaId
const snapshotEncryptionKeyId: string | null = view.encryptionKeyId
const snapshotFirstRowId: bigint | null = view.firstRowId
const snapshotAddedRows: number | null = view.addedRows
const clonedSnapshot: Snapshot = view.clone()
const equalSnapshot: boolean = view.equals(clonedSnapshot)
const snapshotOrder: number = view.compare(clonedSnapshot)
const snapshotHash: bigint = view.stableHash()

declare const manifest: ManifestFile
const manifestPath: string = manifest.manifestPath
const addedSnapshotId: bigint = manifest.addedSnapshotId
const addedFilesCount: number | null = manifest.addedFilesCount
const existingFilesCount: number | null = manifest.existingFilesCount
const deletedFilesCount: number | null = manifest.deletedFilesCount
const addedRowsCount: number | null = manifest.addedRowsCount
const existingRowsCount: number | null = manifest.existingRowsCount
const deletedRowsCount: number | null = manifest.deletedRowsCount
const manifestContent: string = manifest.content
const manifestPartitions: FieldSummaryView[] = manifest.partitions
const manifestKeyMetadata: Buffer | null = manifest.keyMetadata
const manifestFirstRowId: bigint | null = manifest.firstRowId
const clonedManifest: ManifestFile = manifest.clone()
const equalManifest: boolean = manifest.equals(clonedManifest)
const manifestOrder: number = manifest.compare(clonedManifest)
const manifestHash: bigint = manifest.stableHash()

declare const partitionField: PartitionField
const partitionSourceId: number = partitionField.sourceId
const partitionFieldId: number = partitionField.fieldId
const partitionName: string = partitionField.name
const partitionTransform: string = partitionField.transform
const clonedPartitionField: PartitionField = partitionField.clone()
const equalPartitionField: boolean = partitionField.equals(clonedPartitionField)
const partitionFieldOrder: number = partitionField.compare(clonedPartitionField)
const partitionFieldHash: bigint = partitionField.stableHash()

const catalog: Catalog = new iceberg.Catalog('file:///lake/warehouse')
const fromHandle: Catalog = new iceberg.Catalog(new IOBase('file:///lake/warehouse'))
const rootTables = catalog.tables
const fromField: Table = rootTables.create('nyc.taxis', numbered)
const fromExpression: Table = rootTables.create(
  'nyc.taxis',
  'row: struct<id int64, venue utf8> not null',
)
const fromChildren: Table = rootTables.create('nyc.taxis', [schema, schema])
const openedByName: Table = catalog.table('nyc.taxis')
const present: boolean = rootTables.has('nyc.taxis')
const openedOrCreated: Table = rootTables.openOrCreate('nyc.taxis', numbered)
const appended: Table = catalog.append('nyc.taxis', arrowTable)
const replaced: Table = catalog.overwrite('nyc.taxis', BatchReader.from(arrowTable))
const namespaces: string[] = catalog.namespaces.names()
const nested: string[] = catalog.namespace('nyc').namespaces.names()
const tables: string[] = catalog.namespace('nyc').tables.names()
const catalogProperties: Record<string, string> = catalog.properties()
catalog.updateProperties({ owner: 'finance' })
catalog.updateProperties(new Map([['a', 'b']]), ['c'])
catalog.updateProperties()
const salesNamespace = catalog.namespace('sales')
const namespaceProperties: Record<string, string> = salesNamespace.properties()
salesNamespace.updateProperties({ team: 'emea' }, ['old'])

const atBigint: BatchReader = created.scanAt(1n)
const atNumber: BatchReader = created.scanAt(1)
const atFiltered: BatchReader = created.scanAt(1n, { venue: 'XNAS' })
const atMapped: BatchReader = created.scanAt(1n, new Map([['venue', 'XNAS']]), numbered)
const atProjected: BatchReader = created.scanAt(1n, [['venue', 'XNAS']], 'id: int64')

const byRef: Snapshot = created.snapshotByRef('main')
const target: number = created.targetFileSize
const compaction: Compaction = created.compact()
const filesBefore: number = compaction.filesBefore
const filesAfter: number = compaction.filesAfter
const bytesRewritten: number = compaction.bytesRewritten
const clonedCompaction: Compaction = compaction.clone()
const equalCompaction: boolean = compaction.equals(clonedCompaction)
const compactionOrder: number = compaction.compare(clonedCompaction)
const compactionHash: bigint = compaction.stableHash()
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

// Every native Iceberg value is also reachable through the curated namespace.
const options: IcebergOptions = new IcebergOptions({
  commitRetries: 4,
  commitMinBackoffMs: 100,
  commitMaxBackoffMs: 60_000,
  commitTotalTimeoutMs: 1_800_000,
  targetFileSize: 4096,
  readParallelism: 2,
  readParallelMinFiles: 16,
  readParallelMinFileSize: 4096,
  compactAfterCommits: 8,
  dataMimeType: MimeType.AVRO,
})
const puffinOptions: IcebergOptions = new IcebergOptions({ dataMimeType: 'puffin' })
const emptyOptions: IcebergOptions = new IcebergOptions()
const clonedOptions: IcebergOptions = options.clone()
const equalOptions: boolean = options.equals(clonedOptions)
const optionsOrder: number = options.compare(clonedOptions)
const optionsHash: bigint = options.stableHash()
options.commitRetries = 2
options.commitMinBackoffMs = 50
options.commitMaxBackoffMs = 5_000
options.commitTotalTimeoutMs = 30_000
options.targetFileSize = 8192
options.readParallelism = 1
options.readParallelMinFiles = 4
options.readParallelMinFileSize = 1024
options.compactAfterCommits = 3
options.dataMimeType = MimeType.PARQUET
const commitRetries: number = options.commitRetries
const commitMinBackoffMs: number = options.commitMinBackoffMs
const commitMaxBackoffMs: number = options.commitMaxBackoffMs
const commitTotalTimeoutMs: number = options.commitTotalTimeoutMs
const targetFileSizeOption: number = options.targetFileSize
const readParallelism: number = options.readParallelism
const readParallelMinFiles: number = options.readParallelMinFiles
const readParallelMinFileSize: number = options.readParallelMinFileSize
const compactAfterCommits: number | null = options.compactAfterCommits
const dataMimeType: MimeType = options.dataMimeType
const puffinMimeType: MimeType = puffinOptions.dataMimeType

created.setOptions(options)
const resolvedOptions: IcebergOptions = created.options()

const scannedWithOptions: BatchReader = created.scan(numbered, options)
const atWithOptions: BatchReader = created.scanAt(1n, null, numbered, options)
created.append(BatchReader.from(arrowTable), options)
created.overwrite(BatchReader.from(arrowTable), options)

const filtered: BatchReader = created.scanWhere({ venue: 'XNAS' })
const filteredProjected: BatchReader = created.scanWhere(
  [{ column: 'venue', value: 'XNAS' }],
  'row: struct<id int64> not null',
)
const onRef: BatchReader = created.scanRef('nightly')
const onRefFiltered: BatchReader = created.scanRef('nightly', { venue: 'XNAS' }, numbered)

const wholePlan: ScanPlan = created.plan()
const filteredPlan: ScanPlan = created.plan({ venue: 'XNAS' })
const pastPlan: ScanPlan = created.planAt(1n, { venue: 'XNAS' })
const plannedRecords: number = filteredPlan.recordCount
const plannedFiles: number = filteredPlan.filesPlanned
const skippedFiles: number = filteredPlan.filesSkipped
const readManifests: number = filteredPlan.manifestsRead
const skippedManifests: number = filteredPlan.manifestsSkipped
const clonedPlan: ScanPlan = filteredPlan.clone()
const plansEqual: boolean = filteredPlan.equals(clonedPlan)
const planOrder: number = filteredPlan.compare(wholePlan)
const planHash: bigint = filteredPlan.stableHash()

created.overwriteWhere({ venue: 'XNAS' }, BatchReader.from(arrowTable))
created.merge(BatchReader.from(arrowTable), ['id'])
created.merge(BatchReader.from(arrowTable), ['id'], false)
created.mergeWhere({ venue: 'XNAS' }, BatchReader.from(arrowTable), ['id'], true)

// The same five with a trailing per-call options value. Declaring the argument
// is the half a type checker can see: these three shipped accepting one at
// runtime that the generated types never mentioned, and nothing here called
// them that way, so `tsc` had no reason to object.
const filteredWithOptions: BatchReader = created.scanWhere({ venue: 'XNAS' }, numbered, options)
const onRefWithOptions: BatchReader = created.scanRef('nightly', { venue: 'XNAS' }, numbered, options)
created.overwriteWhere({ venue: 'XNAS' }, BatchReader.from(arrowTable), options)
created.merge(BatchReader.from(arrowTable), ['id'], false, options)
created.mergeWhere({ venue: 'XNAS' }, BatchReader.from(arrowTable), ['id'], true, options)

const expired: bigint[] = created.expireSnapshots()
const explicitlyExpired: bigint[] = created.expireSnapshots(1, 1, [1n, 2])
const pastManifests: ManifestFile[] = created.manifestsAt(1n)
created.createBranch('audit', 1n)
created.createTag('nightly', 1)
created.fastForward('audit', 1n)
const dropped: SnapshotRef = created.removeRef('nightly')
const droppedSnapshot: bigint = dropped.snapshotId
const droppedKind: string = dropped.kind
const droppedClone: SnapshotRef = dropped.clone()
const droppedEqual: boolean = dropped.equals(droppedClone)
const droppedOrder: number = dropped.compare(droppedClone)
const droppedHash: bigint = dropped.stableHash()

const namespacesView = catalog.namespaces
const namespaceNames: string[] = namespacesView.names()
const namespaceCount: number = namespacesView.size()
const hasNamespace: boolean = namespacesView.has('sales')
const sales = namespacesView.get('sales')
const salesName: string = sales.name
const madeNamespace = namespacesView.create('emea')
const eitherNamespace = namespacesView.openOrCreate('emea')
const nestedNamespaces = sales.namespaces
// The Map-like surface: lazy keys, values and entries opened one at a time.
const namespaceKeys: string[] = [...namespacesView.keys()]
const namespaceValues = [...namespacesView.values()]
const namespaceEntries: (readonly [string, unknown])[] = [...namespacesView.entries()]
const namespaceForOf: string[] = [...namespacesView]

const tablesView = sales.tables
const tableNames: string[] = tablesView.names()
const tableCount: number = tablesView.size()
const hasOrders: boolean = tablesView.has('orders')
const openedOrders: Table = tablesView.get('orders')
const madeTable: Table = tablesView.create('orders', numbered)
const eitherTable: Table = tablesView.openOrCreate(
  'orders',
  'row: struct<id int64> not null',
)
const appendedThroughView: Table = tablesView.append(
  'orders',
  BatchReader.from(arrowTable),
  options,
)
const replacedThroughView: Table = tablesView.overwrite(
  'orders',
  BatchReader.from(arrowTable),
)
const tableKeys: string[] = [...tablesView.keys()]
const tableValues: Table[] = [...tablesView.values()]
const tableEntries: (readonly [string, Table])[] = [...tablesView.entries()]
