export {
  AsciiEnum,
  BatchReader,
  Bound,
  BoundStatement,
  ByteIterator,
  DataType,
  Digest,
  Expression,
  Field,
  IOBase,
  IOCursor,
  Listing,
  MediaType,
  MimeType,
  ProtocolField,
  RecordOptions,
  Statement,
  TextOptions,
  Timezone,
  Uri,
  Url,
  Urn,
  Scalar,
  Xxh3,
  Xxh128,
  Xxh32,
  Xxh64,
  type FieldBound,
  type FieldCount,
  type FieldSummaryView,
  type MetadataEntry,
  type PartitionEntry,
  type TimezoneAlias,
} from './index'

import type {
  BatchReader,
  BoundStatement,
  ByteIterator,
  DataType,
  Digest,
  Field,
  IOBase,
  IOCursor,
  Listing,
  MediaType,
  MetadataEntry,
  MimeType,
  PartitionEntry,
  ProtocolField,
  RecordOptions,
  Statement,
  TextOptions,
  Timezone,
  Uri,
  Url,
  Urn,
  Scalar,
  Xxh3,
  Xxh128,
  Xxh32,
  Xxh64,
} from './index'
// The Iceberg and FIX values are reached through their namespaces, so they are
// imported here as values to type those and re-exported as types only.
import {
  Catalog,
  Compaction,
  DataFile,
  FixMsg,
  FixRegistry,
  IcebergOptions,
  ManifestFile,
  PartitionField,
  PartitionSpec,
  ScanPlan,
  Snapshot,
  SnapshotRef,
  Table,
} from './index'
import type {
  RecordBatch as ArrowRecordBatch,
  Table as ArrowTable,
  Vector as ArrowVector,
} from 'apache-arrow'
import type { Buffer } from 'node:buffer'
import type { URL as NodeURL } from 'node:url'

export type {
  Catalog,
  Compaction,
  DataFile,
  FixMsg,
  FixRegistry,
  IcebergOptions,
  ManifestFile,
  PartitionField,
  PartitionSpec,
  ScanPlan,
  Snapshot,
  SnapshotRef,
  Table,
}

/** A native MIME wrapper or canonical MIME/extension string. */
export type MimeTypeInput = MimeType | string
/** A native media/MIME wrapper or canonical media string. */
export type MediaTypeInput = MediaType | MimeType | string
/** A native handle, a native `Url`, or anything that names a location. */
export type LocationInput = IOBase | Url | string
/**
 * A class exposing its native struct shape through an actual static getter.
 *
 * TypeScript cannot distinguish an accessor from a readonly stored property;
 * {@link intoField} verifies the property descriptor at runtime.
 */
export interface StructFieldClass {
  readonly intoStructField: Field
}
/**
 * A class instance considered for its constructor's struct-field accessor.
 *
 * TypeScript does not retain the static side of a class on its instance type;
 * the converter therefore validates the static getter descriptor at runtime.
 */
export interface StructFieldInstance {
  readonly constructor: Function
  /** Prevent a constructor value from being mistaken for one of its instances. */
  readonly prototype?: never
}
/** Anything the global {@link intoField} converter accepts. */
export type FieldLike = Field | string | StructFieldClass | StructFieldInstance
/** Convert a native field, expression, or field class to one native `Field`. */
export declare function intoField(value: FieldLike, name?: string | null): Field
/** A native zone wrapper or an IANA name, alias, or fixed offset. */
export type TimezoneInput = Timezone | string
/** Hive partition pairs as a mapping, a Map, or an entry sequence. */
export type PartitionFilters =
  | ObjectMap<string, string>
  | ReadonlyMap<string, string>
  | Iterable<readonly [string, string]>
  | readonly PartitionEntry[]

/** Local mapped-object helper for declaring string-keyed object shapes. */
export type ObjectMap<K extends PropertyKey, V> = { [P in K]: V }

export type JsonLinesCodecFormat = 'json_lines' | 'json-lines' | 'jsonl' | 'ndjson'
export type TomlCodecFormat = 'toml'
export type SingleCodecFormat = 'json' | 'yaml' | 'yml' | TomlCodecFormat
export type CodecFormat = SingleCodecFormat | JsonLinesCodecFormat
export type JsonLinesPath = `${string}.jsonl` | `${string}.ndjson`
export type CodecContent =
  | string
  | Buffer
  | ArrayBuffer
  | SharedArrayBuffer
  | ArrayBufferView
export type CodecReadable = AsyncIterable<CodecContent>
/** String members are content; source locations are file URLs or descriptors. */
export type CodecSyncSource = CodecContent | number | NodeURL
export type CodecSource = CodecSyncSource | CodecReadable

/**
 * The parameter-free identity of one datatype variant.
 *
 * Mirrors `rust/src/datatype_id.rs` and is what `DataType.id` returns.
 * Use it whenever the question is "which variant is this".
 */
export type DataTypeId =
  | 'null'
  | 'boolean'
  | 'int8'
  | 'int16'
  | 'int32'
  | 'int64'
  | 'uint8'
  | 'uint16'
  | 'uint32'
  | 'uint64'
  | 'int128'
  | 'uint128'
  | 'float16'
  | 'float32'
  | 'float64'
  | 'datetime64'
  | 'date32'
  | 'date64'
  | 'time32'
  | 'time64'
  | 'duration32'
  | 'duration64'
  | 'interval'
  | 'binary'
  | 'fixed_size_binary'
  | 'large_binary'
  | 'binary_view'
  | 'utf8'
  | 'large_utf8'
  | 'utf8_view'
  | 'ascii'
  | 'fixed_ascii'
  | 'country'
  | 'currency'
  | 'mic'
  | 'cfi'
  | 'uuid'
  | 'list'
  | 'list_view'
  | 'fixed_size_list'
  | 'large_list'
  | 'large_list_view'
  | 'struct'
  | 'union'
  | 'dictionary'
  | 'decimal32'
  | 'decimal64'
  | 'decimal128'
  | 'decimal256'
  | 'map'
  | 'run_end_encoded'
  | 'variant'
  | 'geometry'
  | 'geography'

/**
 * The coarse family one datatype variant belongs to.
 *
 * Mirrors `rust/src/datatype_kind.rs` and is what `DataType.kind`
 * returns. Only behavior that is uniform across a whole family reads it.
 */
export type DataTypeKind =
  | 'null'
  | 'boolean'
  | 'integer'
  | 'floating'
  | 'decimal'
  | 'temporal'
  | 'text'
  | 'ascii'
  | 'bytes'
  | 'nested'
  | 'geospatial'
  | 'uuid'

interface DataTypeKindById {
  null: 'null'
  boolean: 'boolean'
  int8: 'integer'
  int16: 'integer'
  int32: 'integer'
  int64: 'integer'
  uint8: 'integer'
  uint16: 'integer'
  uint32: 'integer'
  uint64: 'integer'
  int128: 'integer'
  uint128: 'integer'
  float16: 'floating'
  float32: 'floating'
  float64: 'floating'
  datetime64: 'temporal'
  date32: 'temporal'
  date64: 'temporal'
  time32: 'temporal'
  time64: 'temporal'
  duration32: 'temporal'
  duration64: 'temporal'
  interval: 'temporal'
  binary: 'bytes'
  fixed_size_binary: 'bytes'
  large_binary: 'bytes'
  binary_view: 'bytes'
  utf8: 'text'
  large_utf8: 'text'
  utf8_view: 'text'
  ascii: 'ascii'
  fixed_ascii: 'ascii'
  country: 'ascii'
  currency: 'ascii'
  mic: 'ascii'
  cfi: 'ascii'
  uuid: 'uuid'
  list: 'nested'
  list_view: 'nested'
  fixed_size_list: 'nested'
  large_list: 'nested'
  large_list_view: 'nested'
  struct: 'nested'
  union: 'nested'
  dictionary: 'nested'
  decimal32: 'decimal'
  decimal64: 'decimal'
  decimal128: 'decimal'
  decimal256: 'decimal'
  map: 'nested'
  run_end_encoded: 'nested'
  variant: 'nested'
  geometry: 'geospatial'
  geography: 'geospatial'
}

/** The family a variant identity belongs to, as the native core reports it. */
export type DataTypeKindOf<K extends DataTypeId> = DataTypeKindById[K]

/** Core compatibility targets supported by DataType and Field projection. */
export type CompatibilityScheme =
  | 'arrow'
  | 'spark'
  | 'polars'
  | 'pandas'
  | 'iceberg'

/** Required intent for a generic record-write entry point. */
export type IOMode = 'overwrite' | 'append' | 'merge' | 'readonly' | 'random'

/** One field in the v2 Iceberg partition-spec JSON shape. */
export interface PartitionFieldDocument {
  name: string
  transform: string
  'source-id': number
  'field-id': number
}
/** The v2 Iceberg partition-spec JSON shape emitted by the native core. */
export interface PartitionSpecDocument {
  'spec-id': number
  fields: PartitionFieldDocument[]
}

declare const yggdrylHintValue: unique symbol

/** Cached runtime hint for one canonical JavaScript scalar view. */
export interface JSValueHint<
  K extends DataTypeId = DataTypeId,
  V = unknown,
> {
  /** The coarse family, exactly as `DataType.kind` reports it. */
  readonly kind: DataTypeKindOf<K>
  readonly constructor: Function | null
  readonly nullable: boolean
  /** Type-only carrier for the hinted canonical value. */
  readonly [yggdrylHintValue]?: V
}

declare module './index' {
  namespace PartitionSpec {
    /** Parse either the v1 field array or v2 object through the native core. */
    function fromJSON(value: unknown): PartitionSpec
  }

  interface PartitionSpec {
    /** Return the canonical v2 document as natural JavaScript data. */
    intoJSON(): PartitionSpecDocument
    /** Return the canonical document for JSON.stringify. */
    toJSON(): PartitionSpecDocument
  }

  interface Expression {
    /** Return the structural document for JavaScript JSON serialization. */
    toJSON(): unknown
    /** Build a lazy addition, inferring ordinary JavaScript values as literals. */
    add(other: unknown): Expression
    /** Build a lazy subtraction, inferring ordinary JavaScript values as literals. */
    subtract(other: unknown): Expression
    /** Build a lazy multiplication, inferring ordinary JavaScript values as literals. */
    multiply(other: unknown): Expression
    /** Build a lazy division, inferring ordinary JavaScript values as literals. */
    divide(other: unknown): Expression
    /** Build a lazy remainder, inferring ordinary JavaScript values as literals. */
    remainder(other: unknown): Expression
  }

  interface Statement {
    /** Return the structural document for JavaScript JSON serialization. */
    toJSON(): unknown
    /** Resolve every statement expression against one inferred native Field. */
    bind(
      schema: FieldLike,
      parameters?: Readonly<Record<string, unknown>> | Scalar | null,
    ): BoundStatement
  }

