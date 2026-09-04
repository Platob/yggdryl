from __future__ import annotations

import os
import io
from collections.abc import Iterator, Mapping
from enum import IntEnum
from pathlib import Path
from typing import Any, Literal

import pyarrow as pa  # type: ignore[import-untyped]

from yggdryl import (
    AsciiDictionary,
    AsciiEnum,
    Bound,
    BoundStatement,
    DataType,
    Expression,
    Field,
    IOBase,
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
    avro,
    fields,
    gzip,
    iceberg,
    json,
    toml,
    yaml,
    zlib,
    zstd,
)
from yggdryl._native import (
    ByteIterator,
    FieldMetadata,
    IOCursor,
    IcebergNames,
    Listing,
    ScalarEntryIterator,
    ScalarIterator,
)
from yggdryl.enums import Ascii32, AsciiCode
from yggdryl.fields import (
    Ascii24Field,
    Ascii32Field,
    Ascii96Field,
    AsciiField,
    DenseUnionField,
    FixedSizeListField,
    GeographyField,
    GeometryField,
    Int32Field,
    ListField,
    TimeField,
    VariantField,
)

file_uri: Uri = Uri.from_path(Path("data/events.parquet"))
file_url: Url = file_uri.into_url()
joined_uri: Uri = file_uri.joinpath("archive", Path("events.parquet"))
divided_uri: Uri = file_uri / "child"
path: str = file_uri.into_path()
path_protocol: str = os.fspath(file_url)

urn: Urn = Uri("urn:isbn:9780131103627").into_urn()
uri_again: Uri = urn.into_uri()
mime_type: MimeType = file_uri.mime_type
media_type: MediaType = file_uri.media_type
mime_format: Literal["json", "json_lines", "yaml", "toml"] | None = mime_type.format
content_coding: Literal["gzip", "compress", "deflate", "br", "zstd"] | None = (
    MimeType.GZIP.content_coding
)
stem: str | None = file_uri.stem
uri_user: str | None = file_uri.user
uri_password: str | None = file_uri.password
uri_hostname: str | None = file_uri.hostname
uri_bucket: str | None = file_uri.bucket
uri_region: str | None = file_uri.region
file_uri.set_file_name("events.parquet")
file_uri.set_stem("events")
file_uri.set_extension("json")
file_uri.set_extensions(["json", "gz"])
file_uri.set_mime_type(MimeType.JSON)
file_uri.set_media_type(MediaType.from_parts(MimeType.CSV, [MimeType.GZIP]))
removed_extension: bool = file_uri.remove_extension()
cleared_extensions: bool = file_uri.clear_extensions()

field = Field("event", "string", nullable=False)
field_reduce: tuple[object, tuple[str, bool]] = field.__reduce__()
field_metadata: FieldMetadata = field.metadata
field_metadata_equal: bool = field_metadata == Field(
    "other", "string", metadata={}
).metadata
timezone_ordered: bool = Timezone.UTC <= Timezone("Europe/Paris")
timezone_reduce: tuple[object, tuple[str]] = Timezone.UTC.__reduce__()

# Live handles, views, and consuming iterators opt out of Python's inherited
# object-identity hash. These assignments make their stub slots testable.
field_metadata_hash: None = FieldMetadata.__hash__
protocol_field_hash: None = ProtocolField.__hash__
io_hash: None = IOBase.__hash__
cursor_hash: None = IOCursor.__hash__
byte_iterator_hash: None = ByteIterator.__hash__
listing_hash: None = Listing.__hash__
value_iterator_hash: None = ScalarIterator.__hash__
value_entry_iterator_hash: None = ScalarEntryIterator.__hash__
iceberg_names_hash: None = IcebergNames.__hash__
bound_hash: None = Bound.__hash__
bound_statement_hash: None = BoundStatement.__hash__
catalog_hash: None = iceberg.Catalog.__hash__
namespace_hash: None = iceberg.Namespace.__hash__
namespaces_hash: None = iceberg.Namespaces.__hash__
tables_hash: None = iceberg.Tables.__hash__
table_hash: None = iceberg.Table.__hash__
schema_update_hash: None = iceberg.SchemaUpdate.__hash__

