export {
  BatchReader,
  Bound,
  DataType,
  Expression,
  Field,
  IOBase,
  LineIterator,
  Listing,
  MediaType,
  MimeType,
  ProtocolMetadata,
  RecordOptions,
  Statement,
  Timezone,
  Uri,
  Url,
  Urn,
  Value,
  type Compaction,
  type FieldBound,
  type FieldCount,
  type ManifestFileView,
  type MetadataEntry,
  type PartitionEntry,
  type PartitionFieldView,
  type SnapshotView,
  type TimezoneAlias,
} from './index'

import type {
  BatchReader,
  DataType,
  Field,
  IOBase,
  LineIterator,
  Listing,
  MediaType,
  MetadataEntry,
  MimeType,
  PartitionEntry,
  ProtocolMetadata,
  RecordOptions,
  Timezone,
  Uri,
  Url,
  Urn,
  Value,
} from './index'
// The Iceberg values are reached through the `iceberg` namespace, so they are
// imported here as values to type it and re-exported as types only.
import { Catalog, DataFile, PartitionSpec, Table } from './index'
import type {
  RecordBatch as ArrowRecordBatch,
  Table as ArrowTable,
} from 'apache-arrow'
import type { Buffer } from 'node:buffer'
import type { URL as NodeURL } from 'node:url'

export type { Catalog, DataFile, PartitionSpec, Table }

/** A native MIME wrapper or canonical MIME/extension string. */
export type MimeTypeInput = MimeType | string
/** A native media/MIME wrapper or canonical media string. */
export type MediaTypeInput = MediaType | MimeType | string
/** A native handle, a native `Url`, or anything that names a location. */
export type LocationInput = IOBase | Url | string
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
export type CodecSyncSource = CodecContent | number | NodeURL
export type CodecSource = CodecSyncSource | CodecReadable

/**
 * The parameter-free identity of one datatype variant.
 *
 * Mirrors `rust/src/enums/datatype_id.rs` and is what `DataType.id` returns.
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
  | 'float16'
  | 'float32'
  | 'float64'
  | 'timestamp'
  | 'date32'
  | 'date64'
  | 'time32'
  | 'time64'
  | 'duration'
  | 'interval'
  | 'binary'
  | 'fixed_size_binary'
  | 'large_binary'
  | 'binary_view'
  | 'utf8'
  | 'large_utf8'
  | 'utf8_view'
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

/**
 * The coarse family one datatype variant belongs to.
 *
 * Mirrors `rust/src/enums/datatype_kind.rs` and is what `DataType.kind`
 * returns. Only behavior that is uniform across a whole family reads it.
 */