  interface BoundStatement {
    /** Lazily filter, project, and limit one native reader. */
    projectArrowReader(reader: BatchReader): BatchReader
    /** Filter and project one Apache Arrow JS RecordBatch. */
    projectArrowBatch(batch: ArrowRecordBatch): ArrowRecordBatch
    /** Filter, project, and limit one Apache Arrow JS Table. */
    projectArrowTable(table: ArrowTable): ArrowTable
    /** Infer an Arrow holder and preserve its runtime type. */
    projectArrow(reader: BatchReader): BatchReader
    projectArrow(batch: ArrowRecordBatch): ArrowRecordBatch
    projectArrow(table: ArrowTable): ArrowTable
    /** Sort one materialized batch by this statement's native ordering. */
    sortArrowBatch(batch: ArrowRecordBatch): ArrowRecordBatch
  }

  interface AsciiEnum {
    /**
     * Build the generated enum: a frozen object mapping each member name to
     * the code its ASCII value packs to under one width, tagged with the
     * enum's own name. It is name to code only, because a numeric reverse map
     * would collide with values that render as digits; `members` already
     * answers the name to value direction. A code reaches 128 bits at the
     * widest packable width, so every one of them is a `bigint`.
     */
    intoEnum(width: DataType | string): Readonly<Record<string, bigint>>
  }

  interface DataType {
    defaultJSValue(): unknown
    defaultJSHint(): JSValueHint
    defaultArrowScalar(): unknown
    intoSchemeCompat(target: CompatibilityScheme): DataType
  }

  interface FixRegistry {
    /** Walk the fields in ascending canonical-identifier order, lazily. */
    [Symbol.iterator](): Generator<Field>
  }

  interface FixMsg {
    /** Walk the root's `[name, value]` pairs in the order it declares. */
    [Symbol.iterator](): Generator<[string, Scalar]>
  }

  interface Field {
    defaultJSValue(): unknown
    defaultJSHint(): JSValueHint
    defaultArrowScalar(): unknown
    intoSchemeCompat(target: CompatibilityScheme): Field
    /**
     * Cast whatever Arrow JS holds - a Table, RecordBatch, BatchReader, or
     * IPC bytes - to this exact Field, batch by batch, as a Table.
     */
    castArrow(rows: BatchSource, options?: { safe?: boolean }): unknown
    /** The same cast under the generic name. */
    cast(rows: BatchSource, options?: { safe?: boolean }): unknown
  }
}

declare const yggdrylValueType: unique symbol
declare const yggdrylInputType: unique symbol

type NonNullableDataTypeValue<K extends DataTypeId, V> = K extends 'null'
  ? null
  : Exclude<V, null>

type TypedDefaultMethods<K extends DataTypeId, V> = {
  defaultJSValue(): V
  defaultJSHint(): JSValueHint<K, NonNullableDataTypeValue<K, V>>
  defaultArrowScalar(): unknown
}

/** A static variant/value view over the one native DataType runtime class. */
export type TypedDataType<
  K extends DataTypeId,
  V = unknown,
  I = DefaultFieldInput<K, V>,
> = Omit<DataType, keyof TypedDefaultMethods<K, V>> &
  TypedDefaultMethods<K, V> & {
  /** The variant identity, exactly as the native `id` getter reports it. */
  readonly id: K
  /** The coarse family, exactly as the native `kind` getter reports it. */
  readonly kind: DataTypeKindOf<K>
  readonly [yggdrylValueType]: V
  readonly [yggdrylInputType]: I
  readonly __yggdrylValueType?: V
  readonly __yggdrylInputType?: I
}

/** A static variant/value view over the one native Field runtime class. */
export type FieldOf<
  K extends DataTypeId,
  V = unknown,
  N extends string = string,
  I = DefaultFieldInput<K, V>,
> = Omit<Field, keyof TypedDefaultMethods<K, V> | 'name' | 'dtype'> &
  TypedDefaultMethods<K, V> & {
  readonly name: N
  readonly dtype: TypedDataType<
    K,
    NonNullableDataTypeValue<K, V>,
    NonNullableDataTypeValue<K, I>
  >
  readonly [yggdrylValueType]: V
  readonly [yggdrylInputType]: I
  readonly __yggdrylValueType?: V
  readonly __yggdrylInputType?: I
}

export type NullField = FieldOf<'null', null>
export type BooleanField = FieldOf<'boolean', boolean>
export type Int8Field = FieldOf<'int8', number>
export type Int16Field = FieldOf<'int16', number>
export type Int32Field = FieldOf<'int32', number>
export type Int64Field = FieldOf<'int64', bigint>
export type UInt8Field = FieldOf<'uint8', number>
export type UInt16Field = FieldOf<'uint16', number>
export type UInt32Field = FieldOf<'uint32', number>
export type UInt64Field = FieldOf<'uint64', bigint>
export type Float16Field = FieldOf<'float16', number>
export type Float32Field = FieldOf<'float32', number>
export type Float64Field = FieldOf<'float64', number>
export type DateTime64Field = FieldOf<'datetime64', bigint>
export type Date32Field = FieldOf<'date32', number>
export type Date64Field = FieldOf<'date64', bigint>
export type Time32Field = FieldOf<'time32', number>
export type Time64Field = FieldOf<'time64', bigint>
export type TimeField = Time32Field | Time64Field
export type Duration32Field = FieldOf<'duration32', number>
export type Duration64Field = FieldOf<'duration64', bigint>
export type IntervalValue =
  | number
  | readonly [days: number, milliseconds: number]
  | readonly [months: number, days: number, nanoseconds: bigint]
export type IntervalField = FieldOf<'interval', IntervalValue>
export type BinaryField = FieldOf<'binary', Uint8Array>
export type FixedSizeBinaryField = FieldOf<'fixed_size_binary', Uint8Array>
export type LargeBinaryField = FieldOf<'large_binary', Uint8Array>
export type BinaryViewField = FieldOf<'binary_view', Uint8Array>
export type Utf8Field = FieldOf<'utf8', string>
export type LargeUtf8Field = FieldOf<'large_utf8', string>
export type Utf8ViewField = FieldOf<'utf8_view', string>
/** Variable-width ASCII text: any length, stored as the bytes it is. */
export type AsciiField = FieldOf<'ascii', string>
/** ASCII text padded with trailing NUL to a fixed width; read back trimmed. */
export type FixedAsciiField = FieldOf<'fixed_ascii', string>
/** ISO 3166-1 alpha-2, the two-letter country code, in its own two bytes. */
export type CountryField = FieldOf<'country', string>
/** ISO 4217, the three-letter currency code, in its own three bytes. */
export type CurrencyField = FieldOf<'currency', string>
/** ISO 10383, the four-character market identifier code. */
export type MicField = FieldOf<'mic', string>
/** ISO 10962, the six-character instrument classification. */
export type CfiField = FieldOf<'cfi', string>
export type ListField<V = unknown> = FieldOf<'list', V[], string, unknown>
export type ListViewField<V = unknown> = FieldOf<'list_view', V[], string, unknown>
export type FixedSizeListField<V = unknown> = FieldOf<'fixed_size_list', V[], string, unknown>
export type LargeListField<V = unknown> = FieldOf<'large_list', V[], string, unknown>
export type LargeListViewField<V = unknown> = FieldOf<'large_list_view', V[], string, unknown>
export type StructField<V = readonly unknown[]> = FieldOf<'struct', V>
export type UnionValue<V = unknown, I extends number = number> = Readonly<{
  typeId: I
  value: V
}>
export type UnionField<V = UnionValue> = FieldOf<'union', V>
/** The dense Arrow Union with sequential type IDs, as one field type. */
export type DenseUnionField<V = UnionValue, I = V> = FieldOf<'union', V, string, I>
/** The self-describing semi-structured Variant datatype, as one field type. */
export type VariantField = FieldOf<'variant', unknown>
/** One 128-bit identifier; values read back as the hyphenated spelling. */
export type UuidField = FieldOf<'uuid', string>
/** A planar geometry column carrying Well-Known Binary payloads. */
export type GeometryField = FieldOf<'geometry', Uint8Array>
/** A geography column: WKB features on a sphere or spheroid. */
export type GeographyField = FieldOf<'geography', Uint8Array>
export type DictionaryField<V = unknown> = FieldOf<'dictionary', V>
export type Decimal32Field = FieldOf<'decimal32', bigint>
export type Decimal64Field = FieldOf<'decimal64', bigint>
export type Decimal128Field = FieldOf<'decimal128', bigint>
export type Decimal256Field = FieldOf<'decimal256', bigint>
export type DecimalField = Decimal128Field | Decimal256Field
export type MapField<K = unknown, V = unknown> = FieldOf<
  'map',
  ReadonlyMap<K, V>
>
export type RunEndEncodedField<V = unknown> = FieldOf<'run_end_encoded', V>

export type FieldMetadataInput =
  | Readonly<ObjectMap<string, string>>
  | ReadonlyMap<string, string>
  | ReadonlyArray<MetadataEntry | readonly [string, string]>
  | Iterable<readonly [string, string]>

export interface FieldOptions {
  nullable?: boolean
  metadata?: FieldMetadataInput
}

type TypedFieldValue<F extends Field> = F extends { readonly [yggdrylValueType]: infer V }
  ? V
  : unknown
type TypedFieldInput<F extends Field> = F extends { readonly [yggdrylInputType]: infer I }
  ? I
  : unknown
type TypedDataTypeValue<T> = T extends { readonly [yggdrylValueType]: infer V }
  ? V
  : unknown
type TypedDataTypeInput<T> = T extends { readonly [yggdrylInputType]: infer I }
  ? I
  : unknown
type FieldOptionsInput = FieldOptions | undefined
// Mirrors the runtime default in fields.js, which follows Python: a factory
// call that says nothing about nullability produces a nullable field, so the
// declared default value is null rather than a materialized zero.
type NullableSetting<O extends FieldOptionsInput> = O extends undefined
  ? true
  : 'nullable' extends keyof O
    ? O extends { nullable?: infer N }
      ? N
      : true
    : true
type NullableValue<V, O extends FieldOptionsInput> = true extends NullableSetting<O>
  ? V | null
  : V
type DefaultFieldInput<K extends DataTypeId, V> =
  K extends 'int8' | 'int16' | 'int32' | 'int64' | 'uint8' | 'uint16' | 'uint32' | 'uint64'
    ? number | bigint
    : K extends 'datetime64' | 'date32' | 'date64'
      ? number | bigint | Date
      : K extends 'time32' | 'time64' | 'duration32' | 'duration64'
        ? number | bigint
        : K extends 'decimal32' | 'decimal64' | 'decimal128'
          ? number | bigint
          : K extends 'binary' | 'fixed_size_binary' | 'large_binary' | 'binary_view'
            ? Uint8Array | ArrayBuffer
            : V