field.set_alias("payload")
field.set_comment("the latest trade")
field.set_display("Last trade")
field.set_location(file_url)
field.set_property("postgres", "type", "text")
field.set_accept("application/json")
field.set_accept_encoding("gzip")
field.set_accept_language("en")
field.set_accept_ranges("bytes")
field.set_cache_control("public")
field.set_content_disposition("attachment")
field.set_content_encoding("gzip")
field.set_content_language("en")
field.set_content_length(42)
field.set_content_location("../event")
field.set_content_range("bytes 0-9/10")
field.set_content_type("application/json")
field.set_mime_type(MimeType.JSON)
field.set_media_type(MediaType.from_parts(MimeType.JSON, [MimeType.GZIP]))
field.set_etag('"v1"')
field.set_expires("Sun, 16 Aug 2026 00:00:00 GMT")
field.set_last_modified("Sat, 15 Aug 2026 00:00:00 GMT")
field.set_http_location(file_url)
field.set_range("bytes=0-9")
field.set_vary("accept-encoding")
dtype_scalar: pa.Scalar = DataType("int32").arrow_scalar(1)
field_scalar: pa.Scalar = field.arrow_scalar("payload")
default_dtype_scalar: pa.Scalar = DataType("int32").default_arrow_scalar()
default_field_scalar: pa.Scalar = field.default_arrow_scalar()
source_array = pa.array([1, 2], type=pa.int32())
cast_dtype_array: pa.Array = DataType("int64").cast_arrow_array(source_array)
cast_field_array: pa.Array = Field("value", "int64").cast_arrow_array(source_array)
source_batch = pa.record_batch([source_array], names=["value"])
cast_dtype_batch: pa.RecordBatch = DataType.from_fields(
    [Field("value", "int64")]
).cast_arrow_batch(source_batch)
cast_field_batch: pa.RecordBatch = Field(
    "rows", DataType.from_fields([Field("value", "int64")]), nullable=False
).cast_arrow_batch(source_batch)
default_dtype_value: object = DataType("int32").default_pyvalue()
default_field_value: object = field.default_pyvalue()
default_dtype_hint: object = DataType("int32").default_pyhint()
default_field_hint: object = field.default_pyhint()
arrow_compatible: DataType = DataType("uint32").into_scheme_compat("arrow")
spark_compatible: Field = field.into_scheme_compat("spark")
polars_compatible: Field = field.into_scheme_compat("polars")
pandas_compatible: Field = field.into_scheme_compat("pandas")
iceberg_compatible: Field = field.into_scheme_compat("iceberg")
typed_id: Int32Field = fields.int32("id", nullable=False)
typed_id_kind: Literal["int32"] = typed_id.dtype.id
typed_id_value: int | None = typed_id.default_pyvalue()
typed_id_dtype_value: int = typed_id.dtype.default_pyvalue()
typed_id_hint: object = typed_id.default_pyhint()
typed_id_dtype_hint: object = typed_id.dtype.default_pyhint()
typed_clock: TimeField = fields.time("clock", "microseconds", nullable=False)
typed_ids: ListField[int] = fields.list("ids", typed_id)
nullable_item: Int32Field = fields.int32("item")
typed_fixed: FixedSizeListField[int] = fields.fixed_size_list(
    "fixed", nullable_item, 2, nullable=False
)
typed_fixed_value: list[int | None] | None = typed_fixed.default_pyvalue()
typed_fixed_dtype_value: list[int | None] = (
    typed_fixed.dtype.default_pyvalue()
)
typed_struct = fields.struct("row", [typed_id], nullable=False)
typed_struct_value: object | Mapping[str, object] | None = (
    typed_struct.default_pyvalue()
)

avro_schema: avro.Schema = avro.Schema(
    "long", max_depth=8, max_input_bytes=1_024, max_nodes=32
)
avro_schema_again: avro.Schema = avro.Schema.from_value(
    "long", max_depth=8, max_input_bytes=1_024, max_nodes=32
)
avro_single: bytes = avro.dumps_single(1, avro_schema)
avro_blocks: avro.BlockIterator = avro.blocks(
    avro.dumps([1], avro_schema),
    max_depth=8,
    max_input_bytes=1_024,
    max_nodes=32,
)
avro_block: avro.Block = next(avro_blocks)
avro_rows: list[Any] = avro_block.rows()
avro_value: Any = avro.loads_single(
    avro_single,
    avro_schema_again,
    max_depth=8,
    max_input_bytes=1_024,
    max_nodes=32,
)
assert avro_value == 1
assert avro_rows == [1]

value_handle = IOBase("value.json.gz")
value_handle.write_scalar({"id": 1})
loaded_value: Any = value_handle.read_scalar()
typed_loaded_value: Any = value_handle.read_scalar("row: struct<id: int64 not null> not null")
native_loaded_value: Scalar = value_handle.read_scalar(cls=Scalar)
native_typed_loaded_value: Scalar = value_handle.read_scalar(
    "row: struct<id: int64 not null> not null",
    cls=Scalar,
)
byte_handle = IOBase("range.bin")
appended_offset: int = byte_handle.append_bytes(b"symbol")
appended_text_offset: int = byte_handle.append("!")
appended_view_offset: int = byte_handle.append(memoryview(b"?"))
range_bytes: bytes = byte_handle.read_range_bytes(0, 6)
inferred_range: bytes = byte_handle.read_range(0, 6)
explicit_range_bytes: bytes = byte_handle.read_range(0, 6, cls=bytes)
range_text: str = byte_handle.read_range(0, 6, cls=str)
byte_handle.read_range(0, 6, cls=int)  # type: ignore[arg-type]

native_json_value: Scalar = json.loads("1.5", cls=Scalar)
typed_struct_dtype_value: object | Mapping[str, object] = (
    typed_struct.dtype.default_pyvalue()
)
native_instant = Scalar.datetime(0, "us", "UTC")
native_decimal = Scalar.decimal("1234567890123456789012345678901234567890", 2)
native_enum = Scalar.from_enum("io_mode", "append")
native_scalar_field: Field = Scalar.from_py(1).into_field()
native_array_field: Field = Scalar.from_py([1]).into_array_field()
native_struct_field: Field = Scalar.from_py([{"id": 1}]).into_struct_field()
temporal_count: int | None = native_instant.count
temporal_unit: str | None = native_instant.unit
temporal_zone: str | None = native_instant.zone
decimal_coefficient: int | None = native_decimal.unscaled
decimal_scale: int | None = native_decimal.scale
enum_kind: str | None = native_enum.enum_kind
enum_value: str | None = native_enum.enum_value
enum_ordinal: int | None = native_enum.enum_ordinal
dense_union_dtype: DataType = DataType.variant(
    [
        fields.int64("integer", nullable=False),
        fields.utf8("text", nullable=False),
    ]
)
typed_dense_union: DenseUnionField = fields.dense_union(
    "payload",
    tuple(dense_union_dtype),
    nullable=False,
)
typed_dense_union_kind: Literal["union"] = typed_dense_union.dtype.id
typed_dense_union_value: object = typed_dense_union.default_pyvalue()