export type DataTypeKind =
  | 'null'
  | 'boolean'
  | 'integer'
  | 'floating'
  | 'decimal'
  | 'temporal'
  | 'binary'
  | 'string'
  | 'list'
  | 'struct'
  | 'union'
  | 'map'
  | 'dictionary'
  | 'run_end_encoded'

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
  float16: 'floating'
  float32: 'floating'
  float64: 'floating'
  timestamp: 'temporal'
  date32: 'temporal'
  date64: 'temporal'
  time32: 'temporal'
  time64: 'temporal'
  duration: 'temporal'
  interval: 'temporal'
  binary: 'binary'
  fixed_size_binary: 'binary'
  large_binary: 'binary'
  binary_view: 'binary'
  utf8: 'string'
  large_utf8: 'string'
  utf8_view: 'string'
  list: 'list'
  list_view: 'list'
  fixed_size_list: 'list'
  large_list: 'list'
  large_list_view: 'list'
  struct: 'struct'
  union: 'union'
  dictionary: 'dictionary'
  decimal32: 'decimal'
  decimal64: 'decimal'
  decimal128: 'decimal'
  decimal256: 'decimal'
  map: 'map'
  run_end_encoded: 'run_end_encoded'
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
  interface DataType {
    defaultJSValue(): unknown
    defaultJSHint(): JSValueHint
    defaultArrowScalar(): unknown
    toSchemeCompat(target: CompatibilityScheme): DataType
  }

  interface Field {
    defaultJSValue(): unknown
    defaultJSHint(): JSValueHint
    defaultArrowScalar(): unknown
    toSchemeCompat(target: CompatibilityScheme): Field
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
> = Omit<Field, keyof TypedDefaultMethods<K, V> | 'name' | 'dataType'> &
  TypedDefaultMethods<K, V> & {
  readonly name: N
  readonly dataType: TypedDataType<
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
export type TimestampField = FieldOf<'timestamp', bigint>
export type Date32Field = FieldOf<'date32', number>
export type Date64Field = FieldOf<'date64', bigint>
export type Time32Field = FieldOf<'time32', number>
export type Time64Field = FieldOf<'time64', bigint>
export type TimeField = Time32Field | Time64Field
export type DurationField = FieldOf<'duration', bigint>
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
/** A finite Variant is the existing dense Arrow Union field representation. */
export type VariantField<V = UnionValue, I = V> = FieldOf<'union', V, string, I>
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
    : K extends 'timestamp' | 'date32' | 'date64'
      ? number | bigint | Date
      : K extends 'time32' | 'time64' | 'duration'
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
  timestamp(
    name: string,
    unit?: string,
    timezone?: string,
    options?: FieldOptions,
  ): TimestampField
  timestamp(name: string, unit: string, options: FieldOptions): TimestampField
  date32(name: string, options?: FieldOptions): Date32Field
  date64(name: string, options?: FieldOptions): Date64Field
  time(name: string, unit: string, options?: FieldOptions): TimeField
  time32(name: string, unit?: string, options?: FieldOptions): Time32Field
  time32(name: string, options: FieldOptions): Time32Field
  time64(name: string, unit?: string, options?: FieldOptions): Time64Field
  time64(name: string, options: FieldOptions): Time64Field
  duration(name: string, unit?: string, options?: FieldOptions): DurationField
  duration(name: string, options: FieldOptions): DurationField
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
  variant(
    name: string,
    members: Iterable<Field>,
    options?: FieldOptions,
  ): VariantField
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
type VariantMembersValue<Fs extends readonly AnyField[]> =
  number extends Fs['length']
    ? UnionValue<TypedFieldValue<Fs[number]>, number>
    : {
        [P in TuplePositions<Fs>]: Fs[P] extends AnyField
          ? UnionValue<TypedFieldValue<Fs[P]>, TuplePositionNumber<P>>
          : never
      }[TuplePositions<Fs>]
type VariantMembersInput<Fs extends readonly AnyField[]> =
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
  timestamp<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, timezone?: string, options?: O): NamedField<'timestamp', bigint, N, O>
  timestamp<const N extends string, const O extends FieldOptionsInput>(name: N, unit: string, options: O): NamedField<'timestamp', bigint, N, O>
  date32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'date32', number, N, O>
  date64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'date64', bigint, N, O>
  time<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit: string, options?: O): NamedField<'time32', number, N, O> | NamedField<'time64', bigint, N, O>
  time32<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'time32', number, N, O>
  time32<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'time32', number, N, O>
  time64<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'time64', bigint, N, O>
  time64<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'time64', bigint, N, O>
  duration<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'duration', bigint, N, O>
  duration<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'duration', bigint, N, O>
  interval<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, unit?: string, options?: O): NamedField<'interval', IntervalValue, N, O>
  interval<const N extends string, const O extends FieldOptionsInput>(name: N, options: O): NamedField<'interval', IntervalValue, N, O>
  binary<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'binary', Uint8Array, N, O>
  fixedSizeBinary<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, byteWidth: number, options?: O): NamedField<'fixed_size_binary', Uint8Array, N, O>
  largeBinary<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'large_binary', Uint8Array, N, O>
  binaryView<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'binary_view', Uint8Array, N, O>
  utf8<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'utf8', string, N, O>
  largeUtf8<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'large_utf8', string, N, O>
  utf8View<const N extends string, const O extends FieldOptionsInput = undefined>(name: N, options?: O): NamedField<'utf8_view', string, N, O>
  list<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'list', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  listView<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'list_view', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  fixedSizeList<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, length: number, options?: O): NamedField<'fixed_size_list', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  largeList<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'large_list', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  largeListView<const N extends string, F extends Field, const O extends FieldOptionsInput = undefined>(name: N, item: F, options?: O): NamedField<'large_list_view', TypedFieldValue<F>[], N, O, TypedFieldInput<F>[]>
  struct<const N extends string, const Fs extends readonly AnyField[], const O extends FieldOptionsInput = undefined>(name: N, children: Fs, options?: O): NamedField<'struct', StructValue<Fs>, N, O, StructInputValue<Fs>>
  union<const N extends string, const Ms extends readonly (readonly [number, AnyField])[], const O extends FieldOptionsInput = undefined>(name: N, members: Ms, mode?: 'sparse' | 'dense', options?: O): NamedField<'union', UnionMembersValue<Ms>, N, O, UnionMembersInput<Ms>>
  union<const N extends string, const Ms extends readonly (readonly [number, AnyField])[], const O extends FieldOptionsInput>(name: N, members: Ms, options: O): NamedField<'union', UnionMembersValue<Ms>, N, O, UnionMembersInput<Ms>>
  variant<const N extends string, const Fs extends readonly AnyField[], const O extends FieldOptionsInput = undefined>(name: N, members: Fs, options?: O): NamedField<'union', VariantMembersValue<Fs>, N, O, VariantMembersInput<Fs>>
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