type NamedField<
  K extends DataTypeId,
  V,
  N extends string,
  O extends FieldOptionsInput,
  I = DefaultFieldInput<K, V>,
> = FieldOf<K, NullableValue<V, O>, N, NullableValue<I, O>>
type AnyField = FieldOf<DataTypeId, unknown, string, unknown>
type DataTypeInput = DataType | string

/** Datatype-specific factories returning the canonical native Field class. */
export interface FieldsNamespace {
  null(name: string, options?: FieldOptions): NullField
  boolean(name: string, options?: FieldOptions): BooleanField
  int8(name: string, options?: FieldOptions): Int8Field
  int16(name: string, options?: FieldOptions): Int16Field
  int32(name: string, options?: FieldOptions): Int32Field
  int64(name: string, options?: FieldOptions): Int64Field
  uint8(name: string, options?: FieldOptions): UInt8Field
  uint16(name: string, options?: FieldOptions): UInt16Field
  uint32(name: string, options?: FieldOptions): UInt32Field
  uint64(name: string, options?: FieldOptions): UInt64Field
  float16(name: string, options?: FieldOptions): Float16Field
  float32(name: string, options?: FieldOptions): Float32Field
  float64(name: string, options?: FieldOptions): Float64Field
  datetime64(
    name: string,
    unit?: string,
    timezone?: string,
    options?: FieldOptions,
  ): DateTime64Field
  datetime64(name: string, unit: string, options: FieldOptions): DateTime64Field
  date32(name: string, options?: FieldOptions): Date32Field
  date64(name: string, options?: FieldOptions): Date64Field
  time(name: string, unit: string, options?: FieldOptions): TimeField
  time32(name: string, unit?: string, options?: FieldOptions): Time32Field
  time32(name: string, options: FieldOptions): Time32Field
  time64(name: string, unit?: string, options?: FieldOptions): Time64Field
  time64(name: string, options: FieldOptions): Time64Field
  duration32(name: string, unit?: string, options?: FieldOptions): Duration32Field
  duration32(name: string, options: FieldOptions): Duration32Field
  duration64(name: string, unit?: string, options?: FieldOptions): Duration64Field
  duration64(name: string, options: FieldOptions): Duration64Field
  interval(name: string, unit?: string, options?: FieldOptions): IntervalField
  interval(name: string, options: FieldOptions): IntervalField
  binary(name: string, options?: FieldOptions): BinaryField
  fixedSizeBinary(
    name: string,
    byteWidth: number,
    options?: FieldOptions,
  ): FixedSizeBinaryField
  largeBinary(name: string, options?: FieldOptions): LargeBinaryField
  binaryView(name: string, options?: FieldOptions): BinaryViewField
  utf8(name: string, options?: FieldOptions): Utf8Field
  largeUtf8(name: string, options?: FieldOptions): LargeUtf8Field
  utf8View(name: string, options?: FieldOptions): Utf8ViewField
  ascii(name: string, options?: FieldOptions): AsciiField
  fixedAscii(name: string, width: number, options?: FieldOptions): FixedAsciiField
  list<F extends Field>(
    name: string,
    item: F,
    options?: FieldOptions,
  ): ListField<TypedFieldValue<F>>
  listView<F extends Field>(
    name: string,
    item: F,
    options?: FieldOptions,
  ): ListViewField<TypedFieldValue<F>>
  fixedSizeList<F extends Field>(
    name: string,
    item: F,
    length: number,
    options?: FieldOptions,
  ): FixedSizeListField<TypedFieldValue<F>>
  largeList<F extends Field>(
    name: string,
    item: F,
    options?: FieldOptions,
  ): LargeListField<TypedFieldValue<F>>
  largeListView<F extends Field>(
    name: string,
    item: F,
    options?: FieldOptions,
  ): LargeListViewField<TypedFieldValue<F>>
  struct(
    name: string,
    children: Iterable<Field>,
    options?: FieldOptions,
  ): StructField
  union(
    name: string,
    members: Iterable<readonly [number, Field]>,
    mode?: 'sparse' | 'dense',
    options?: FieldOptions,
  ): UnionField
  union(
    name: string,
    members: Iterable<readonly [number, Field]>,
    options: FieldOptions,
  ): UnionField
  denseUnion(
    name: string,
    members: Iterable<Field>,
    options?: FieldOptions,
  ): DenseUnionField
  variant(name: string, options?: FieldOptions): VariantField
  uuid(name: string, options?: FieldOptions): UuidField
  country(name: string, options?: FieldOptions): CountryField
  currency(name: string, options?: FieldOptions): CurrencyField
  mic(name: string, options?: FieldOptions): MicField
  cfi(name: string, options?: FieldOptions): CfiField
  geometry(name: string, crs?: string, options?: FieldOptions): GeometryField
  geometry(name: string, options: FieldOptions): GeometryField
  geography(
    name: string,
    crs?: string,
    algorithm?: string,
    options?: FieldOptions,
  ): GeographyField
  geography(name: string, crs: string, options: FieldOptions): GeographyField
  geography(name: string, options: FieldOptions): GeographyField
  dictionary(
    name: string,
    key: DataTypeInput,
    value: DataTypeInput,
    options?: FieldOptions,
  ): DictionaryField
  decimal(
    name: string,
    precision: number,
    scale?: number,
    options?: FieldOptions,
  ): DecimalField
  decimal(name: string, precision: number, options: FieldOptions): DecimalField
  decimal32(
    name: string,
    precision: number,
    scale?: number,
    options?: FieldOptions,
  ): Decimal32Field
  decimal32(
    name: string,
    precision: number,
    options: FieldOptions,
  ): Decimal32Field
  decimal64(
    name: string,
    precision: number,
    scale?: number,
    options?: FieldOptions,
  ): Decimal64Field
  decimal64(
    name: string,
    precision: number,
    options: FieldOptions,
  ): Decimal64Field
  decimal128(
    name: string,
    precision: number,
    scale?: number,
    options?: FieldOptions,
  ): Decimal128Field
  decimal128(
    name: string,
    precision: number,
    options: FieldOptions,
  ): Decimal128Field
  decimal256(
    name: string,
    precision: number,
    scale?: number,
    options?: FieldOptions,
  ): Decimal256Field
  decimal256(
    name: string,
    precision: number,
    options: FieldOptions,
  ): Decimal256Field
  map(
    name: string,
    entries: Field,
    keysSorted?: boolean,
    options?: FieldOptions,
  ): MapField
  map(name: string, entries: Field, options: FieldOptions): MapField
  mapOf(
    name: string,
    key: DataTypeInput,
    value: DataTypeInput,
    keysSorted?: boolean,
    options?: FieldOptions,
  ): MapField
  mapOf(
    name: string,
    key: DataTypeInput,
    value: DataTypeInput,
    options: FieldOptions,
  ): MapField
  runEndEncoded<F extends Field>(
    name: string,
    runEnds: Field,
    values: F,
    options?: FieldOptions,
  ): RunEndEncodedField<TypedFieldValue<F>>
}

/** A struct value is one ordered slot per child field, in declaration order. */
type StructValue<Fs extends readonly AnyField[]> = {
  readonly [I in keyof Fs]: Fs[I] extends Field ? TypedFieldValue<Fs[I]> : never
}
/** The matching ordered construction tuple for one literal Field tuple. */
type StructInputValue<Fs extends readonly AnyField[]> = {
  readonly [I in keyof Fs]: Fs[I] extends Field ? TypedFieldInput<Fs[I]> : never
}
type UnionMembersValue<Ms extends readonly (readonly [number, AnyField])[]> = {
  [P in keyof Ms]: Ms[P] extends readonly [infer I extends number, infer F extends AnyField]
    ? UnionValue<TypedFieldValue<F>, I>
    : never
}[number]
type UnionMembersInput<Ms extends readonly (readonly [number, AnyField])[]> = {
  [P in keyof Ms]: Ms[P] extends readonly [infer I extends number, infer F extends AnyField]
    ? UnionValue<TypedFieldInput<F>, I>
    : never
}[number]
type TuplePositions<T extends readonly unknown[]> = Exclude<
  keyof T,
  keyof readonly unknown[]
>
type TuplePositionNumber<P> = P extends `${infer I extends number}` ? I : never
type DenseUnionMembersValue<Fs extends readonly AnyField[]> =
  number extends Fs['length']
    ? UnionValue<TypedFieldValue<Fs[number]>, number>
    : {
        [P in TuplePositions<Fs>]: Fs[P] extends AnyField
          ? UnionValue<TypedFieldValue<Fs[P]>, TuplePositionNumber<P>>
          : never
      }[TuplePositions<Fs>]
type DenseUnionMembersInput<Fs extends readonly AnyField[]> =
  number extends Fs['length']
    ? UnionValue<TypedFieldInput<Fs[number]>, number>
    : {
        [P in TuplePositions<Fs>]: Fs[P] extends AnyField
          ? UnionValue<TypedFieldInput<Fs[P]>, TuplePositionNumber<P>>
          : never
      }[TuplePositions<Fs>]
type MapKeyValue<F extends Field> = TypedFieldValue<F> extends Readonly<{
  key: infer K
  value: infer V
}>
  ? readonly [K, V]
  : readonly [unknown, unknown]
type MapInputKeyValue<F extends Field> = TypedFieldInput<F> extends Readonly<{
  key: infer K
  value: infer V
}>
  ? readonly [K, V]
  : readonly [unknown, unknown]