# The parenthesis disambiguates: a bare DataType.variant() is the Variant
# datatype, and the three geospatial-era factories carry their own literals.
bare_variant_dtype: DataType = DataType.variant()
typed_variant: VariantField = fields.variant("payload", nullable=False)
typed_variant_kind: Literal["variant"] = typed_variant.dtype.id
geometry_dtype: DataType = DataType.geometry("EPSG:3857")
typed_geometry: GeometryField = fields.geometry("shape", nullable=False)
typed_geometry_kind: Literal["geometry"] = typed_geometry.dtype.id
typed_geometry_value: bytes = typed_geometry.dtype.default_pyvalue()
geography_dtype: DataType = DataType.geography("OGC:CRS84", "karney")
typed_geography: GeographyField = fields.geography("region", "OGC:CRS84", "vincenty")
typed_geography_kind: Literal["geography"] = typed_geography.dtype.id
typed_geography_value: bytes | None = typed_geography.default_pyvalue()
ascii_dtype: DataType = DataType.ascii(3)
ascii_width: int | None = ascii_dtype.ascii_width
currency_dtype: DataType = DataType.from_logical_name("currency")
logical_names: dict[str, DataType] = DataType.logical_names()
typed_ascii: Ascii32Field = fields.ascii32("ccy", nullable=False)
typed_ascii_kind: Literal["ascii32"] = typed_ascii.dtype.id
typed_ascii_narrow: Ascii24Field = fields.ascii24("ccy", nullable=False)
typed_ascii_narrow_kind: Literal["ascii24"] = typed_ascii_narrow.dtype.id
typed_ascii_isin: Ascii96Field = fields.ascii96("isin")
typed_ascii_isin_kind: Literal["ascii96"] = typed_ascii_isin.dtype.id
typed_ascii_value: str = typed_ascii.dtype.default_pyvalue()
typed_ascii_width: AsciiField = fields.ascii("isin", 12)
typed_ascii_width_value: str | None = typed_ascii_width.default_pyvalue()
ascii_dictionary: AsciiDictionary = AsciiDictionary("ascii32", key="int32")
ascii_code: int = ascii_dictionary.push("USD")
ascii_seeded: AsciiDictionary = AsciiDictionary.from_values("ascii32", ["USD", "EUR"])
ascii_value: str | None = ascii_seeded.get(0)
ascii_lookup: int | None = ascii_seeded.get_code("EUR")
ascii_vocabulary: list[str] = ascii_seeded.values
ascii_dictionary_dtype: DataType = ascii_seeded.dtype
ascii_key_dtype: DataType = ascii_seeded.key
ascii_values_dtype: DataType = ascii_seeded.values_dtype
ascii_column: pa.Array = ascii_seeded.into_arrow_array(["USD", None, "EUR"])
ascii_recovered: AsciiDictionary = AsciiDictionary.from_arrow_array(ascii_column)
ascii_enum: type[IntEnum] = ascii_seeded.into_intenum("Currency")
ascii_member: IntEnum = ascii_enum["USD"]
ascii_members: list[str] = [member.name for member in ascii_enum]
ascii_member_name: str = AsciiDictionary.member_name("n/a")


class TypedCurrency(Ascii32):
    USD = "USD"
    EUR = "EUR"


ascii_declared_code: int = int(TypedCurrency.USD)
ascii_declared_value: str = TypedCurrency.USD.into_str()
ascii_parsed: TypedCurrency = TypedCurrency.from_str("JPY")
ascii_by_code: TypedCurrency = TypedCurrency.from_code(0x55534400)
ascii_declared_dtype: DataType = TypedCurrency.dtype()
ascii_declared_enum: AsciiEnum = TypedCurrency.as_enum()
ascii_declared_field: Field = TypedCurrency.field("ccy", nullable=False)
ascii_declared_dictionary: AsciiDictionary = TypedCurrency.into_dictionary()
ascii_recovered_class: type[AsciiCode] = AsciiCode.from_field(ascii_declared_field)
ascii_base: type[AsciiCode] = TypedCurrency

ascii_declaration: AsciiEnum = AsciiEnum("Side", {"BUY": "B"})
ascii_declaration_json: str = ascii_declaration.into_json()
ascii_declaration_parsed: AsciiEnum = AsciiEnum.from_json(ascii_declaration_json)
ascii_declaration_name: str = ascii_declaration.name
ascii_declaration_members: dict[str, str] = ascii_declaration.members
ascii_declaration_value: str | None = ascii_declaration.get("BUY")
ascii_declaration_member: str | None = ascii_declaration.get_member("B")
ascii_declaration_prior: str | None = ascii_declaration.insert("SELL", "S")
ascii_declaration_removed: str | None = ascii_declaration.remove("SELL")
ascii_declaration_codes: list[tuple[str, int]] = ascii_declaration.into_members("ascii32")
ascii_declaration_dictionary: AsciiDictionary = ascii_declaration.into_dictionary("ascii32")
ascii_field_enum: AsciiEnum | None = ascii_declared_field.ascii_enum

byte_chunks: Iterator[bytes] = IOBase.from_bytes(b"payload").pstream_bytes(
    position=1, batch_size=3
)
io_kind: Literal[
    "memory", "file", "directory", "table", "namespace", "catalog", "unknown"
] = IOBase.from_bytes().kind
cursor_chunks: Iterator[bytes] = IOBase.from_bytes(b"payload").cursor().stream_bytes(
    batch_size=3
)