export interface CodecOptions {
  /** Explicit format; generic APIs otherwise infer a path suffix or content. */
  format?: SingleCodecFormat
  /** Maximum recursive container depth in the inclusive range 1..48. */
  maxDepth?: number
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

/** The canonical resolution a native time, timestamp, or duration counts in. */
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
export interface SingleDocumentCodec {
  loads<T = unknown>(content: CodecContent, options?: CodecOptions): T
  load<T = unknown>(source: CodecReadable, options?: CodecOptions): Promise<T>
  load<T = unknown>(source: CodecSyncSource, options?: CodecOptions): T
  dumps(value: unknown, options?: CodecOptions): Buffer
  dump(value: unknown, options?: CodecOptions): Buffer
  dump(value: unknown, destination: CodecWritable, options?: CodecOptions): Promise<void>
  dump(value: unknown, destination: CodecSyncDestination, options?: CodecOptions): void
  loadStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options?: CodecOptions,
  ): Promise<T>
  dumpStream(
    value: unknown,
    stream: CodecWritable,
    options?: CodecOptions,
  ): Promise<void>
}

/** One-document operations plus JSON Lines/YAML collection operations. */
export interface StructuredCodec extends SingleDocumentCodec {
  loadsAll<T = unknown>(content: CodecContent, options?: CodecOptions): T[]
  loadAll<T = unknown>(source: CodecReadable, options?: CodecOptions): AsyncIterable<T>
  loadAll<T = unknown>(source: CodecSyncSource, options?: CodecOptions): T[]
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
  loadAllStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options?: CodecOptions,
  ): AsyncIterable<T>
  dumpAllStream(
    values: Iterable<unknown> | AsyncIterable<unknown>,
    stream: CodecWritable,
    options?: CodecOptions,
  ): Promise<void>
}

