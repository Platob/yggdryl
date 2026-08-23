import {
  Field,
  MediaType,
  MimeType,
  ProtocolMetadata,
  Url,
  fields,
  intoField,
  type MetadataEntry,
} from '..'

class TypedRow {
  static get intoStructField(): Field {
    return fields.struct('TypedRow', [fields.int64('id')], { nullable: false })
  }
}
const classField: Field = intoField(TypedRow)
const instanceField: Field = intoField(new TypedRow())

class GetterRow {
  static get intoStructField(): Field {
    return fields.struct('GetterRow', [fields.int64('id')], { nullable: false })
  }
}
const getterField: Field = intoField(GetterRow, 'row')

class MethodRow {
  static intoStructField(): Field {
    return fields.struct('MethodRow', [], { nullable: false })
  }
}
// @ts-expect-error class metadata is a Field-valued getter, not a method
intoField(MethodRow)

const metadata: MetadataEntry[] = [{ key: 'source', value: 'book' }]
const field = new Field('id', 'bigint', false, metadata)
field.set('source', 'feed')
field.update({ venue: 'XPAR' })
field.update(metadata)
field.update(new Map([['session', 'regular']]))
const fieldHash: bigint = field.stableHash()
const fieldJson: unknown = field.toJSON()
const arrowField: Field = Field.fromArrow({
  toString: () => field.toString(),
})
const entries: Array<readonly [string, string]> = [...field]
const dictionaryId: bigint | null = field.dictionaryId
const dictionaryOrdered: boolean | null = field.dictionaryIsOrdered
field.setAlias('identifier')
field.setCatalogName('analytics')
field.setSchemaName('public')
field.setTableName('events')
field.setParquetFieldId(17)
field.setLocation('s3://warehouse/events/data.parquet')
field.setAccept('application/json')
field.setAcceptEncoding('gzip')
field.setAcceptLanguage('en')
field.setAcceptRanges('bytes')
field.setCacheControl('public')
field.setContentDisposition('attachment')
field.setContentEncoding('gzip')
field.setContentLanguage('en')
field.setContentLength(42n)
field.setContentLocation('../event')
field.setContentRange('bytes 0-9/10')
field.setContentType('application/json')
field.setMimeType(MimeType.JSON)
field.setMediaType(MediaType.fromParts(MimeType.JSON, [MimeType.GZIP]))
field.setEtag('"v1"')
field.setExpires('Sun, 16 Aug 2026 00:00:00 GMT')
field.setLastModified('Sat, 15 Aug 2026 00:00:00 GMT')
field.setHttpLocation('https://example.test/event')
field.setRange('bytes=0-9')
field.setVary('accept-encoding')
const alias: string | null = field.alias
const catalogName: string | null = field.catalogName
const schemaName: string | null = field.schemaName
const tableName: string | null = field.tableName
const id: number | null = field.parquetFieldId
const location: Url | null = field.location
const accept: string | null = field.accept
const contentLength: bigint | null = field.contentLength
const contentType: string | null = field.contentType
const mimeType: MimeType = field.mimeType
const mediaType: MediaType = field.mediaType
const httpLocation: Url | null = field.httpLocation
const previousProperty: string | null = field.setProperty('postgres', 'type', 'bigint')
const property: string | null = field.getProperty('postgres', 'type')
const hasProperty: boolean = field.hasProperty('postgres', 'type')
const properties: MetadataEntry[] = field.propertyIter('postgres')
field.clearProperties('postgres')

const iceberg: ProtocolMetadata = field.iceberg
const namedProtocols: ProtocolMetadata[] = [
  field.http,
  field.file,
  field.urn,
  field.postgres,
  field.postgresql,
  field.mysql,
  field.arrow,
  field.sql,
  field.glue,
  field.iceberg,
  field.fix,
  field.field,
  field.dtype,
  field.s3,
  field.gs,
  field.az,
  field.spark,
  field.polars,
  field.pandas,
]
const protocol: ProtocolMetadata = field.protocol('HTTPS')
const protocolScheme: string = protocol.scheme
const protocolPrefix: string = protocol.prefix
const protocolKey: string = protocol.key('content-type')
const protocolSize: number = iceberg.size
const protocolValue: string | null = iceberg.get('doc')
const protocolHas: boolean = iceberg.has('doc')
iceberg.set('doc', 'closing price')
iceberg.update({ 'schema-id': '3' })
iceberg.update(metadata)
iceberg.update(new Map([['field-id', '7']]))
const protocolNames: string[] = iceberg.keys()
const protocolValues: string[] = iceberg.values()
const protocolEntries: MetadataEntry[] = iceberg.entries()
const protocolPairs: Array<readonly [string, string]> = [...iceberg]
const protocolText: string = iceberg.toString()
const protocolJson: unknown = iceberg.toJSON()
const protocolDeleted: boolean = iceberg.delete('doc')
iceberg.clear()

const partitionRoot: Field = Field.from(
  'row: struct<year: int32 not null, price: float64 not null> not null',
).withPartitionFields(['year'])
const isPartition: boolean = partitionRoot.isPartition
const hasPartitionFields: boolean = partitionRoot.hasPartitionFields
const partitionFieldLen: number = partitionRoot.partitionFieldLen
const partitionFields: Field[] = partitionRoot.partitionFields()
const partitionFieldNames: string[] = partitionRoot.partitionFieldNames()
const onlyPartitionFields: Field = partitionRoot.onlyPartitionFields()
const withoutPartitionFields: Field = partitionRoot.withoutPartitionFields()
const withPartition: Field = partitionRoot.withPartition(false)
partitionRoot.setPartition(false)

void fieldHash
void fieldJson
void arrowField
void entries
void dictionaryId
void dictionaryOrdered
void alias
void catalogName
void schemaName
void tableName
void id
void location
void accept
void contentLength
void contentType
void mimeType
void mediaType
void httpLocation
void previousProperty
void property
void hasProperty
void properties
void namedProtocols
void protocol
void protocolScheme
void protocolPrefix
void protocolKey
void protocolSize
void protocolValue
void protocolHas
void protocolNames
void protocolValues
void protocolEntries
void protocolPairs
void protocolText
void protocolJson
void protocolDeleted
void isPartition
void hasPartitionFields
void partitionFieldLen
void partitionFields
void partitionFieldNames
void onlyPartitionFields
void withoutPartitionFields
void withPartition