# These are deliberate negative checks. Under ``mypy --strict``, each ignore
# becomes unused if a typed view regresses to ``Any`` or drops its nullable /
# generated-dataclass branch.
field_default_cannot_be_assumed_present: int = (
    nullable_item.default_pyvalue()  # type: ignore[assignment]
)
fixed_children_cannot_be_assumed_present: list[int] = (
    typed_fixed.dtype.default_pyvalue()  # type: ignore[assignment]
)
struct_default_is_not_always_a_mapping: Mapping[str, object] = (
    typed_struct.dtype.default_pyvalue()  # type: ignore[assignment]
)
dynamic_default_needs_narrowing: int = (
    DataType("int32").default_pyvalue()  # type: ignore[assignment]
)
dynamic_field_default_needs_narrowing: str = (
    field.default_pyvalue()  # type: ignore[assignment]
)
hint_is_a_runtime_typing_object: type[int] = (
    typed_id.default_pyhint()  # type: ignore[assignment]
)
invalid_time_unit = DataType.time(1)  # type: ignore[arg-type]
# ``mysql`` is a metadata namespace, not one of the five compatibility targets.
invalid_compatibility_target = field.into_scheme_compat("mysql")  # type: ignore[arg-type]
inferred_dictionary: Field = fields.dictionary("labels", int, str)
inferred_mapping: Field = fields.map_of("counts", str, pa.int32())
field_differences: list[str] = list(field.show_diffs(typed_id, False))
json_source: json.Source = io.BytesIO(b'{"value":42}')
yaml_source: yaml.Source = io.StringIO("value: 42\n")
toml_source: toml.Source = io.StringIO("value = 42\n")
json_destination: json.Destination = io.BytesIO()
yaml_destination: yaml.Destination = Path("value.yaml")
toml_destination: toml.Destination = io.StringIO()
decoded_json: dict[str, int] = json.loads(json_source)
decoded_yaml: dict[str, int] = yaml.loads(yaml_source)
decoded_toml: dict[str, int] = toml.loads(toml_source)
typed_json: object = json.loads("42", field=field)
typed_yaml: object = yaml.loads("42\n", field=field)
typed_toml: object = toml.loads("value = 42\n", field=typed_struct)
encoded_json: bytes = json.dumps(decoded_json)
encoded_yaml: bytes = yaml.dumps(decoded_yaml)
encoded_toml: bytes = toml.dumps(decoded_toml)
returned_json: bytes = json.dump(decoded_json)
returned_yaml: str = yaml.dump(decoded_yaml, utf8=True)
returned_toml: bytes = toml.dump(decoded_toml)
json.dump(decoded_json, json_destination)
yaml.dump(decoded_yaml, yaml_destination)
toml.dump(decoded_toml, toml_destination)
assert typed_json is not None and typed_yaml is not None and typed_toml is not None
assert returned_json and returned_yaml and returned_toml

alias: str | None = field.alias
comment: str | None = field.comment
display: str | None = field.display
location: Url | None = field.location
property_value: str | None = field.get_property("postgres", "type")
properties: list[tuple[str, str]] = list(field.property_iter("postgres"))
iceberg_properties: ProtocolField = field.iceberg
postgres_properties: ProtocolField = field.protocol("POSTGRES")
protocol_scheme: str = iceberg_properties.scheme
protocol_prefix: str = iceberg_properties.prefix
protocol_key: str = iceberg_properties.key("doc")
protocol_comment: str | None = iceberg_properties.comment
protocol_display: str | None = iceberg_properties.display
iceberg_properties["doc"] = "closing price"
iceberg_properties.update({"schema-id": "3"}, snapshot="9")
protocol_names: list[str] = list(iceberg_properties)
protocol_values: list[str] = list(iceberg_properties.values())
protocol_entries: list[tuple[str, str]] = list(iceberg_properties.items())
protocol_value: object = iceberg_properties.get("doc")
protocol_present: bool = "doc" in iceberg_properties
protocol_len: int = len(iceberg_properties)
del iceberg_properties["doc"]
iceberg_properties.clear()

partitioned: Field = Field(
    "row",
    DataType.from_fields([fields.int32("year", nullable=False)]),
    nullable=False,
).with_partition_fields(["year"])
partition_children: list[Field] = partitioned.partition_fields
partition_names: list[str] = partitioned.partition_field_names
partition_count: int = partitioned.partition_field_len
partition_present: bool = partitioned.has_partition_fields
partition_only: Field = partitioned.only_partition_fields()
partition_rest: Field = partitioned.without_partition_fields()
partition_marked: bool = partition_children[0].is_partition
partition_children[0].set_partition(False)
accept: str | None = field.accept
content_length: int | None = field.content_length
content_type: str | None = field.content_type
field_mime_type: MimeType = field.mime_type
field_media_type: MediaType = field.media_type
http_location: Url | None = field.http_location

assert path
assert path_protocol
assert uri_again
assert mime_type
assert media_type
assert stem
assert removed_extension or cleared_extensions or not file_uri.extensions
assert alias
assert comment == "the latest trade"
assert display == "Last trade"
assert protocol_comment == "the latest trade"
assert protocol_display == "Last trade"
assert location
assert property_value
assert properties
assert postgres_properties
assert protocol_scheme == "iceberg"
assert protocol_prefix == "iceberg"
assert protocol_key == "iceberg:doc"
assert protocol_names == ["doc", "schema-id", "snapshot"]
assert protocol_values
assert protocol_entries
assert protocol_value == "closing price"
assert protocol_present
assert protocol_len == 3
assert partition_children
assert partition_names == ["year"]
assert partition_count == 1
assert partition_present
assert partition_only
assert partition_rest
assert partition_marked
assert dtype_scalar
assert field_scalar
assert default_dtype_scalar
assert default_field_scalar
assert default_dtype_value == 0
assert default_field_value == ""
assert default_dtype_hint
assert default_field_hint
assert arrow_compatible
assert spark_compatible
assert typed_id_kind
assert typed_clock
assert typed_ids
assert inferred_dictionary
assert inferred_mapping
assert field_differences == [] or field_differences
assert encoded_json
assert encoded_yaml
assert encoded_toml

record_handle: IOBase = IOBase(Path("trades.arrows"))
parquet_statistics = IOBase(Path("trades.parquet")).read_parquet_statistics()
parquet_rows: int = parquet_statistics["num_rows"]
parquet_created_by: str | None = parquet_statistics["created_by"]
parquet_minimum: bytes | None = parquet_statistics["row_groups"][0]["columns"][0][
    "min_bytes"
]
parquet_geospatial = IOBase(
    Path("shapes.parquet")
).read_parquet_geospatial_statistics("shape")
parquet_geometry_types: list[int] = parquet_geospatial["geometry_types"]