/** Literal-name/nullability overloads that infer exact Field tuples. */
export interface FieldsNamespace {
  null<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'null', null, N, O>
  boolean<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'boolean', boolean, N, O>
  int8<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'int8', number, N, O>
  int16<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'int16', number, N, O>
  int32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'int32', number, N, O>
  int64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'int64', bigint, N, O>
  uint8<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'uint8', number, N, O>
  uint16<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'uint16', number, N, O>
  uint32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'uint32', number, N, O>
  uint64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'uint64', bigint, N, O>
  float16<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'float16', number, N, O>
  float32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'float32', number, N, O>
  float64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'float64', number, N, O>
  datetime64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, timezone?: string, options?: O): NamedField<'datetime64', bigint, N, O>
  datetime64<const N extends string, const O extends FieldOptionsInput>(name: N, unit: string, options: O): NamedField<'datetime64', bigint, N, O>
  date32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'date32', number, N, O>
  date64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'date64', bigint, N, O>
  time<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit: string, options?: O): NamedField<'time32', number, N, O> | NamedField<'time64', bigint, N, O>
  time32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'time32', number, N, O>
  time32<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'time32', number, N, O>
  time64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'time64', bigint, N, O>
  time64<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'time64', bigint, N, O>
  duration32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'duration32', number, N, O>
  duration32<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'duration32', number, N, O>
  duration64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'duration64', bigint, N, O>
  duration64<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'duration64', bigint, N, O>
  interval<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'interval', IntervalValue, N, O>
  interval<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'interval', IntervalValue, N, O>
  binary<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'binary', Uint8Array, N, O>
  fixedSizeBinary<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, byteWidth: number, options?: O): NamedField<'fixed_size_binary', Uint8Array, N, O>
  largeBinary<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'large_binary', Uint8Array, N, O>
  binaryView<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'binary_view', Uint8Array, N, O>
  utf8<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'utf8', string, N, O>
  largeUtf8<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'large_utf8', string, N, O>
  utf8View<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'utf8_view', string, N, O>
  ascii<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'ascii', string, N, O>
  fixedAscii<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, width: number, options?: O): NamedField<'fixed_ascii', string, N, O>
  list<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'list', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  listView<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'list_view', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  fixedSizeList<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, length: number, options?: O): NamedField<'fixed_size_list', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  largeList<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'large_list', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  largeListView<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'large_list_view', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  struct<const N extends string, const Fs extends readonly AnyField[], const O extends FieldOptionsInput = undefined>(name: N, children: Fs, options?: O): NamedField<'struct', StructValue<Fs>, N, O, StructInputValue<Fs>>
  union<const N extends string, const Ms extends readonly (readonly [number, AnyField])[], const O extends FieldOptionsInput = undefined>(name: N, members: Ms, mode?: 'sparse' | 'dense', options?: O): NamedField<'union', UnionMembersValue<Ms>, N, O, UnionMembersInput<Ms>>
  union<const N extends string, const Ms extends readonly (readonly [number, AnyField])[], const O extends FieldOptionsInput>(name: N, members: Ms, options: O): NamedField<'union', UnionMembersValue<Ms>, N, O, UnionMembersInput<Ms>>
  denseUnion<const N extends string, const Fs extends readonly AnyField[], const O extends FieldOptionsInput = undefined>(name: N, members: Fs, options?: O): NamedField<'union', DenseUnionMembersValue<Fs>, N, O, DenseUnionMembersInput<Fs>>
  variant<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'variant', unknown, N, O>
  uuid<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'uuid', string, N, O>
  country<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'country', string, N, O>
  currency<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'currency', string, N, O>
  mic<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'mic', string, N, O>
  cfi<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'cfi', string, N, O>
  geometry<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, crs?: string, options?: O): NamedField<'geometry', Uint8Array, N, O>
  geometry<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'geometry', Uint8Array, N, O>
  geography<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, crs?: string, algorithm?: string, options?: O): NamedField<'geography', Uint8Array, N, O>
  geography<const N extends string, const O extends FieldOptionsInput>(name: N, crs: string, options: O): NamedField<'geography', Uint8Array, N, O>
  geography<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'geography', Uint8Array, N, O>
  dictionary<const N extends string, K extends DataTypeInput, V extends DataTypeInput, const O extends FieldOptionsInput = undefined>(name: N, key: K, value: V, options?: O): NamedField<'dictionary', TypedDataTypeValue<V>, N, O, TypedDataTypeInput<V>>
  decimal<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, precision: number, scale?: number, options?: O): NamedField<'decimal128', bigint, N, O> | NamedField<'decimal256', bigint, N, O>
  decimal<const N extends string, const O extends FieldOptionsInput>(name: N, precision: number, options: O): NamedField<'decimal128', bigint, N, O> | NamedField<'decimal256', bigint, N, O>
  decimal32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, precision: number, scale?: number, options?: O): NamedField<'decimal32', bigint, N, O>
  decimal32<const N extends string, const O extends FieldOptionsInput>(name: N, precision: number, options: O): NamedField<'decimal32', bigint, N, O>
  decimal64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, precision: number, scale?: number, options?: O): NamedField<'decimal64', bigint, N, O>
  decimal64<const N extends string, const O extends FieldOptionsInput>(name: N, precision: number, options: O): NamedField<'decimal64', bigint, N, O>
  decimal128<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, precision: number, scale?: number, options?: O): NamedField<'decimal128', bigint, N, O>
  decimal128<const N extends string, const O extends FieldOptionsInput>(name: N, precision: number, options: O): NamedField<'decimal128', bigint, N, O>
  decimal256<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, precision: number, scale?: number, options?: O): NamedField<'decimal256', bigint, N, O>
  decimal256<const N extends string, const O extends FieldOptionsInput>(name: N, precision: number, options: O): NamedField<'decimal256', bigint, N, O>
  map<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, entries: F, keysSorted?: boolean, options?: O): NamedField<'map', ReadonlyMap<MapKeyValue<F>[0], MapKeyValue<F>[1]>, N, O, ReadonlyMap<MapInputKeyValue<F>[0], MapInputKeyValue<F>[1]>>
  map<const N extends string, F extends Field, const O extends FieldOptionsInput>(name: N, entries: F, options: O): NamedField<'map', ReadonlyMap<MapKeyValue<F>[0], MapKeyValue<F>[1]>, N, O, ReadonlyMap<MapInputKeyValue<F>[0], MapInputKeyValue<F>[1]>>
  mapOf<const N extends string, K extends DataTypeInput, V extends DataTypeInput, const O extends FieldOptionsInput = undefined>(name: N, key: K, value: V, keysSorted?: boolean, options?: O): NamedField<'map', ReadonlyMap<TypedDataTypeValue<K>, TypedDataTypeValue<V>>, N, O, ReadonlyMap<TypedDataTypeInput<K>, TypedDataTypeInput<V>>>
  mapOf<const N extends string, K extends DataTypeInput, V extends DataTypeInput, const O extends FieldOptionsInput>(name: N, key: K, value: V, options: O): NamedField<'map', ReadonlyMap<TypedDataTypeValue<K>, TypedDataTypeValue<V>>, N, O, ReadonlyMap<TypedDataTypeInput<K>, TypedDataTypeInput<V>>>
  runEndEncoded<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, runEnds: Field, values: F, options?: O): NamedField<'run_end_encoded', TypedFieldValue<F>, N, O, TypedFieldInput<F>>
}

export declare const fields: FieldsNamespace

/** One JSON value accepted as an Avro schema document. */
export type AvroSchemaDocument =
  | null
  | boolean
  | number
  | string
  | readonly AvroSchemaDocument[]
  | Readonly<{ [name: string]: AvroSchemaDocument }>

/** Any schema spelling normalized into the one native Avro schema graph. */
export type AvroSchemaInput =
  | AvroSchema
  | Scalar
  | CodecContent
  | AvroSchemaDocument

/** Binary input for an Avro container or single-object datum. */
export type AvroBytes = Exclude<CodecContent, string>

/** Native resource limits shared by every Avro decode path. */
export interface AvroDecodeLimits {
  /** Maximum structural nesting in a schema or datum. */
  maxDepth?: number | null
  /** Maximum encoded bytes consumed by one decode. */
  maxInputBytes?: number | null
  /** Maximum decoded nodes or object-container rows. */
  maxNodes?: number | null
}
/** Container/block decode options, including one optional reader schema. */
export interface AvroDecodeOptions extends AvroDecodeLimits {
  /** Resolve writer rows onto this reader schema while decoding. */
  readerSchema?: AvroSchemaInput | null
}

/** A parsed Avro schema backed by the native Rust schema graph. */
export interface AvroSchema {
  /** The root Avro kind, such as `record` or `long`. */
  readonly kind: string
  /** The Avro Parsing Canonical Form. */
  readonly canonicalForm: string
  /** The exact CRC-64-AVRO fingerprint of the canonical form. */
  readonly fingerprint: bigint
  /** Whether another schema retains the same behavior-affecting document. */
  equals(other: AvroSchema): boolean
  /** Compare schemas by the core's complete retained-schema order. */
  compare(other: AvroSchema): number
  /** Deterministic hash of the complete retained schema. */
  stableHash(): bigint
  /** Make a cheap native clone sharing the parsed schema graph. */
  clone(): AvroSchema
  /** Return the original schema document as natural JavaScript data. */
  intoJSON(): AvroSchemaDocument
  /** Return the Avro Parsing Canonical Form. */
  intoCanonicalForm(): string
  /** Encode one natural value with Avro single-object framing. */
  intoSingleObject(value: unknown): Buffer
  /** Decode one single-object datum through the shared native Scalar pivot. */
  fromSingleObject<T = unknown>(input: AvroBytes, options?: AvroDecodeLimits | null): T
  /** Return the canonical form for JavaScript's string protocol. */
  toString(): string
  /** Return the original document for JSON serialization. */
  toJSON(): AvroSchemaDocument
}

export declare const AvroSchema: {
  readonly prototype: AvroSchema
  /** Parse a natural value, native Scalar, JSON text, or JSON bytes. */
  new(value: AvroSchemaInput, options?: AvroDecodeLimits | null): AvroSchema
  /** Parse any accepted schema representation. */
  from(value: AvroSchemaInput, options?: AvroDecodeLimits | null): AvroSchema
}

/** One decoded Avro object container. */
export interface AvroContainer<T = unknown> {
  /** The writer schema carried in the object-container header. */
  schema: AvroSchema
  /** User metadata, excluding Avro's reserved header entries. */
  metadata: Record<string, string>
  /** Rows decoded through the shared native Scalar conversion. */
  rows: T[]
}

/** One compressed Avro object-container block, decoded only when requested. */
export interface AvroBlock<T = unknown> {
  /** Row count declared by the block header. */
  readonly count: bigint
  /** Compressed payload size in bytes. */
  readonly size: bigint
  /** Decompress and decode this block through the iterator's reader schema. */
  rows(): T[]
}

/** A fused lazy iterator retaining one bounded native read window. */
export interface AvroBlocks<T = unknown> extends IterableIterator<AvroBlock<T>> {
  /** Writer schema carried by the object-container header. */
  readonly schema: AvroSchema
  /** User metadata, excluding Avro's reserved header entries. */
  readonly metadata: Record<string, string>
  /** Return one metadata value without constructing the metadata object. */
  get(key: string): string | undefined
  next(): IteratorResult<AvroBlock<T>>
}

/** Raw native Avro schema, container, and single-object operations. */
export interface Avro {
  readonly Schema: typeof AvroSchema
  /** Lazily yield still-compressed object-container blocks. */
  blocks<T = unknown>(input: AvroBytes, options?: AvroDecodeOptions | null): AvroBlocks<T>
  loads<T = unknown>(input: AvroBytes, options?: AvroDecodeOptions | null): AvroContainer<T>
  dumps(
    rows: Iterable<unknown>,
    schema: AvroSchemaInput,
    metadata?: FieldMetadataInput | null,
  ): Buffer
  loadsSingle<T = unknown>(
    input: AvroBytes,
    schema: AvroSchemaInput,
    options?: AvroDecodeLimits | null,
  ): T
  dumpsSingle(value: unknown, schema: AvroSchemaInput): Buffer
}