export interface GenericCodec {
  from<T = unknown>(source: CodecReadable, options: JsonLinesCodecOptions): AsyncIterable<T>
  from<T = unknown>(source: CodecReadable, options?: CodecOptions): Promise<T>
  from<T = unknown>(source: CodecSyncSource, options: JsonLinesCodecOptions): T[]
  from<T = unknown>(source: CodecSyncSource, options?: CodecOptions): T
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
  fromStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options: JsonLinesCodecOptions,
  ): AsyncIterable<T>
  fromStream<T = unknown>(
    stream: AsyncIterable<CodecContent>,
    options?: CodecOptions,
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

/** Byte-first JSON codec; multi-value methods use JSON Lines. */
export declare const json: StructuredCodec
/** Byte-first single-document TOML codec with collision-safe typed envelopes. */
export declare const toml: SingleDocumentCodec
/** Byte-first YAML codec with tagged class comments and multi-document support. */
export declare const yaml: StructuredCodec
/** Generic format-inferred byte codec. */
export declare const codec: GenericCodec

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
    function variant<const Fs extends readonly AnyField[]>(
      fields: Fs,
    ): TypedDataType<
      'union',
      VariantMembersValue<Fs>,
      VariantMembersInput<Fs>
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
  interface ProtocolMetadata extends Iterable<readonly [string, string]> {
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
  interface Value {
    /** The JavaScript spelling of this value, or the value itself when it has none. */
    asJs(options?: CodecOptions): unknown
  }
  namespace Value {
    /** Convert one JavaScript value into the native value it becomes. */
    function fromJs(value: unknown, options?: CodecOptions): Value
  }
  interface Uri extends Iterable<string> {}
  interface Url extends Iterable<string> {
    /** Join path components onto this location, as `path.join`. */
    joinpath(...others: string[]): Url
  }
  interface Urn extends Iterable<string> {}

  /** A native handle, a native `Url`, or anything that names a location. */
  type LocationInput = IOBase | Url | string
  /** A caller-supplied Arrow file system: the vtable as a plain object. */
  type ArrowFileSystemInput = ArrowFileSystemHandler
  /** A location, or the file system one of its locations sits on. */
  type LocationOrFileSystemInput = LocationInput | ArrowFileSystemHandler
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
  /** A native root `Field`, or the field expression naming one. */
  type FieldInput = Field | string
  /** A native `DataType`, or the type expression naming one. */
  type DataTypeInput = DataType | string
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

  /** Iterating a handle lists its immediate children. */
  interface IOBase extends Iterable<IOBase>, Disposable {
    /** Resolve a child of this resource, as `path.join`. */
    joinpath(...others: string[]): IOBase
    /** Leaves beneath this one carrying every requested partition pair. */
    childrenWhere(filters: PartitionFilters, includePrivate?: boolean): Listing

    /** Read the canonical non-null struct root `Field` of this resource. */
    readArrowField(options?: RecordOptionsInput | null): Field
    /** Read this resource's rows, selecting and casting as the options say. */
    readArrowBatchReader(options?: RecordOptionsInput | null): BatchReader
    /**
     * Iterate this resource's text records, one at a time.
     *
     * Without options every line is a record. `logs: true` starts a record at
     * every line carrying a leading timestamp, and a `pattern` starts one at
     * every line the expression matches - each record carrying the lines that
     * follow it until the next one starts. Any content coding the name
     * declares decodes as a stream, so a `.log.gz` costs one window rather
     * than its decompressed size.
     */
    readLines(pattern: string, options?: TextLineInput | null): LineIterator
    readLines(options?: TextLineInput | null): LineIterator
    /**
     * Project this resource's text records into a `BatchReader`.
     *
     * A text-line surface beside `readLines`, never a record method: each
     * record becomes one typed row, with one nullable column per named
     * capture group and the constant `customFields` columns after them. A
     * capture whose whole sub-pattern is one of the closed inference table's
     * exact spellings types itself - `(?<threadId>\d+)` is `int64` - and
     * `captureTypes` declares the rest (a native `DataType` or a
     * type-expression string), parsed strictly: a captured text the datatype
     * cannot read is an error, never a silent null. A batch closes on
     * whichever bound trips first, `byteSize` or `batchSize`.
     * `schemaFromPattern` answers the same schema without a reader. The
     * boundary is the standard copied IPC one, never zero-copy.
     */
    readArrowLines(pattern: string, options?: TextLineInput | null): BatchReader
    readArrowLines(options?: TextLineInput | null): BatchReader
    /**
     * Replace this resource's records with `lines`, each terminated.
     *
     * Streaming: the iterable is pulled one record at a time and never
     * collected, so a million-record write costs one reused buffer. `linesep`
     * unset writes the platform-neutral `\n`.
     */
    writeLines(
      lines: Iterable<string | Uint8Array>,
      options?: TextLineInput | null,
    ): void
    /** Append `lines` after this resource's end, streaming as `writeLines` does. */
    appendLines(
      lines: Iterable<string | Uint8Array>,
      options?: TextLineInput | null,
    ): void
    /** Replace or merge this resource's rows with every batch `batches` yields. */
    writeArrowBatchReader(
      batches: BatchSource,
      options?: RecordOptionsInput | null,
    ): void
    /** Add every batch `batches` yields after the rows this resource holds. */
    appendArrowBatchReader(
      batches: BatchSource,
      options?: RecordOptionsInput | null,
    ): void

    /** Read this resource's rows, as `readArrowBatchReader` does. */
    readArrow(options?: RecordOptionsInput | null): BatchReader
    /**
     * Replace or merge this resource's rows with whatever `rows` holds.
     *
     * An async source returns a promise, because its rows do not exist until
     * they are awaited; every synchronous source returns nothing.
     */
    writeArrow(rows: RowSource, options?: RecordOptionsInput | null): void
    writeArrow(
      rows: AsyncIterable<RowSource>,
      options?: RecordOptionsInput | null,
    ): Promise<void>
    /** Add whatever `rows` holds after the rows this resource holds. */
    appendArrow(rows: RowSource, options?: RecordOptionsInput | null): void
    appendArrow(
      rows: AsyncIterable<RowSource>,
      options?: RecordOptionsInput | null,
    ): Promise<void>

    /**
     * Read this resource's rows as records: plain objects, or instances of
     * the class you pass, whose constructor receives one plain row.
     */
    readRecords<T = Record<string, unknown>>(
      cls?: (new (row: Record<string, unknown>) => T) | RecordOptionsInput | null,
      options?: RecordOptionsInput | null,
    ): IterableIterator<T>
    /** Replace or merge this resource's rows; `writeArrow` under the record name. */
    writeRecords(rows: RowSource, options?: RecordOptionsInput | null): void
    writeRecords(
      rows: AsyncIterable<RowSource>,
      options?: RecordOptionsInput | null,
    ): Promise<void>
    /** Add records after the rows this resource holds. */
    appendRecords(rows: RowSource, options?: RecordOptionsInput | null): void
    appendRecords(
      rows: AsyncIterable<RowSource>,
      options?: RecordOptionsInput | null,
    ): Promise<void>
  }

  /** Iterating a reader yields one Apache Arrow JS record batch at a time. */
  interface BatchReader extends Iterable<ArrowRecordBatch> {
    /** Drain every remaining batch into one Apache Arrow JS table. */
    toTable(): ArrowTable
  }
  namespace BatchReader {
    /** Build a reader from a reader, an Arrow JS value, or Arrow IPC bytes. */
    function from(source: BatchSource, rootName?: string): BatchReader
  }

  interface Table {
    /** Append batches as a new snapshot, keeping everything already stored. */
    append(batches: BatchSource): void
    /** Replace every row with `batches` as a new snapshot. */
    overwrite(batches: BatchSource): void
    /** Read the current snapshot, keeping the columns `field` names. */
    scan(field?: SchemaInput | null): BatchReader
    /** Add a schema, make it current, and write a new metadata document. */
    evolveSchema(schema: SchemaInput): number
    /** Record a chain of column operations, committed as one new schema. */
    updateSchema(): SchemaUpdateBuilder
  }

  interface Catalog {
    /** Append rows to the named table, creating it on first write. */
    append(name: string, data: BatchSource): Table
    /** Replace the named table's rows, creating it on first write. */
    overwrite(name: string, data: BatchSource): Table
  }

  namespace Timezone {
    /** Coordinated Universal Time, the one zone that is always registered. */
    const UTC: Timezone
  }
}

/** Anything that names a stream of Arrow record batches. */
export type BatchSource =
  | BatchReader
  | ArrowTable
  | ArrowRecordBatch
  | readonly ArrowRecordBatch[]
  | Buffer
  | Uint8Array
  | ArrayBuffer
/**
 * Anything the generic entry points build a reader from: everything
 * `BatchSource` covers, plus an Arrow JS `RecordBatchReader`, a `Vector` a
 * one-column schema names, an object of named columns, plain records, and any
 * iterable of those.
 */
export type RowSource =
  | BatchSource
  | { readAll(): ArrowRecordBatch[] }
  | { readonly [column: string]: unknown }
  | Iterable<unknown>
/** Native record settings, or the media type naming the encoding. */
export type RecordOptionsInput = RecordOptions | MediaTypeInput
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
  updateType(path: string, dataType: DataTypeInput): SchemaUpdate
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
  /** An Iceberg table reached entirely through one container handle. */
  readonly Table: typeof Table
  /** How a table turns column values into the directories it writes. */
  readonly PartitionSpec: typeof PartitionSpec
  /** One live data file of a snapshot, with the spec that placed it. */
  readonly DataFile: typeof DataFile
  /** Number every column of a schema, so a table can carry it. */
  assignFieldIds(schema: Field, start?: number): Field
  /** Throw the core message when a type change is not a legal promotion. */
  canPromote(fromType: DataTypeInput, toType: DataTypeInput): void
  /** Read an Iceberg schema document as a root `Field`. */
  schemaFromJson(name: string, document: Value | unknown): Field
  /** Write a root `Field` as an Iceberg schema document. */
  schemaToJson(schema: Field): Value
}