class ForeignArrowReader:
    def __arrow_c_stream__(self, requested_schema: object | None = None, /) -> object:
        return object()


class NotArrowReader:
    pass


record_options: RecordOptions = record_handle.record_options()
hashable_record_options = RecordOptions("trades.arrows")
record_options_stable_hash: int = hashable_record_options.stable_hash()
record_options_hash: int = hash(hashable_record_options)
record_options_ordered: bool = hashable_record_options <= RecordOptions("trades.arrows")
record_options_reduce: tuple[object, tuple[dict[str, Any]]] = (
    hashable_record_options.__reduce__()
)
record_options_copy: RecordOptions = hashable_record_options.__copy__()
record_options_deepcopy: RecordOptions = hashable_record_options.__deepcopy__({})
record_options.batch_row_size = 1024
record_options.commit_row_size = 10_000
record_options.name = "trade"
record_options.safe = True
record_mime_type: MimeType = record_options.mime_type
declared_field: Field | None = record_options.field
record_options.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])
record_batches: pa.RecordBatchReader = record_handle.read_arrow_reader(
    options=record_options,
)
stored_root: Field = record_handle.read_arrow_field()
logical_rows: int = record_handle.row_size
logical_columns: int = record_handle.column_size
io_capable: bool = record_handle.is_io()
record_handle.overwrite_arrow_reader(record_batches, options=record_options)
record_handle.append_arrow_reader(record_batches, options=record_options)
record_handle.write_arrow_reader(record_batches, "overwrite", options=record_options)
record_handle.write_arrow_reader(record_batches, "invalid")  # type: ignore[arg-type]
record_handle.overwrite_arrow_reader(ForeignArrowReader())
record_handle.overwrite_arrow_reader(NotArrowReader())  # type: ignore[arg-type]
record_options.merge_by_names = ["id"]
avro_record_options = RecordOptions("trades.avro")
avro_block_codec: str | None = avro_record_options.block_codec
avro_record_options.block_codec = "zstandard"
avro_sync_marker: bytes | None = avro_record_options.sync_marker
avro_record_options.sync_marker = memoryview(b"0123456789abcdef")
avro_record_options.sync_marker = None
record_match_key: list[str] = record_options.merge_by_names
record_handle.merge_arrow_reader(record_batches, options=record_options)

text_record_options = TextOptions()
text_record_options.rowheader = r"\[(?<level>[A-Z]+)\]"
text_record_options.lstrip = r"^\s+"
text_record_options.rstrip = r"\s+$"
text_record_options.linesep = memoryview(b"\r\n")
text_record_options.autotype = True
text_record_options.timezone = Timezone.UTC
text_header: str | None = text_record_options.rowheader
text_lstrip: str | None = text_record_options.lstrip
text_rstrip: str | None = text_record_options.rstrip
text_linesep: bytes | None = text_record_options.linesep
text_autotype: bool = text_record_options.autotype
text_timezone: Timezone | None = text_record_options.timezone
text_handle: IOBase = IOBase(Path("app.log")).into_text(text_record_options)
line_batches: pa.RecordBatchReader = text_handle.read_arrow_reader(
    options=text_record_options
)

generic_batches: pa.RecordBatchReader = record_handle.read_arrow_reader(options=record_options)
arrow_table: pa.Table = pa.table({"id": [1]})
arrow_batch: pa.RecordBatch = arrow_table.to_batches()[0]
record_handle.overwrite_arrow_table(arrow_table)
record_handle.append_arrow_table(arrow_table)
record_handle.merge_arrow_table(arrow_table, options=record_options)
record_handle.write_arrow_table(arrow_table, "append")
record_handle.overwrite_arrow_batch(arrow_batch)
record_handle.append_arrow_batch(arrow_batch)
record_handle.merge_arrow_batch(arrow_batch, options=record_options)
record_handle.write_arrow_batch(arrow_batch, "overwrite")
record_options.merge_by_names = []
record_handle.overwrite_records([{"id": 1}], options=record_options)
record_handle.append_records([{"id": 2}], options=record_options)
record_options.merge_by_names = ["id"]
record_handle.merge_records([{"id": 2}], options=record_options)
record_handle.write_records([{"id": 3}], "merge", options=record_options)
plain_records: Iterator[dict[str, Any]] = record_handle.read_records(
    options=record_options
)


class TypedRecord:
    id: int


typed_records: Iterator[TypedRecord] = record_handle.read_records(TypedRecord)
record_options.merge_by_names = []
record_handle.append_arrow_reader(generic_batches)
pandas_frames: Iterator[Any] = record_handle.read_pandas()
pandas_frame: Any = record_handle.read_pandas_frame(options=record_options)
record_handle.overwrite_pandas(pandas_frames)
record_handle.append_pandas(pandas_frames)
record_options.merge_by_names = ["id"]
record_handle.merge_pandas(pandas_frames, options=record_options)
record_handle.write_pandas(pandas_frames, "merge", options=record_options)
record_options.merge_by_names = []
record_handle.overwrite_pandas_frame(pandas_frame, options=record_options)
record_handle.append_pandas_frame(pandas_frame)
record_options.merge_by_names = ["id"]
record_handle.merge_pandas_frame(pandas_frame, options=record_options)
record_handle.write_pandas_frame(pandas_frame, "append")
polars_frames: Iterator[Any] = record_handle.read_polars()
polars_frame: Any = record_handle.read_polars_frame(options=record_options)
record_handle.overwrite_polars(polars_frames)
record_handle.append_polars(polars_frames)
record_options.merge_by_names = ["id"]
record_handle.merge_polars(polars_frames, options=record_options)
record_handle.write_polars(polars_frames, "overwrite")
record_options.merge_by_names = []
record_handle.overwrite_polars_frame(polars_frame, options=record_options)
record_handle.append_polars_frame(polars_frame)
record_options.merge_by_names = ["id"]
record_handle.merge_polars_frame(polars_frame, options=record_options)
record_handle.write_polars_frame(polars_frame, "merge", options=record_options)
parquet_options: RecordOptions = RecordOptions("trades.parquet")
row_group_size: int | None = parquet_options.max_row_group_size
footer_metadata: dict[str, str] | None = parquet_options.key_value_metadata