/** Apache Avro operations backed entirely by the Rust core. */
export declare const avro: Avro

export interface CodecOptions {
  /** Explicit format; generic APIs otherwise infer a path suffix or content. */
  format?: SingleCodecFormat
  /** Native schema used by the core parser to type natural values. */
  field?: FieldLike
  /** Maximum recursive container depth in the inclusive range 1..48. */
  maxDepth?: number | null
  /** Maximum encoded bytes consumed by one decoder invocation. */
  maxInputBytes?: number | null
  /** Maximum scalar and container nodes decoded per document. */
  maxNodes?: number | null
  /** Maximum values yielded by a multi-document decoder. */
  maxDocuments?: number | null
  /**
   * Output indentation: omitted uses the format default, `null` requests no
   * layout, a number requests spaces per level, and `"\t"` requests tabs.
   */
  indent?: number | '\t' | null
  /** Return the exact native `Scalar` instead of its natural JavaScript view. */
  scalar?: boolean | null
}

/** Options for one type-inferred `IOBase.readRange` call. */
export interface RangeReadOptions {
  /** Decode the range as UTF-8 text instead of returning its bytes. */
  text?: boolean | null
}

/** Options for one format-inferred `IOBase.readScalar` call. */
export interface ScalarReadOptions {
  /** Native schema used by the core parser to type natural values. */
  field?: FieldLike | null
  /** Return the exact native `Scalar` instead of its natural JavaScript view. */
  scalar?: boolean | null
}

/** Page-cache controls for {@link IOBase.buffered}. */
export interface BufferedOptions {
  /** Bytes per page; rounded up to a power of two in the core. */
  pageSize?: number | null
  /** Total cached bytes; raised to hold the pinned first and last pages. */
  maxBytes?: number | null
  /** Milliseconds an untouched page remains cached. */
  ttlMs?: number | null
}

/**
 * The options of a format with `{{ }}` placeholder support: YAML and TOML.
 *
 * JSON is a data interchange format and refuses the pair by name, which is
 * why its methods take plain {@link CodecOptions}.
 */
export interface TemplateCodecOptions extends CodecOptions {
  /**
   * Resolve Jinja-style `{{ NAME }}` placeholders from this mapping.
   *
   * Substitution is off unless this or `environment` is set. A name nothing
   * resolves is an error naming it, never a silent empty string, and this
   * mapping wins over the environment.
   */
  placeholders?: Record<string, unknown> | ReadonlyMap<string, unknown> | null
  /**
   * Also resolve placeholders from the process environment.
   *
   * Its own switch, and off by default: with it off no environment variable is
   * read at all. A document that resolves a secret into a value that is then
   * dumped, logged, or written to a table has leaked it.
   */
  environment?: boolean
}

export interface JsonLinesCodecOptions extends Omit<CodecOptions, 'format'> {
  format: JsonLinesCodecFormat
}

/** The canonical resolution a native time, datetime, or duration counts in. */
export type CodecTimeUnit =
  | 's'
  | 'ms'
  | 'us'
  | 'ns'
  | 'year_month'
  | 'day_time'
  | 'month_day_nano'

export interface CodecNodeWritable {
  write(chunk: Uint8Array, callback?: (error?: Error | null) => void): unknown
  once?(event: 'error', callback: (error: Error) => void): unknown
  off?(event: 'error', callback: (error: Error) => void): unknown
}

export interface CodecWebWriter {
  readonly ready?: PromiseLike<void>
  write(chunk: Uint8Array): void | PromiseLike<void>
  releaseLock(): void
}

export interface CodecWebWritable {
  getWriter(): CodecWebWriter
}

export type CodecWritable = CodecNodeWritable | CodecWebWritable
export type CodecSyncDestination = string | number | NodeURL
export type CodecDestination = CodecSyncDestination | CodecWritable

/** One-document byte codec. Caller-owned streams are never closed. */
export interface SingleDocumentCodec<O extends CodecOptions = CodecOptions> {
  loads(content: CodecContent, options: O & { scalar: true }): Scalar
  loads<T = unknown>(content: CodecContent, options?: O): T
  load(source: CodecReadable, options: O & { scalar: true }): Promise<Scalar>
  load<T = unknown>(source: CodecReadable, options?: O): Promise<T>
  load(source: CodecSyncSource, options: O & { scalar: true }): Scalar
  load<T = unknown>(source: CodecSyncSource, options?: O): T
  dumps(value: unknown, options?: CodecOptions): Buffer
  dump(value: unknown, options?: CodecOptions): Buffer
  dump(value: unknown, destination: CodecWritable, options?: CodecOptions): Promise<void>
  dump(value: unknown, destination: CodecSyncDestination, options?: CodecOptions): void
  loadStream(
    stream: AsyncIterable<CodecContent>,
    options: O & { scalar: true },
  ): Promise<Scalar>
  loadStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options?: O,
  ): Promise<T>
  dumpStream(
    value: unknown,
    stream: CodecWritable,
    options?: CodecOptions,
  ): Promise<void>
}

/** One-document operations plus JSON Lines/YAML collection operations. */
export interface StructuredCodec<O extends CodecOptions = CodecOptions>
  extends SingleDocumentCodec<O> {
  loadsAll(content: CodecContent, options: O & { scalar: true }): Scalar[]
  loadsAll<T = unknown>(content: CodecContent, options?: O): T[]
  loadAll(source: CodecReadable, options: O & { scalar: true }): AsyncIterable<Scalar>
  loadAll<T = unknown>(source: CodecReadable, options?: O): AsyncIterable<T>
  loadAll(source: CodecSyncSource, options: O & { scalar: true }): Scalar[]
  loadAll<T = unknown>(source: CodecSyncSource, options?: O): T[]
  dumpAll(values: Iterable<unknown>, options?: CodecOptions): Buffer
  dumpAll(
    values: Iterable<unknown> | AsyncIterable<unknown>,
    destination: CodecWritable,
    options?: CodecOptions,
  ): Promise<void>
  dumpAll(
    values: Iterable<unknown>,
    destination: CodecSyncDestination,
    options?: CodecOptions,
  ): void
  loadAllStream(
    stream: AsyncIterable<CodecContent>,
    options: O & { scalar: true },
  ): AsyncIterable<Scalar>
  loadAllStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options?: O,
  ): AsyncIterable<T>
  dumpAllStream(
    values: Iterable<unknown> | AsyncIterable<unknown>,
    stream: CodecWritable,
    options?: CodecOptions,
  ): Promise<void>
}

export interface GenericCodec {
  from(
    source: CodecReadable,
    options: JsonLinesCodecOptions & { scalar: true },
  ): AsyncIterable<Scalar>
  from(
    source: CodecReadable,
    options: TemplateCodecOptions & { scalar: true },
  ): Promise<Scalar>
  from(
    source: CodecSyncSource,
    options: JsonLinesCodecOptions & { scalar: true },
  ): Scalar[]
  from(
    source: CodecSyncSource,
    options: TemplateCodecOptions & { scalar: true },
  ): Scalar
  from<T = unknown>(source: CodecReadable, options: JsonLinesCodecOptions): AsyncIterable<T>
  from<T = unknown>(source: CodecReadable, options?: TemplateCodecOptions): Promise<T>
  from<T = unknown>(source: CodecSyncSource, options: JsonLinesCodecOptions): T[]
  from<T = unknown>(source: CodecSyncSource, options?: TemplateCodecOptions): T
  into(
    values: Iterable<unknown>,
    options: JsonLinesCodecOptions,
  ): Buffer
  into(
    values: Iterable<unknown>,
    destination: JsonLinesPath,
    options?: Omit<CodecOptions, 'format'>,
  ): void
  into(value: unknown, options?: CodecOptions): Buffer
  into(
    values: Iterable<unknown> | AsyncIterable<unknown>,
    destination: CodecWritable,
    options: JsonLinesCodecOptions,
  ): Promise<void>
  into(
    values: Iterable<unknown>,
    destination: CodecSyncDestination,
    options: JsonLinesCodecOptions,
  ): void
  into(value: unknown, destination: CodecWritable, options?: CodecOptions): Promise<void>
  into(value: unknown, destination: CodecSyncDestination, options?: CodecOptions): void
  fromStream(
    stream: AsyncIterable<CodecContent>,
    options: JsonLinesCodecOptions & { scalar: true },
  ): AsyncIterable<Scalar>
  fromStream(
    stream: AsyncIterable<CodecContent>,
    options: TemplateCodecOptions & { scalar: true },
  ): Promise<Scalar>
  fromStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options: JsonLinesCodecOptions,
  ): AsyncIterable<T>
  fromStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options?: TemplateCodecOptions,
  ): Promise<T>
  intoStream(
    values: Iterable<unknown> | AsyncIterable<unknown>,
    stream: CodecWritable,
    options: JsonLinesCodecOptions,
  ): Promise<void>
  intoStream(
    value: unknown,
    stream: CodecWritable,
    options?: CodecOptions,
  ): Promise<void>
}

/** A Parquet footer key/value entry; order and duplicate keys are preserved. */
export interface ParquetKeyValue {
  key: string
  value: string
}

/** Axis-aligned bounds carried by Parquet geospatial statistics. */
export interface ParquetBoundingBox {
  xmin: number
  xmax: number
  ymin: number
  ymax: number
  zmin: number | null
  zmax: number | null
  mmin: number | null
  mmax: number | null
}

/** Bounds and sorted ISO geometry type codes for one WKB column. */
export interface ParquetGeospatialStatistics {
  bounding_box: ParquetBoundingBox | null
  geometry_types: number[]
}

/** Counts, encoded bounds, and optional geospatial data for one column chunk. */
export interface ParquetColumnStatistics {
  path: string
  compressed_size: number | bigint
  uncompressed_size: number | bigint
  null_count: number | bigint | null
  min_bytes: Buffer | null
  max_bytes: Buffer | null
  geospatial: ParquetGeospatialStatistics | null
}

/** Footer statistics for one row group, in file order. */
export interface ParquetRowGroupStatistics {
  num_rows: number | bigint
  compressed_size: number | bigint
  file_offset: number | bigint | null
  columns: ParquetColumnStatistics[]
}

/** Whole-file Parquet footer statistics, decoded through the native Scalar pivot. */
export interface ParquetFileStatistics {
  num_rows: number | bigint
  created_by: string | null
  key_value_metadata: ParquetKeyValue[]
  row_groups: ParquetRowGroupStatistics[]
}