export declare const iceberg: Iceberg

/**
 * The whole text-record extractor, shared by every entry point that reads,
 * writes, or describes text records.
 *
 * It is exactly what a JSON, YAML, or TOML document parses into - the same
 * names, one snake_case spelling accepted beside each camelCase one - so a
 * reader is specifiable from configuration alone, with no JavaScript in the
 * loop.
 */
export interface TextLineInput {
  /** Start a record at every line this expression matches. */
  pattern?: string | null
  /** The opening spelled directly: `'every_line'` or `'timestamp'`. */
  opening?: 'every_line' | 'timestamp' | null
  /** A second expression matched against the record's opening line. */
  header?: string | null
  /** `logs: true` is `opening: 'timestamp'`, spelled for the common case. */
  logs?: boolean | null
  /**
   * The record terminator. Unset reads `\n`, `\r\n`, and a lone `\r`,
   * mixed in one resource, and writes `\n`.
   */
  linesep?: string | null
  /** What to trim from each end: `'whitespace'`, `'none'`, or `'chars:...'`. */
  lstrip?: string | null
  rstrip?: string | null
  /** Close a batch after this many decoded input bytes. */
  byteSize?: number | null
  /** Close a batch after this many rows; the first bound to trip wins. */
  batchSize?: number | null
  /** The named capture holding the record's timestamp. */
  timestampCapture?: string | null
  /** The zone a naive timestamp is read in, making `unix` a real instant. */
  timezone?: string | null
  /** Constant columns appended to every row. */
  customFields?:
    | ReadonlyMap<string, unknown>
    | Iterable<readonly [string, unknown]>
    | Record<string, unknown>
    | null
  /** Declared datatypes for the captures inference does not type. */
  captureTypes?:
    | ReadonlyMap<string, DataType | string>
    | Iterable<readonly [string, DataType | string]>
    | Record<string, DataType | string>
    | null
}

/**
 * Build the line projection's root Struct `Field` straight from a pattern.
 *
 * The schema `readArrowLines` emits, without a resource or a reader in
 * sight: named captures become typed columns - `(?<threadId>\d+)` infers
 * `int64`, a `captureTypes` entry declares - so a caller marks its partition
 * columns and creates the Iceberg table before the first log line exists.
 */
export declare function schemaFromPattern(
  pattern: string,
  options?: TextLineInput | null,
): Field
export declare function schemaFromPattern(options: TextLineInput): Field

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
 * S3 client, or anything else, and `IOBase.fromArrowFs` turns it into an
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
export interface ArrowFileSystemHandler {
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