iceberg_schema: Field = iceberg.assign_field_ids(
    pa.schema([pa.field("id", pa.int64(), nullable=False)])
)
iceberg_spec: iceberg.PartitionSpec = iceberg.PartitionSpec.unpartitioned()
iceberg_table: iceberg.Table = iceberg.Table.create(
    IOBase(Path("trades")), iceberg_schema, iceberg_spec
)
iceberg_scan: pa.RecordBatchReader = iceberg_table.scan(iceberg_schema)
iceberg_snapshot: iceberg.Snapshot | None = iceberg_table.current_snapshot
iceberg_files: list[tuple[iceberg.DataFile, iceberg.PartitionSpec]] = (
    iceberg_table.data_files()
)
iceberg_manifests: list[iceberg.ManifestFile] = iceberg_table.manifests()
iceberg_evolved: int = iceberg_table.evolve_schema(iceberg_schema)
if iceberg_manifests:
    iceberg_manifest = iceberg_manifests[0]
    manifest_content: str = iceberg_manifest.content
    manifest_min_sequence: int = iceberg_manifest.min_sequence_number
    manifest_key: bytes | None = iceberg_manifest.key_metadata
    manifest_partitions: tuple[
        tuple[bool, bool | None, bytes | None, bytes | None], ...
    ] = iceberg_manifest.partitions
    manifest_first_row: int | None = iceberg_manifest.first_row_id
    added_file_count: int | None = iceberg_manifest.added_files_count
    existing_file_count: int | None = iceberg_manifest.existing_files_count
    deleted_file_count: int | None = iceberg_manifest.deleted_files_count
    added_row_count: int | None = iceberg_manifest.added_rows_count
    existing_row_count: int | None = iceberg_manifest.existing_rows_count
    deleted_row_count: int | None = iceberg_manifest.deleted_rows_count
if iceberg_snapshot is not None:
    snapshot_key: str | None = iceberg_snapshot.encryption_key_id
    snapshot_direct_manifests: tuple[str, ...] | None = iceberg_snapshot.manifests
    snapshot_first_row: int | None = iceberg_snapshot.first_row_id
    snapshot_added_rows: int | None = iceberg_snapshot.added_rows
if iceberg_files:
    iceberg_file = iceberg_files[0][0]
    iceberg_file_mime_type: MimeType = iceberg_file.mime_type
    file_key: bytes | None = iceberg_file.key_metadata
    nan_counts: dict[int, int] = iceberg_file.nan_value_counts
    equality_ids: list[int] | None = iceberg_file.equality_ids
    referenced_file: str | None = iceberg_file.referenced_data_file
    content_offset: int | None = iceberg_file.content_offset
    content_size: int | None = iceberg_file.content_size_in_bytes

assert record_mime_type
assert declared_field is None or declared_field
assert stored_root
assert row_group_size
assert footer_metadata is None or footer_metadata == {}
assert plain_records
assert iceberg_spec.is_unpartitioned()
assert iceberg_scan
assert iceberg_snapshot is None or iceberg_snapshot.operation
assert iceberg_files == [] or iceberg_files
assert iceberg_manifests == [] or iceberg_manifests
assert iceberg_evolved >= 0

iceberg_document: dict[str, object] = iceberg.schema_into_json(iceberg_schema)
iceberg_reread: Field = iceberg.schema_from_json("row", iceberg_document)

assert iceberg_document
assert iceberg_reread

# Record configuration crosses through the one options object.
selected_options: RecordOptions = record_handle.record_options()
selected_options.select_by_names = ["id"]
selected_options.batch_row_size = 1024
selected_reader: pa.RecordBatchReader = record_handle.read_arrow_reader(
    options=selected_options
)
selected_root: Field = record_handle.read_arrow_field(options=selected_options)

assert selected_reader
assert selected_root

# Iceberg keeps all configuration in its own options type.
iceberg_options: iceberg.IcebergOptions = iceberg.IcebergOptions(
    commit_retries=2,
    commit_total_timeout_ms=30_000,
    target_file_size=1024,
    data_mime_type="avro",
)
iceberg_puffin_options: iceberg.IcebergOptions = iceberg.IcebergOptions(
    data_mime_type=MimeType.PUFFIN
)
iceberg_retries: int = iceberg_options.commit_retries
iceberg_timeout: int = iceberg_options.commit_total_timeout_ms
iceberg_mime_type: MimeType = iceberg_options.data_mime_type
iceberg_options.data_mime_type = MimeType.PARQUET
iceberg_table.append(
    pa.table({"id": [1]}), options=iceberg.IcebergOptions(commit_retries=1)
)
iceberg_table.overwrite(
    pa.table({"id": [1]}),
    options=iceberg.IcebergOptions(data_mime_type="avro"),
)
iceberg_table.set_options(iceberg.IcebergOptions(target_file_size=2048))
iceberg_resolved: iceberg.IcebergOptions = iceberg_table.options()
iceberg_options_scan: pa.RecordBatchReader = iceberg_table.scan(
    options=iceberg.IcebergOptions(read_parallelism=2)
)