/** Byte-first JSON codec; multi-value methods use JSON Lines. */
export declare const json: StructuredCodec
/** Byte-first single-document TOML codec using only natural TOML shapes. */
export declare const toml: SingleDocumentCodec<TemplateCodecOptions>
/** Byte-first YAML codec with tagged class comments and multi-document support. */
export declare const yaml: StructuredCodec<TemplateCodecOptions>

/**
 * The core's static enum vocabularies, as canonical spellings.
 *
 * Pure enums cross the boundary as strings by convention - a datatype id is
 * `'int64'`, a codec is `'gzip'` - and this is the frozen enumeration of what
 * those strings can be, unpacked from one native listing so it can never
 * drift from the Rust constants it mirrors.
 */
export declare const enums: {
  /** Every datatype variant identity, e.g. `'int64'`, `'decimal128'`. */
  readonly dataTypeIds: readonly DataTypeId[]
  /** Every datatype family, e.g. `'integer'`, `'decimal'`. */
  readonly dataTypeKinds: readonly DataTypeKind[]
  /** Every temporal resolution and interval layout, e.g. `'ms'`, `'year_month'`. */
  readonly timeUnits: readonly CodecTimeUnit[]
  /** Both union modes: `'sparse'` and `'dense'`. */
  readonly unionModes: readonly ('sparse' | 'dense')[]
  /** Every generic I/O intent. */
  readonly ioModes: readonly IOMode[]
  /** Every content coding, e.g. `'identity'`, `'gzip'`, `'zstd'`. */
  readonly codecs: readonly string[]
  /** Every answer a handle gives about what it addresses, e.g. `'file'`. */
  readonly ioKinds: readonly string[]
  /** The compatibility targets `intoSchemeCompat` accepts, e.g. `'arrow'`. */
  readonly compatibilitySchemes: readonly CompatibilityScheme[]
  /** Every digest algorithm, e.g. `'xxh3-64'`, `'xxh3-128'`. */
  readonly digestAlgorithms: readonly DigestAlgorithm[]
  /** The named points of the shared 0-to-9 compression scale. */
  readonly levels: {
    readonly none: number
    readonly fast: number
    readonly default: number
    readonly best: number
  }
}
/** Generic format-inferred byte codec. */
export declare const codec: GenericCodec

/** One xxHash algorithm, spelled the same way in every language. */
export type DigestAlgorithm = 'xxh32' | 'xxh64' | 'xxh3-64' | 'xxh3-128'

/** Anything a digest reads bytes from. */
export type DigestContent = Buffer | Uint8Array | ArrayBuffer | SharedArrayBuffer | string

/** How one digest call is seeded, and - for XXH3 - which secret it uses. */
export interface DigestOptions {
  /** The seed, as a bigint or an exact non-negative number. */
  readonly seed?: bigint | number
  /** A custom secret, at least `SECRET_MINIMUM_LENGTH` bytes. XXH3 only. */
  readonly secret?: DigestContent
}

/**
 * XXH32, XXH64, XXH3-64, and XXH3-128 over bytes, values, and handles.
 *
 * The one-shot functions answer a `number` for XXH32 - a 32-bit value always
 * fits one exactly - and a `bigint` for the wider results. `Digest` is the
 * answer that carries its algorithm with it, which is what keeps `xxh64` and
 * `xxh3-64`, both 64 bits wide, from being confused for one another.
 *
 * xxHash is not a cryptographic hash: a digest detects accidental change,
 * never an adversary who chooses the input. It is also not Iceberg's `bucket`
 * transform, which the specification pins to murmur3 x86_32.
 */
export declare const xxhash: {
  /** The shortest custom secret XXH3 accepts, in bytes. */
  readonly SECRET_MINIMUM_LENGTH: number
  readonly Digest: typeof Digest
  readonly Xxh32: typeof Xxh32
  readonly Xxh64: typeof Xxh64
  readonly Xxh3: typeof Xxh3
  readonly Xxh128: typeof Xxh128
  /** Digest a complete value with XXH32. */
  xxh32(data: DigestContent, options?: DigestOptions | bigint | number): number
  /** Digest a complete value with XXH64. */
  xxh64(data: DigestContent, options?: DigestOptions | bigint | number): bigint
  /** Digest a complete value with XXH3, answering 64 bits. */
  xxh3(data: DigestContent, options?: DigestOptions | bigint | number): bigint
  /** Digest a complete value with XXH3, answering 128 bits. */
  xxh128(data: DigestContent, options?: DigestOptions | bigint | number): bigint
  /** Digest a complete value, carrying the algorithm with the answer. */
  digest(data: DigestContent, algorithm: DigestAlgorithm): Digest
}

export interface ArrowStringCompatible {
  toString(): string
}

declare module './index' {
  /** A native MIME wrapper or canonical MIME/extension string. */
  type MimeTypeInput = MimeType | string
  /** A native media/MIME wrapper or canonical media string. */
  type MediaTypeInput = MediaType | MimeType | string

  interface DataType extends Iterable<Field> {
    showDiffs(other: DataType, withMetadata?: boolean): IterableIterator<string>
  }
  namespace DataType {
    function fromArrow(value: DataType | string | ArrowStringCompatible): DataType
    function fromFields(fields: Iterable<Field>): DataType
    // The parenthesis disambiguates: bare `variant()` is the self-describing
    // Variant datatype; `variant(fields)` stays the dense-union sugar.
    function variant(): TypedDataType<'variant', unknown>
    function variant<const Fs extends readonly AnyField[]>(
      fields: Fs,
    ): TypedDataType<
      'union',
      DenseUnionMembersValue<Fs>,
      DenseUnionMembersInput<Fs>
    >
    function variant(fields: Iterable<Field>): DataType
  }
  interface Field extends Iterable<readonly [string, string]> {
    showDiffs(other: Field, withMetadata?: boolean): IterableIterator<string>
    update(values: FieldMetadataInput): void
  }
  namespace Field {
    function fromArrow(value: Field | string | ArrowStringCompatible): Field
  }
  interface ProtocolField extends Iterable<readonly [string, string]> {
    update(values: FieldMetadataInput): void
  }
  namespace MimeType {
    const OCTET_STREAM: MimeType
    const JSON: MimeType
    const JSON_LINES: MimeType
    const YAML: MimeType
    const TOML: MimeType
    const CSV: MimeType
    const TSV: MimeType
    const PARQUET: MimeType
    const ARROW_FILE: MimeType
    const ARROW_STREAM: MimeType
    const AVRO: MimeType
    const ORC: MimeType
    const PUFFIN: MimeType
    const PLAIN_TEXT: MimeType
    const MARKDOWN: MimeType
    const HTML: MimeType
    const CSS: MimeType
    const JAVASCRIPT: MimeType
    const XML: MimeType
    const PDF: MimeType
    const CBOR: MimeType
    const MESSAGE_PACK: MimeType
    const PROTOBUF: MimeType
    const SQLITE3: MimeType
    const PNG: MimeType
    const JPEG: MimeType
    const GIF: MimeType
    const WEBP: MimeType
    const SVG: MimeType
    const MP3: MimeType
    const WAV: MimeType
    const OGG: MimeType
    const FLAC: MimeType
    const MP4: MimeType
    const WEBM: MimeType
    const WOFF: MimeType
    const WOFF2: MimeType
    const TTF: MimeType
    const OTF: MimeType
    const XLS: MimeType
    const XLSX: MimeType
    const ODS: MimeType
    const DOC: MimeType
    const DOCX: MimeType
    const GZIP: MimeType
    const ZSTD: MimeType
    const BROTLI: MimeType
    const ZLIB: MimeType
    const COMPRESS: MimeType
    const BZIP2: MimeType
    const XZ: MimeType
    const LZ4: MimeType
    const SNAPPY: MimeType
    const ZIP: MimeType
    const SEVEN_ZIP: MimeType
    const RAR: MimeType
    const TAR: MimeType
  }
  interface MediaType extends Iterable<MimeType> {
    /** Atomically replace encodings from one single-pass iterable. */
    setEncodings(values: Iterable<MimeTypeInput>): void
  }
  namespace MediaType {
    /** Build from a base MIME value and one single-pass iterable of encodings. */
    function fromParts(
      base: MimeTypeInput,
      encodings: Iterable<MimeTypeInput>,
    ): MediaType
    /** Infer from one single-pass iterable of compound filename extensions. */
    function fromExtensions(values: Iterable<string>): MediaType
  }
  interface Scalar extends Iterable<Scalar> {
    /** Add an inferred JavaScript or native numeric value in Rust. */
    add(other: unknown): Scalar
    /** Subtract an inferred JavaScript or native numeric value in Rust. */
    subtract(other: unknown): Scalar
    /** Multiply by an inferred JavaScript or native numeric value in Rust. */
    multiply(other: unknown): Scalar
    /** Divide by an inferred JavaScript or native numeric value in Rust. */
    divide(other: unknown): Scalar
    /** Take the numeric remainder after one inferred conversion. */
    remainder(other: unknown): Scalar
    /** Negate this numeric value in Rust. */
    negate(): Scalar
    /** Return this numeric value's checked absolute value in Rust. */
    absolute(): Scalar
    /** Number of direct sequence children, mapping entries, or record fields. */
    length: number
    /** Whether this is an empty sequence, mapping, or record. */
    isEmpty(): boolean
    /** Return one non-negative sequence index without projecting the child. */
    at(index: number): Scalar | null
    /** Read a sequence index, mapping key, or record field. */
    get(key: unknown): Scalar | null
    /** Whether a sequence index, mapping key, or record field resolves. */
    has(key: unknown): boolean
    /** Resolve a dotted mapping/record key and sequence-index path. */
    path(path: string): Scalar | null
    /** Return a mapping or record with one persistent replacement. */
    set(key: unknown, value: unknown): Scalar
    /** Return a mapping or record without one string key. */
    remove(key: string): Scalar
    /** The JavaScript spelling of this value, or the value itself when it has none. */
    asJs(options?: CodecOptions): unknown
    /** Infer the exact native Field for this scalar value. */
    intoField(): Field
    /** Infer the exact item Field for this non-empty outer sequence. */
    intoArrayField(): Field
    /** Infer a non-null Struct root from named record rows. */
    intoStructField(): Field
    /** Materialize this value as an Apache Arrow scalar. */
    intoArrowScalar(field?: Field): unknown
    /** Materialize this sequence as an Apache Arrow Vector. */
    intoArrowArray(field?: Field): ArrowVector
    /** Materialize record values as one Apache Arrow RecordBatch. */
    intoArrowBatch(field?: Field): ArrowRecordBatch
    /** Materialize record values as an Apache Arrow Table. */
    intoArrowTable(field?: Field): ArrowTable
  }
  namespace Scalar {
    /** Convert one JavaScript value into the native value it becomes. */
    function fromJs(value: unknown, options?: CodecOptions): Scalar
    /** Read one item from a one-item Apache Arrow Vector. */
    function fromArrowScalar(value: ArrowVector, field?: Field): Scalar
    /** Read an Apache Arrow Vector through native Arrow IPC. */
    function fromArrowArray(value: ArrowVector, field?: Field): Scalar
    /** Read an Apache Arrow RecordBatch through native Arrow IPC. */
    function fromArrowBatch(
      value: ArrowRecordBatch,
      field?: Field,
    ): Scalar
    /** Read an Apache Arrow Table through native Arrow IPC. */
    function fromArrowTable(value: ArrowTable, field?: Field): Scalar
  }
  interface Uri extends Iterable<string> {
    /** Join path components through the generic URI core. */
    joinPath(...others: (string | readonly string[])[]): Uri
  }
  interface Url extends Iterable<string> {
    /** Join path components onto this location, as `path.join`. */
    joinpath(...others: string[]): Url
  }
  interface Urn extends Iterable<string> {}

  /** A native handle, a native `Url`, or anything that names a location. */
  type LocationInput = IOBase | Url | string
  /** A caller-supplied Arrow-compatible file system as a plain object. */
  type FileSystemInput = FileSystemHandler
  /** A location, or the file system one of its locations sits on. */
  type LocationOrFileSystemInput = LocationInput | FileSystemHandler
  /** A partition spec, or the column names one would be built from. */
  type PartitionInput = PartitionSpec | readonly string[]
  /** A native zone wrapper or an IANA name, alias, or fixed offset. */
  type TimezoneInput = Timezone | string
  /** Hive partition pairs as a mapping, a Map, or an entry sequence. */
  type PartitionFilters =
    | ObjectMap<string, string>
    | ReadonlyMap<string, string>
    | Iterable<readonly [string, string]>
    | readonly PartitionEntry[]
  /** A table schema: a root `Field`, an expression, or the child `Field`s. */
  type TableSchemaInput = Field | string | readonly Field[]
  /** Any value the canonical `intoField` boundary accepts. */
  type FieldInput = FieldLike
  /** A native `DataType`, or the type expression naming one. */
  type DataTypeInput = DataType | string
  /** Declared root metadata: entries, a plain object, a Map, or a Field. */
  type MetadataInput = FieldMetadataInput
  /** The `bigint` a snapshot reports, or a number no larger than 2^53. */
  type SnapshotIdInput = bigint | number
  /** Scan filters: the same `(column, value)` pairs `childrenWhere` takes. */
  type ScanFilters = PartitionFilters
  /** Property updates as an object, a `Map`, or an entry sequence. */
  type PropertyUpdates = FieldMetadataInput

  /**
   * A listing is a JavaScript iterable and iterator at once, so `for...of`
   * walks it and `[...listing]` drains it. Nothing is collected on the way
   * across the boundary: the walk runs as the iterator is drained.
   */
  interface Listing extends Iterable<IOBase> {}

  /** A lazy, bounded byte stream exposed through JavaScript iteration. */
  interface ByteIterator extends IterableIterator<Buffer> {
    next(): IteratorResult<Buffer>
  }

  /** Iterating a handle lists its immediate children. */
  interface IOBase extends Iterable<IOBase>, Disposable {
    /** The storage role this handle currently addresses. */
    kind: string
    /** Whether this handle exposes either its byte or record surface. */
    isIo(): boolean
    /** Resolve a child of this resource, as `path.join`. */
    joinpath(...others: string[]): IOBase
    /** Leaves beneath this one carrying every requested partition pair. */
    childrenWhere(filters: PartitionFilters, includePrivate?: boolean): Listing
    /** Stream bounded byte arrays from `position`, without collecting them. */
    pstreamBytes(position?: number | null, batchSize?: number | null): ByteIterator
    /** Add or reconfigure one native page cache and return this handle. */
    buffered(options?: BufferedOptions | null): IOBase
    /** Retain flat plain-text record options and return this handle. */
    intoText(options?: TextOptions | null): IOBase
    /** Read `length` bytes from `offset` as bytes or as UTF-8 text. */
    readRange(
      offset: number,
      length: number,
      options: RangeReadOptions & { text: true },
    ): string
    readRange(
      offset: number,
      length: number,
      options?: RangeReadOptions | null,
    ): Buffer
    /** Append bytes or UTF-8 text after the last byte, returning its offset. */
    append(data: ArrayBufferView | ArrayBuffer | string): number
    /** Decode inferred JSON, YAML, or TOML, including its content coding. */
    readScalar(options: ScalarReadOptions & { scalar: true }): Scalar
    readScalar<T = unknown>(options?: ScalarReadOptions | FieldLike | null): T
    /** Encode one JavaScript or native `Scalar` through the inferred format. */
    writeScalar(value: unknown | Scalar): void
    /** Read a Parquet leaf's footer statistics without decoding rows. */
    readParquetStatistics(): ParquetFileStatistics
    /** Recompute one Parquet WKB column's bounds and geometry types. */
    readParquetGeospatialStatistics(
      column: string,
    ): ParquetGeospatialStatistics

    /** Read the canonical non-null struct root `Field` of this resource. */
    readArrowField(options?: RecordOptionsInput | null): Field
    /** Read this resource's rows, selecting and casting as the options say. */
    readArrowReader(options?: RecordOptionsInput | null): BatchReader
    /** Replace this resource's rows with one native reader. */
    overwriteArrowReader(reader: BatchReader, options?: RecordOptionsInput | null): void
    /** Append one native reader after this resource's rows. */
    appendArrowReader(reader: BatchReader, options?: RecordOptionsInput | null): void
    /** Merge one native reader by the non-empty `options.mergeByNames` keys. */
    mergeArrowReader(reader: BatchReader, options?: RecordOptionsInput | null): void
    /** Write one native reader using the required explicit mode. */
    writeArrowReader(
      reader: BatchReader,
      mode: IOMode,
      options?: RecordOptionsInput | null,
    ): void

    /** Replace this resource's rows with one Apache Arrow JS table. */
    overwriteArrowTable(table: ArrowTable, options?: RecordOptionsInput | null): void
    /** Append one Apache Arrow JS table after this resource's rows. */
    appendArrowTable(table: ArrowTable, options?: RecordOptionsInput | null): void
    /** Merge one Apache Arrow JS table by the non-empty `options.mergeByNames` keys. */
    mergeArrowTable(table: ArrowTable, options?: RecordOptionsInput | null): void
    /** Write one Apache Arrow JS table using the required explicit mode. */
    writeArrowTable(
      table: ArrowTable,
      mode: IOMode,
      options?: RecordOptionsInput | null,
    ): void

    /** Replace this resource's rows with one Apache Arrow JS record batch. */
    overwriteArrowBatch(
      batch: ArrowRecordBatch,
      options?: RecordOptionsInput | null,
    ): void
    /** Append one Apache Arrow JS record batch after this resource's rows. */
    appendArrowBatch(
      batch: ArrowRecordBatch,
      options?: RecordOptionsInput | null,
    ): void
    /** Merge one Apache Arrow JS record batch by `options.mergeByNames`. */
    mergeArrowBatch(
      batch: ArrowRecordBatch,
      options?: RecordOptionsInput | null,
    ): void
    /** Write one Apache Arrow JS record batch using the explicit mode. */
    writeArrowBatch(
      batch: ArrowRecordBatch,
      mode: IOMode,
      options?: RecordOptionsInput | null,
    ): void

    /**
     * Read this resource's rows as records: plain objects, or instances of
     * the class you pass, whose constructor receives one plain row.
     */
    readRecords<T = Record<string, unknown>>(
      options?: RecordOptionsInput | null,
    ): IterableIterator<T>
    readRecords<T>(
      cls: new (row: Record<string, unknown>) => T,
      options?: RecordOptionsInput | null,
    ): IterableIterator<T>
    /** Replace this resource's rows with plain objects or field-class instances. */
    overwriteRecords(
      rows: AsyncIterable<StructRecord>,
      options?: RecordOptionsInput | null,
    ): Promise<void>
    overwriteRecords(rows: RecordSource, options?: RecordOptionsInput | null): void
    /** Append plain objects or field-class instances after the stored rows. */
    appendRecords(
      rows: AsyncIterable<StructRecord>,
      options?: RecordOptionsInput | null,
    ): Promise<void>
    appendRecords(rows: RecordSource, options?: RecordOptionsInput | null): void
    /** Merge records by the non-empty `options.mergeByNames` keys. */
    mergeRecords(
      rows: AsyncIterable<StructRecord>,
      options?: RecordOptionsInput | null,
    ): Promise<void>
    mergeRecords(rows: RecordSource, options?: RecordOptionsInput | null): void
    /** Write records using the required explicit mode. */
    writeRecords(
      rows: AsyncIterable<StructRecord>,
      mode: IOMode,
      options?: RecordOptionsInput | null,
    ): Promise<void>
    writeRecords(
      rows: RecordSource,
      mode: IOMode,
      options?: RecordOptionsInput | null,
    ): void
  }

  interface IOCursor {
    /** Stream from this cursor and advance it only as chunks are yielded. */
    streamBytes(batchSize?: number | null): ByteIterator
  }

  /** Iterating a reader yields one Apache Arrow JS record batch at a time. */
  interface BatchReader extends Iterable<ArrowRecordBatch> {
    /** Drain every remaining batch into one Apache Arrow JS table. */
    intoTable(): ArrowTable
  }
  namespace BatchReader {
    /** Build a reader from a reader, an Arrow JS value, or Arrow IPC bytes. */
    function from(source: BatchSource, rootName?: string): BatchReader
  }

  /**
   * The catalog's names iterator is a JS iterable and iterator at once, so
   * `for...of` walks it and `[...keys]` drains it - nothing is collected on
   * the way across the boundary.
   */
  interface IcebergNames extends Iterable<string> {}

  /**
   * The collection views are Map-like: `for...of` yields the names lazily,
   * and `values`/`entries` open each named resource through `get`, one at a
   * time.
   */
  interface Namespaces extends Iterable<string> {
    values(): IterableIterator<Namespace>
    entries(): IterableIterator<readonly [string, Namespace]>
  }
  interface Tables extends Iterable<string> {
    values(): IterableIterator<Table>
    entries(): IterableIterator<readonly [string, Table]>
  }