assert iceberg_retries >= 0
assert iceberg_timeout >= 0
assert iceberg_mime_type
assert iceberg_puffin_options
assert iceberg_resolved
assert iceberg_options_scan

# The catalog chains through its views: namespaces, then tables, then a table.
catalog: iceberg.Catalog = iceberg.Catalog(Path("warehouse"))
catalog_namespaces: iceberg.Namespaces = catalog.namespaces
namespace: iceberg.Namespace = catalog_namespaces["sales"]
namespace_names: list[str] = list(catalog_namespaces)
namespace_count: int = len(catalog_namespaces)
namespace_known: bool = "sales" in catalog_namespaces
nested: iceberg.Namespace = namespace.namespaces.open_or_create("eu")
namespace_tables: iceberg.Tables = namespace.tables
chained_table: iceberg.Table = catalog.namespaces["sales"].tables["orders"]
table_names: list[str] = list(namespace_tables)
table_known: bool = "orders" in namespace_tables
created_table: iceberg.Table = namespace_tables.create("fills", iceberg_schema)
opened_table: iceberg.Table = namespace_tables.open_or_create(
    "fills", iceberg_schema
)
appended_table: iceberg.Table = namespace_tables.append(
    "orders",
    pa.table({"id": [1]}),
    options=iceberg.IcebergOptions(data_mime_type="avro"),
)
overwritten_table: iceberg.Table = namespace_tables.overwrite(
    "orders", pa.table({"id": [1]}), options=iceberg_options
)

assert namespace.name
assert nested.name
assert namespace_names == [] or namespace_names
assert namespace_count >= 0
assert namespace_known or not namespace_known
assert chained_table
assert table_names == [] or table_names
assert table_known or not table_known
assert created_table and opened_table and appended_table and overwritten_table

# A filter is a mapping or a sequence of pairs, and it rides beside the same
# projection and options every other scan takes.
filtered_scan: pa.RecordBatchReader = iceberg_table.scan_where({"venue": "XNAS"})
paired_scan: pa.RecordBatchReader = iceberg_table.scan_where(
    [("venue", "XNAS")],
    iceberg_schema,
    options=iceberg.IcebergOptions(read_parallelism=2),
)
unfiltered_scan: pa.RecordBatchReader = iceberg_table.scan_where()
branch_scan: pa.RecordBatchReader = iceberg_table.scan_ref("nightly")
branch_projection: pa.RecordBatchReader = iceberg_table.scan_ref(
    "nightly",
    {"venue": "XNAS"},
    iceberg_schema,
    options=iceberg.IcebergOptions(data_mime_type="avro"),
)

# A plan answers in counts, so every getter is an `int` rather than a view.
scan_plan: iceberg.ScanPlan = iceberg_table.plan()
filtered_plan: iceberg.ScanPlan = iceberg_table.plan([("venue", "XNAS")])
historic_plan: iceberg.ScanPlan = iceberg_table.plan_at(1, {"venue": "XNAS"})
planned_records: int = scan_plan.record_count
planned_files: int = scan_plan.files_planned
skipped_files: int = scan_plan.files_skipped
read_manifests: int = scan_plan.manifests_read
skipped_manifests: int = scan_plan.manifests_skipped

assert filtered_scan and paired_scan and unfiltered_scan
assert branch_scan and branch_projection
assert filtered_plan and historic_plan
assert planned_records >= 0
assert planned_files >= skipped_files or skipped_files >= planned_files
assert read_manifests >= 0
assert skipped_manifests >= 0

# Scoped writes take filters first and one explicit options value.
iceberg_table.overwrite_where(
    {"venue": "XNAS"},
    pa.table({"id": [1]}),
    options=iceberg.IcebergOptions(data_mime_type="avro"),
)
iceberg_table.overwrite_where(None, pa.table({"id": [1]}))
iceberg_table.merge(pa.table({"id": [1]}), ["id"])
iceberg_table.merge(
    pa.table({"id": [1]}),
    ["id"],
    safe=False,
    options=iceberg.IcebergOptions(commit_retries=1),
)
iceberg_table.merge_where(
    [("venue", "XNAS")],
    pa.table({"id": [1]}),
    ["id"],
    safe=True,
    options=iceberg.IcebergOptions(target_file_size=1024),
)

# Maintenance answers with the identifiers it acted on, or with nothing.
expired_snapshots: list[int] = iceberg_table.expire_snapshots()
explicitly_expired: list[int] = iceberg_table.expire_snapshots(0, 1, [2, 3])
iceberg_table.fast_forward("nightly", 1)
snapshot_manifests: list[iceberg.ManifestFile] = iceberg_table.manifests_at(1)

assert expired_snapshots == [] or expired_snapshots
assert explicitly_expired == [] or explicitly_expired
assert snapshot_manifests == [] or snapshot_manifests

# More deliberate negative checks: a filter is not one string, a plan getter is
# not a view, and a snapshot is named by identifier rather than by value.
one_string_is_not_a_filter = iceberg_table.plan("venue")  # type: ignore[arg-type]
a_plan_getter_is_a_count: str = (
    iceberg_table.plan().files_planned  # type: ignore[assignment]
)
a_snapshot_is_named_by_identifier = iceberg_table.manifests_at(
    iceberg_snapshot  # type: ignore[arg-type]
)

# The three codings hold bytes in and bytes out, raw DEFLATE included.
gzip_bytes: bytes = gzip.dumps(b'{"id": 1}')
gzip_plain: bytes = gzip.loads(gzip_bytes)
zlib_bytes: bytes = zlib.dumps(b'{"id": 1}', level=9)
zlib_plain: bytes = zlib.loads(zlib_bytes)
zlib_raw: bytes = zlib.dumps_raw(b'{"id": 1}', 1)
zlib_raw_plain: bytes = zlib.loads_raw(zlib_raw)
zstd_bytes: bytes = zstd.dumps(b'{"id": 1}')
zstd_plain: bytes = zstd.loads(zstd_bytes)