  interface Table {
    /** Append rows as a new snapshot, keeping everything already stored. */
    append(rows: IcebergSource, options?: IcebergOptions | null): void
    /** Replace every row with `rows` as a new snapshot. */
    overwrite(rows: IcebergSource, options?: IcebergOptions | null): void
    /** Replace only the rows `filters` selects, keeping every other file. */
    overwriteWhere(
      filters: PartitionFilters | null | undefined,
      rows: IcebergSource,
      options?: IcebergOptions | null,
    ): void
    /** Merge `rows` into the stored rows, matching on `mergeByNames`. */
    merge(
      rows: IcebergSource,
      mergeByNames: readonly string[],
      safe?: boolean | null,
      options?: IcebergOptions | null,
    ): void
    /** Merge `rows` into the rows `filters` selects, on `mergeByNames`. */
    mergeWhere(
      filters: PartitionFilters | null | undefined,
      rows: IcebergSource,
      mergeByNames: readonly string[],
      safe?: boolean | null,
      options?: IcebergOptions | null,
    ): void
    /** Read the current snapshot, keeping the columns `field` names. */
    scan(field?: SchemaInput | null, options?: IcebergOptions | null): BatchReader
    /** Read the rows matching `filters`, keeping the columns `field` names. */
    scanWhere(
      filters?: PartitionFilters | null,
      field?: SchemaInput | null,
      options?: IcebergOptions | null,
    ): BatchReader
    /** Add a schema, make it current, and write a new metadata document. */
    evolveSchema(schema: SchemaInput): number
    /** Record a chain of column operations, committed as one new schema. */
    updateSchema(): SchemaUpdateBuilder
  }

  interface Tables {
    /** Append rows to the named table, creating it on first write. */
    append(name: string, rows: IcebergSource, options?: IcebergOptions | null): Table
    /** Replace the named table's rows, creating it on first write. */
    overwrite(name: string, rows: IcebergSource, options?: IcebergOptions | null): Table
  }

  interface Catalog {
    /** Append rows to the named table, creating it on first write. */
    append(name: string, rows: IcebergSource, options?: IcebergOptions | null): Table
    /** Replace the named table's rows, creating it on first write. */
    overwrite(name: string, rows: IcebergSource, options?: IcebergOptions | null): Table
  }

  namespace Timezone {
    /** Coordinated Universal Time, the one zone that is always registered. */
    const UTC: Timezone
  }
}

/**
 * Anything an Iceberg write takes: everything that names a stream of Arrow
 * record batches, plus the JavaScript rows the record surface accepts.
 */
export type IcebergSource = BatchSource | RecordSource

/** Anything that names a stream of Arrow record batches. */
export type BatchSource =
  | BatchReader
  | ArrowTable
  | ArrowRecordBatch
  | readonly ArrowRecordBatch[]
  | Buffer
  | Uint8Array
  | ArrayBuffer
/** One row accepted by the record-specific write entry points. */
// `& object` is what keeps a primitive out: every JavaScript value carries a
// `constructor`, so a bare number structurally satisfies StructFieldInstance.
export type StructRecord = Record<string, unknown> | (StructFieldInstance & object)
/** One record or a synchronous sequence of records. */
export type RecordSource = StructRecord | Iterable<StructRecord>
/** Native record settings, or the media type naming the encoding. */
export type RecordOptionsInput = RecordOptions | TextOptions | MediaTypeInput
/** A partition spec, or the column names one would be built from. */
export type PartitionInput = PartitionSpec | readonly string[]
/** A native root `Field`, or the field expression naming one. */
export type SchemaInput = Field | string
/** A table schema: a root `Field`, an expression, or the child `Field`s. */
export type TableSchemaInput = Field | string | readonly Field[]

/**
 * A chainable recording of column operations against a table's schema.
 *
 * `Table.updateSchema()` hands one out; each call records without touching
 * anything, and `commit()` replays the chain onto the schema the table has
 * then, adds the evolved schema, makes it current, and writes one new
 * metadata document, returning the new schema's identifier.
 */
export interface SchemaUpdate {
  /** Record a new column under `parent` - `''` names the root itself. */
  addColumn(parent: string, field: SchemaInput): SchemaUpdate
  /** Record the removal of the column at `path`, retiring its identifier. */
  dropColumn(path: string): SchemaUpdate
  /** Record a rename of the column at `path`; its identifier is kept. */
  renameColumn(path: string, name: string): SchemaUpdate
  /** Record a new `iceberg:doc` documentation string on the column at `path`. */
  updateDoc(path: string, doc: string): SchemaUpdate
  /** Record that the column at `path` becomes optional. */
  makeNullable(path: string): SchemaUpdate
  /** Record a type promotion on the column at `path`. */
  updateType(path: string, dtype: DataTypeInput): SchemaUpdate
  /** Replay the chain, make the evolved schema current, and commit once. */
  commit(): number
}
// The augmentation block below resolves bare `SchemaUpdate` against the
// generated module, so the chainable interface travels under an alias.
type SchemaUpdateBuilder = SchemaUpdate

/** `yggdryl::iceberg`: the table format, over the record encodings. */
export interface Iceberg {
  /** A warehouse folder of namespaces of Iceberg tables. */
  readonly Catalog: typeof Catalog
  /** A bounded report of a completed compaction. */
  readonly Compaction: typeof Compaction
  /** Per-call Iceberg commit, scan, and compaction settings. */
  readonly IcebergOptions: typeof IcebergOptions
  /** One manifest-list row under its complete core identity. */
  readonly ManifestFile: typeof ManifestFile
  /** One immutable field of a partition spec. */
  readonly PartitionField: typeof PartitionField
  /** An Iceberg table reached entirely through one container handle. */
  readonly Table: typeof Table
  /** How a table turns column values into the directories it writes. */
  readonly PartitionSpec: typeof PartitionSpec
  /** One live data file of a snapshot, with the spec that placed it. */
  readonly DataFile: typeof DataFile
  /** A bounded five-count report of scan pruning. */
  readonly ScanPlan: typeof ScanPlan
  /** One immutable table snapshot. */
  readonly Snapshot: typeof Snapshot
  /** One immutable branch or tag definition. */
  readonly SnapshotRef: typeof SnapshotRef
  /** Number every column of a schema, so a table can carry it. */
  assignFieldIds(schema: Field, start?: number): Field
  /** Throw the core message when a type change is not a legal promotion. */
  canPromote(fromType: DataTypeInput, toType: DataTypeInput): void
  /** Read an Iceberg schema document as a root `Field`. */
  schemaFromJson(name: string, document: Scalar | unknown): Field
  /** Write a root `Field` as an Iceberg schema document. */
  schemaIntoJson(schema: Field): Scalar
}

export declare const iceberg: Iceberg

/**
 * A message value: the native scalar, or the row `Scalar.fromJs` reads.
 *
 * A plain object is the obvious JavaScript spelling of a named row, and the
 * declared root Struct field is what orders, types and validates it - in the
 * core, as every other row is.
 */
export type FixValueInput = Scalar | Record<string, unknown> | readonly unknown[]

/**
 * The public `FixMsg` constructor, which widens the value the native class
 * takes: `Scalar.fromJs` lives in the loader, so it runs here.
 */
export interface FixMsgConstructor {
  /** Build a message, linking the process default when none is named. */
  new (
    field: Field,
    value: FixValueInput,
    registry?: FixRegistry | null,
  ): FixMsg
  readonly prototype: FixMsg
}

/** `yggdryl::fix`: the FIX dictionary, its message, and the process default. */
export interface Fix {
  /**
   * The FIX specification's own dictionary, and what an absent `fix:branch`
   * means: the branch every bare tag and every bare name resolves in.
   */
  readonly STANDARD_BRANCH: string
  /**
   * The first tag the FIX specification does not assign itself. Below it a tag
   * forces `STANDARD_BRANCH`, so no other dictionary may claim one.
   */
  readonly STANDARD_TAG_LIMIT: number
  /**
   * FIX field definitions resolved by identifier, by tag, by name, or by
   * dotted path.
   */
  readonly FixRegistry: typeof FixRegistry
  /** A FIX message: a value plus the registry that types it. */
  readonly FixMsg: FixMsgConstructor
  /** The process-wide registry, loading it on the first call. */
  globalRegistry(): FixRegistry
  /** Install the process-wide registry before anything resolves it. */
  installGlobalRegistry(registry: FixRegistry): void
}

export declare const fix: Fix

/** What an Arrow file system reports one path to be. */
export type ArrowFileKind = 'file' | 'directory' | 'unknown' | 'not-found'

/** What an Arrow file system handler reports about one path. */
export interface ArrowFileInfo {
  /**
   * The location, as the file system itself names it. A `fileInfo` answer
   * may omit it - the path that was asked about is what it means - while a
   * `list` entry stands for a location of its own and must name it.
   */
  path?: string
  /** `'file'`, `'directory'`, or `'unknown'` for a path holding nothing. */
  kind: ArrowFileKind
  /**
   * The byte length. Produce a `bigint`: a length is 64-bit, and an object
   * larger than 2^53 bytes is a real object. A `number` is read where it is
   * exact, so a handler over `fs.Stats` needs no conversion.
   */
  size?: bigint | number
}

/**
 * A file system Yggdryl reads and writes through, supplied by the caller.
 *
 * Arrow JS ships no file system, so this is the vtable `pyarrow.fs` already
 * implements, spelled in camelCase: implement it over a `Map`, `node:fs`, an
 * S3 client, or anything else, and `IOBase.fromFs` turns it into an
 * ordinary handle - globs, Hive partitions, IPC, Parquet, and Iceberg tables
 * included - with no code per backend.
 *
 * Every method is called synchronously, and only on the thread that supplied
 * the handler: a JavaScript value belongs to one isolate, so a handle built
 * from one cannot be read or written from a `Worker`. Absence is a normal
 * answer rather than a failure - a missing file reads nothing, a missing
 * directory lists empty, removing what is not there is done - and a handler
 * that throws instead is asked what is at the path before its failure is
 * surfaced, so `node:fs`'s own `ENOENT` needs no guarding.
 */
export interface FileSystemHandler {
  /** This file system's own name, and the scheme its locations carry. */
  readonly typeName?: string
  /** What is at `path` right now. */
  fileInfo(path: string): ArrowFileInfo
  /** Every entry under `path`; `recursive` descends. */
  list(path: string, recursive: boolean): readonly ArrowFileInfo[]
  /**
   * Bytes `[offset, offset + length)` of the file at `path`.
   *
   * `offset` is a `bigint` because a position in a file is 64-bit; `length`
   * is a number because it is the size of one read. Returning fewer bytes is
   * how the end of a file is reported, and nothing at all is how absence is.
   */
  readRange(path: string, offset: bigint, length: number): Uint8Array | null | undefined
  /** Replace the file at `path` with exactly `bytes`. */
  writeFull(path: string, bytes: Buffer): void
  /** Create the directory at `path`; an existing one is success. */
  createDir(path: string): void
  /** Remove the file at `path`; a missing one is success. */
  deleteFile(path: string): void
}