assert gzip_plain == zlib_plain == zlib_raw_plain == zstd_plain

# A handle's declared coding is optional, and the transfers answer in bytes.
declared_codec: str | None = record_handle.codec
coded_handle: IOBase = IOBase(Path("trades.arrows.gz"))
bytes_written: int = record_handle.compress_into(coded_handle)
bytes_levelled: int = record_handle.compress_into(coded_handle, "zstd", 9)
bytes_read: int = coded_handle.decompress_into(record_handle)
bytes_decoded: int = coded_handle.decompress_into(record_handle, codec="gzip")

assert declared_codec is None or declared_codec
assert bytes_written >= 0
assert bytes_levelled >= 0
assert bytes_read >= 0
assert bytes_decoded >= 0

# The filter, in the same three languages the schema is in.
expression_schema: Field = Field(
    "trades",
    DataType.from_fields(
        [
            Field("ccy", "utf8", True),
            Field("price", "decimal128(9,2)", True),
        ]
    ),
    False,
)
expression: Expression = Expression("ccy = 'EUR' and price > 100")
expression_parsed: Expression = Expression.parse("ccy = 'EUR'")
expression_restored: Expression = Expression.from_json(expression.into_json())
expression_named: Expression = Expression.column("ccy")
expression_constant: Expression = Expression.literal("EUR")
expression_held: Expression = Expression.attribute("partition", "year")
expression_stat: Expression = Expression.attribute("size")
expression_late: Expression = Expression.parameter("floor")
expression_true: Expression = Expression.always_true()
expression_false: Expression = Expression.always_false()
expression_columns: list[str] = expression.columns()
expression_attributes: list[str] = expression_held.attributes()
expression_parameters: list[str] = expression_late.parameters()
expression_conjuncts: list[Expression] = expression.conjuncts()
expression_depth: int = expression.depth()
expression_document: str = expression.into_json()
expression_both: Expression = expression_named & expression_constant
expression_either: Expression = expression_named | "price > 1"
expression_negated: Expression = ~expression_named
expression_field: Field = expression_named.field(expression_schema)
expression_bound: Bound = expression.bind(expression_schema)
expression_bound_text: Expression = expression_bound.expression
expression_bound_field: Field = expression_bound.field
expression_is_predicate: bool = expression_bound.is_predicate
expression_bound_columns: list[str] = expression_bound.columns
expression_reads_rows: bool = expression_bound.reads_rows
expression_matches: bool = expression_bound.matches(["EUR", None])
expression_value: object = expression_bound.eval({"ccy": "EUR"})
expression_split: tuple[Expression, Expression] = expression_bound.partition_split()

statement: Statement = Statement("select ccy where ccy = 'EUR' limit 10")
statement_restored: Statement = Statement.from_json(statement.into_json())
statement_projections: list[str] = statement.projections
statement_predicate: Expression | None = statement.predicate
statement_ordering: list[
    tuple[Expression, Literal["ascending", "descending"], Literal["first", "last"] | None]
] = statement.ordering
statement_limit: int | None = statement.limit
statement_is_all: bool = statement.is_all
bound_statement: BoundStatement = statement.bind(expression_schema)
bound_statement_schema: Field = bound_statement.schema
bound_statement_output: Field = bound_statement.output
bound_statement_projections: list[Bound] = bound_statement.projections
bound_statement_predicate: Bound | None = bound_statement.predicate
bound_statement_ordering: list[
    tuple[Bound, Literal["ascending", "descending"], Literal["first", "last"] | None]
] = bound_statement.ordering
bound_statement_limit: int | None = bound_statement.limit
bound_statement_is_all: bool = bound_statement.is_all
statement_batch = pa.record_batch({"ccy": ["EUR"], "price": [1]})
projected_statement_batch: pa.RecordBatch = bound_statement.project_arrow(statement_batch)
projected_statement_table: pa.Table = bound_statement.project_arrow(pa.Table.from_batches([statement_batch]))
projected_statement_reader: pa.RecordBatchReader = bound_statement.project_arrow(
    pa.RecordBatchReader.from_batches(statement_batch.schema, [statement_batch])
)
sorted_statement_batch: pa.RecordBatch = bound_statement.sort_arrow_batch(statement_batch)

expression_matched: list[IOBase] = list(
    IOBase("file:///lake").children_matching("&holder.partition['year'] = '2024'")
)

assert str(expression)
assert expression_parsed and expression_restored
assert expression_named and expression_constant and expression_held
assert expression_stat and expression_late and expression_true and expression_false
assert expression_columns == ["ccy", "price"]
assert expression_attributes and not expression_parameters or expression_parameters
assert expression_conjuncts and expression_depth >= 1
assert expression_document and expression_both and expression_either
assert expression_negated and expression_field and expression_bound_text
assert expression_bound_field and expression_is_predicate
assert expression_bound_columns and not expression_reads_rows is None
assert expression_matches or not expression_matches
assert expression_value is None or expression_value
assert expression_split
assert statement_restored and statement_projections
assert statement_predicate is None or statement_predicate
assert statement_ordering == [] or statement_ordering
assert statement_limit is None or statement_limit
assert statement_is_all or not statement_is_all
assert bound_statement and bound_statement_schema and bound_statement_output
assert bound_statement_projections == [] or bound_statement_projections
assert bound_statement_predicate is None or bound_statement_predicate
assert bound_statement_ordering == [] or bound_statement_ordering
assert bound_statement_limit is None or bound_statement_limit
assert bound_statement_is_all or not bound_statement_is_all
assert projected_statement_batch and projected_statement_table
assert projected_statement_reader and sorted_statement_batch
assert expression_matched == [] or expression_matched
