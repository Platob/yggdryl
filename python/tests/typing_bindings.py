from __future__ import annotations

import datetime
import io
import os
from collections.abc import Iterator, Mapping
from enum import IntEnum
from pathlib import Path
from typing import Any, Literal

import pyarrow as pa  # type: ignore[import-untyped]
import pyarrow.fs as pa_fs  # type: ignore[import-untyped]

import yggdryl
from yggdryl import (
    ArrowCastPlan,
    BloombergCodeField,
    Bound,
    BoundSelector,
    BytesField,
    CfiCodeField,
    CountryField,
    CcyField,
    CusipCodeField,
    DataType,
    DenseUnionField,
    Expression,
    Field,
    Filter,
    FixedSizeSerieField,
    GeographyField,
    GeometryField,
    IOBase,
    Int32Field,
    IsinCodeField,
    MediaType,
    MicCodeField,
    MimeType,
    Parameters,
    Plan,
    ProtocolField,
    PythonMetadata,
    RecordOptions,
    Records,
    Scalar,
    SedolCodeField,
    Selector,
    Serie,
    SerieField,
    SerieReader,
    StringField,
    Term,
    TextLine,
    TextOptions,
    TimeField,
    Timezone,
    Uri,
    Url,
    Urn,
    UuidField,
    VariantField,
    Version,
    VersionField,
    avro,
    fix,
    gzip,
    iceberg,
    json,
    temporal,
    toml,
    txhash,
    xxhash,
    yaml,
    zlib,
    zstd,
)
from yggdryl._native import (
    ByteIterator,
    BytesParameters,
    FieldMetadata,
    FixCode,
    FixDirection,
    FixEntryTuple,
    FixMessages,
    IOCursor,
    IcebergNames,
    Listing,
    ScalarEntryIterator,
    ScalarIterator,
    StringEnum,
    StringParameters,
)
from yggdryl.coding import Coded, Gzip, Identity, Zlib, Zstd
from yggdryl.enums import AsciiCode, CcyCode, fixed_ascii
from yggdryl.holder import (
    Buffer,
    Buffered,
    FsFile,
    FsFolder,
    FsPath,
    LocalFile,
    LocalFolder,
    LocalPath,
)
from yggdryl.media import Avro, Ipc, Media, Parquet, Text

numeric_version: Version = Version(5, 0, 2)
parsed_version: Version = Version.from_str("255.255.65535")
version_parts: tuple[int, int, int] = (parsed_version.major, parsed_version.minor, parsed_version.patch)
version_hash: int = numeric_version.stable_hash()
version_compared: bool = numeric_version <= parsed_version
version_copied: Version = numeric_version.__copy__()
version_pickled: tuple[object, tuple[int, int, int]] = numeric_version.__reduce__()
typed_version: VersionField = yggdryl.version("fixversion")
typed_version_kind: Literal["version"] = typed_version.dtype.id

file_uri: Uri = Uri.from_path(Path("data/events.parquet"))
file_url: Url = file_uri.into_url()
joined_uri: Uri = file_uri.joinpath("archive", Path("events.parquet"))
divided_uri: Uri = file_uri / "child"
path: str = file_uri.into_path()
path_protocol: str = os.fspath(file_url)

# Components answer raw text or the text their escapes stand for, and the query
# answers as the pairs it spells.
raw_query: str | None = file_url.query()
decoded_query: str | None = file_url.query(decode=True)
decoded_fragment: str | None = file_url.fragment(True)
decoded_path: str = file_url.path_text(decode=True)
query_parameters: Parameters = file_url.parameters(decode=True)
parameter_value: str = query_parameters.setdefault("as of", "2026-01-02")
parameter_or_none: str | None = query_parameters.get("absent")
parameter_or_default: str | int = query_parameters.get("absent", 0)
parameter_values: tuple[str, ...] = query_parameters.get_all("as of")
parameter_keys: list[str] = list(query_parameters.keys())
parameter_items: list[tuple[str, str]] = list(query_parameters.items())
parameter_map: dict[str, str] = query_parameters.into_dict()
query_parameters["symbol"] = "AAPL"
query_parameters.append("symbol", "MSFT")
query_parameters.update({"venue": "XNAS"}, region="eu")
popped_parameter: str | None = query_parameters.pop("region", None)
del query_parameters["symbol"]
query_text: str | None = query_parameters.into_query()
file_url.set_query(query_text)
assert query_parameters.decode
assert len(query_parameters) >= 0
assert ("venue" in query_parameters) or True

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
fix_messages_hash: None = FixMessages.__hash__
fix_codec_hash: None = fix.FixCodec.__hash__
bound_hash: None = Bound.__hash__
bound_selector_hash: None = BoundSelector.__hash__
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
default_field_serie: Serie = Serie.from_default(field)
default_field_rows: Serie = Serie.from_default(field, 3)
default_field_scalar: pa.Scalar = default_field_serie.into_arrow_scalar()
source_array = pa.array([1, 2], type=pa.int32())
landed_array: Serie = Serie.from_arrow_array(source_array)
cast_field_array: pa.Array = Serie.from_arrow_array(
    source_array, Field("value", "int64"), safe=False, nullability="strict"
).into_arrow_array()
bit_cast_field_array: pa.Array = Serie.from_arrow_array(
    pa.array([2**64 - 1], type=pa.uint64()), Field("value", "int64"), representation="bits"
).into_arrow_array()
cast_dtype_serie: Serie = landed_array.cast(DataType("int64"))
cast_field_serie: Serie = landed_array.cast(Field("value", "int64"), nullability="strict")
source_batch = pa.record_batch([source_array], names=["value"])
cast_root = Field("rows", DataType.from_fields([Field("value", "int64")]), nullable=False)
landed_batch: Serie = Serie.from_arrow_batch(source_batch)
cast_field_batch: pa.RecordBatch = Serie.from_arrow_batch(
    source_batch, cast_root
).into_arrow_batch()
drained_reader: Serie = Serie.from_arrow_reader(
    pa.RecordBatchReader.from_batches(source_batch.schema, [source_batch]), cast_root
)
serie_reader: SerieReader = SerieReader.from_arrow_reader(
    pa.RecordBatchReader.from_batches(source_batch.schema, [source_batch]), cast_root
)
serie_reader_field: Field = serie_reader.field
serie_reader_batches: list[Serie] = list(serie_reader)
serie_reader_stream: pa.RecordBatchReader = SerieReader.from_arrow_reader(
    pa.RecordBatchReader.from_batches(source_batch.schema, [source_batch])
).into_arrow_reader()
cast_plan = ArrowCastPlan(source_batch.schema, cast_root, safe=True, nullability="default")
cast_plan_source: pa.Field = cast_plan.source
cast_plan_target: Field = cast_plan.target
cast_plan_identity: bool = cast_plan.is_identity
cast_plan_serie: Serie = cast_plan.apply(source_batch)
cast_plan_column: Serie = ArrowCastPlan(Field("value", "int32"), Field("value", "int64")).apply(
    landed_array
)
applied_root = Field(
    "rows", DataType.from_fields([Field("value", "int64")]), nullable=False
)
applied_batch: pa.RecordBatch = applied_root.apply_arrow_batch(
    source_batch, digest=True, transform=True, cast=True
)
applied_schema: pa.Schema = applied_root.apply_arrow_schema(source_batch.schema)
applied_reader: pa.RecordBatchReader = applied_root.apply_arrow_reader(
    pa.RecordBatchReader.from_batches(source_batch.schema, [source_batch])
)
applied_partition_batch: pa.RecordBatch = applied_root.partition.apply_arrow_batch(
    source_batch
)
applied_digest_batch: pa.RecordBatch = applied_root.digest.apply_arrow_batch(source_batch)
partition_sources: list[str] | None = applied_root.partition.sources
partition_transform: str | None = applied_root.partition.transform
filled_digest_batch: pa.RecordBatch = xxhash.Xxh3().apply_arrow_batch(
    Field(
        "rows",
        DataType.from_fields([Field("value", "int64")]),
        nullable=False,
    ),
    source_batch,
    force=True,
)
coupled_value: txhash.TxHash = txhash.txh3(b"AAPL", 1_700_000_000_000_000)
coupled_unix: int = coupled_value.unix
coupled_unit: str = coupled_value.unit
coupled_digest_half: xxhash.Digest = coupled_value.digest
coupled_bytes: bytes = bytes(coupled_value)
coupled_instant: Scalar = coupled_value.into_datetime()
coupled_uuid: Scalar = coupled_value.into_uuid()
coupled_sequenced_uuid: Scalar = coupled_value.into_sequenced_uuid(7, 11)
coupled_restated: txhash.TxHash = coupled_value.with_unit("s")
coupled_parts: txhash.TxHash = txhash.TxHash.from_parts(datetime.datetime.now(datetime.timezone.utc), coupled_digest_half)
coupled_hasher: txhash.TxHasher = txhash.TxHasher("xxh64", unit="s", seed=7)
coupled_hashed: txhash.TxHash = coupled_hasher.digest(b"AAPL", 1_700_000_000)
coupled_scalar_hashed: txhash.TxHash = coupled_hasher.digest_scalar(Scalar.from_("AAPL"), 1)
coupled_unix_of: int = coupled_hasher.unix_of("2023-11-14T22:13:20Z")
coupled_rows: pa.Array = txhash.row_txhashes(source_batch, pa.array([1], pa.int64()))
coupled_split: tuple[pa.Array, pa.Array] = txhash.decompose(coupled_rows)
coupled_joined: pa.Array = txhash.compose(coupled_split[0], coupled_split[1])
coupled_now: int = txhash.unix_now("ms")
coupled_width: int = txhash.width("xxh3-128")
coupled_dtype: DataType = txhash.dtype("xxh32")
coupled_time: str | None = field.digest.time
coupled_holder_unit: str | None = field.digest.unit
coupled_flag: bool = field.digest.is_coupled()
default_dtype_native_scalar: Scalar = DataType("int32").default_scalar()
default_field_native_scalar: Scalar = field.default_scalar()
default_dtype_hint: object = DataType("int32").default_pyhint()
default_field_hint: object = field.default_pyhint()
arrow_compatible: DataType = DataType("uint32").into_scheme_compat("arrow")
spark_compatible: Field = field.into_scheme_compat("spark")
polars_compatible: Field = field.into_scheme_compat("polars")
pandas_compatible: Field = field.into_scheme_compat("pandas")
iceberg_compatible: Field = field.into_scheme_compat("iceberg")
typed_id: Int32Field = yggdryl.int32("id", nullable=False)
typed_id_kind: Literal["int32"] = typed_id.dtype.id
typed_id_default_scalar: Scalar = typed_id.default_scalar()
typed_id_dtype_default_scalar: Scalar = typed_id.dtype.default_scalar()
typed_id_hint: object = typed_id.default_pyhint()
typed_id_dtype_hint: object = typed_id.dtype.default_pyhint()
typed_bit_cast_array: pa.Array = Serie.from_arrow_array(
    pa.array([2**32 - 1], type=pa.uint32()), typed_id, representation="bits"
).into_arrow_array()
typed_clock: TimeField = yggdryl.time("clock", "microseconds", nullable=False)
typed_ids: SerieField[int] = yggdryl.serie("ids", typed_id)
nullable_item: Int32Field = yggdryl.int32("item")
typed_fixed: FixedSizeSerieField[int] = yggdryl.fixed_size_serie(
    "fixed", nullable_item, 2, nullable=False
)
typed_fixed_default_scalar: Scalar = typed_fixed.default_scalar()
typed_fixed_dtype_default_scalar: Scalar = typed_fixed.dtype.default_scalar()
typed_struct = yggdryl.struct("row", [typed_id], nullable=False)
typed_struct_default_scalar: Scalar = typed_struct.default_scalar()

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
typed_struct_dtype_default_scalar: Scalar = typed_struct.dtype.default_scalar()
native_instant = DataType('datetime64(us,"UTC")').scalar(0)
native_decimal = Scalar.decimal("1234567890123456789012345678901234567890", 2)
native_enum_text: str | None = Scalar.from_enum("IOMode", "append").as_str()
native_scalar_id: str = native_instant.id
native_scalar_family: str = native_instant.family
native_scalar_field: Field = Scalar.from_(1).into_field()
native_array_field: Field = Scalar.from_([1]).into_array_field()
native_struct_field: Field = Scalar.from_([{"id": 1}]).into_struct_field()
temporal_count: int | None = native_instant.count
temporal_unit: str | None = native_instant.unit
temporal_zone: str | None = native_instant.zone
decimal_coefficient: int | None = native_decimal.unscaled
decimal_scale: int | None = native_decimal.scale
dense_union_dtype: DataType = DataType.variant(
    [
        yggdryl.int64("integer", nullable=False),
        yggdryl.utf8("text", nullable=False),
    ]
)
typed_dense_union: DenseUnionField = yggdryl.dense_union(
    "payload",
    tuple(dense_union_dtype),
    nullable=False,
)
typed_dense_union_kind: Literal["union"] = typed_dense_union.dtype.id
typed_dense_union_default_scalar: Scalar = typed_dense_union.default_scalar()

# The parenthesis disambiguates: a bare DataType.variant() is the Variant
# datatype, and the three geospatial-era factories carry their own literals.
bare_variant_dtype: DataType = DataType.variant()
typed_variant: VariantField = yggdryl.variant("payload", nullable=False)
typed_variant_kind: Literal["variant"] = typed_variant.dtype.id
geometry_dtype: DataType = DataType.geometry("EPSG:3857")
typed_geometry: GeometryField = yggdryl.geometry("shape", nullable=False)
typed_geometry_kind: Literal["geometry"] = typed_geometry.dtype.id
typed_geometry_default_scalar: Scalar = typed_geometry.dtype.default_scalar()
geography_dtype: DataType = DataType.geography("OGC:CRS84", "karney")
typed_geography: GeographyField = yggdryl.geography("region", "OGC:CRS84", "vincenty")
typed_geography_kind: Literal["geography"] = typed_geography.dtype.id
typed_geography_default_scalar: Scalar = typed_geography.default_scalar()
ascii_dtype: DataType = DataType.fixed_ascii(3)
ascii_width: int | None = ascii_dtype.fixed_byte_width
ascii_parameters: StringParameters | None = ascii_dtype.string_parameters
ascii_charset: str | None = ascii_dtype.charset
string_parameters: StringParameters = StringParameters("large_string", "windows-1252")
string_parameters_layout: str = string_parameters.layout
string_parameters_charset: str = string_parameters.charset
string_parameters_bound: int | None = string_parameters.bound
string_parameters_fixed: int | None = string_parameters.fixed
string_parameters_max: int | None = string_parameters.max
string_dtype: DataType = DataType.string("string", "utf-8", 16)
utf8_dtype: DataType = DataType.utf8()
large_utf8_dtype: DataType = DataType.large_utf8()
utf8_view_dtype: DataType = DataType.utf8_view()
variable_ascii_dtype: DataType = DataType.ascii()
fixed_utf8_dtype: DataType = DataType.fixed_utf8(8)
bytes_parameters: BytesParameters = BytesParameters("fixed_size_binary", 16)
bytes_parameters_layout: str = bytes_parameters.layout
bytes_parameters_fixed: int | None = bytes_parameters.fixed
bytes_parameters_max: int | None = bytes_parameters.max
bytes_dtype: DataType = DataType.bytes("binary_view", 64)
binary_dtype: DataType = DataType.binary()
large_binary_dtype: DataType = DataType.large_binary()
binary_view_dtype: DataType = DataType.binary_view()
fixed_size_binary_dtype: DataType = DataType.fixed_size_binary(16)
bytes_dtype_parameters: BytesParameters | None = bytes_dtype.bytes_parameters
ccy_dtype: DataType = DataType.from_logical_name("ccy")
ccy_width: int | None = ccy_dtype.fixed_byte_width
logical_names: dict[str, DataType] = DataType.logical_names()
prebuilt_lists: dict[str, list[str]] = StringEnum.prebuilt()
prebuilt_mics: StringEnum = StringEnum.from_logical_name("mic")
typed_ascii: StringField = yggdryl.ascii("note", nullable=False)
typed_ascii_kind: Literal[
    "utf8", "large_utf8", "utf8_view", "large_utf8_view", "fixed_utf8", "sized_utf8",
    "ascii", "large_ascii", "ascii_view", "large_ascii_view", "fixed_ascii", "sized_ascii",
    "cp1252", "large_cp1252", "cp1252_view", "large_cp1252_view", "fixed_cp1252",
    "sized_cp1252",
] = typed_ascii.dtype.id
typed_ascii_fixed: StringField = yggdryl.fixed_ascii("ccy", 3, nullable=False)
typed_ascii_fixed_kind: Literal[
    "utf8", "large_utf8", "utf8_view", "large_utf8_view", "fixed_utf8", "sized_utf8",
    "ascii", "large_ascii", "ascii_view", "large_ascii_view", "fixed_ascii", "sized_ascii",
    "cp1252", "large_cp1252", "cp1252_view", "large_cp1252_view", "fixed_cp1252",
    "sized_cp1252",
] = typed_ascii_fixed.dtype.id
typed_string: StringField = yggdryl.string(
    "name", layout="string", charset="windows-1252", max=32, nullable=False
)
typed_fixed_utf8: StringField = yggdryl.fixed_utf8("name", 8)
typed_large_utf8_view: StringField = yggdryl.large_utf8_view("name")
typed_sized_utf8: StringField = yggdryl.sized_utf8("name", 32)
typed_large_ascii: StringField = yggdryl.large_ascii("name")
typed_ascii_view: StringField = yggdryl.ascii_view("name")
typed_large_ascii_view: StringField = yggdryl.large_ascii_view("name")
typed_sized_ascii: StringField = yggdryl.sized_ascii("name", 4)
typed_cp1252: StringField = yggdryl.cp1252("name")
typed_large_cp1252: StringField = yggdryl.large_cp1252("name")
typed_cp1252_view: StringField = yggdryl.cp1252_view("name")
typed_large_cp1252_view: StringField = yggdryl.large_cp1252_view("name")
typed_fixed_cp1252: StringField = yggdryl.fixed_cp1252("name", 8)
typed_sized_cp1252: StringField = yggdryl.sized_cp1252("name", 32)
typed_bytes: BytesField = yggdryl.bytes("blob", layout="binary_view", max=64)
typed_fixed_bytes: BytesField = yggdryl.bytes("digest", layout="fixed_binary", fixed=16)
typed_binary: BytesField = yggdryl.binary("payload", nullable=False)
typed_binary_kind: Literal[
    "binary",
    "large_binary",
    "binary_view",
    "large_binary_view",
    "fixed_binary",
    "sized_binary",
] = typed_binary.dtype.id
typed_country: CountryField = yggdryl.country("iso", nullable=False)
typed_country_kind: Literal["country"] = typed_country.dtype.id
typed_ccy: CcyField = yggdryl.ccy("ccy", nullable=False)
typed_ccy_kind: Literal["ccy"] = typed_ccy.dtype.id
typed_mic: MicCodeField = yggdryl.mic("venue")
typed_mic_kind: Literal["mic"] = typed_mic.dtype.id
typed_cfi: CfiCodeField = yggdryl.cfi("classification")
typed_cfi_kind: Literal["cfi"] = typed_cfi.dtype.id
typed_isin: IsinCodeField = yggdryl.isin("instrument")
typed_isin_kind: Literal["isin"] = typed_isin.dtype.id
typed_cusip: CusipCodeField = yggdryl.cusip("cusip")
typed_cusip_kind: Literal["cusip"] = typed_cusip.dtype.id
typed_sedol: SedolCodeField = yggdryl.sedol("sedol")
typed_sedol_kind: Literal["sedol"] = typed_sedol.dtype.id
typed_bloomberg: BloombergCodeField = yggdryl.bloomberg("bloomberg")
typed_bloomberg_kind: Literal["bloomberg"] = typed_bloomberg.dtype.id
typed_uuid: UuidField = yggdryl.uuid("id", nullable=False)
typed_uuid_kind: Literal["uuid"] = typed_uuid.dtype.id
typed_uuid_default_scalar: Scalar = typed_uuid.dtype.default_scalar()
typed_ascii_default_scalar: Scalar = typed_ascii.dtype.default_scalar()
typed_ascii_sedol: StringField = yggdryl.fixed_ascii("sedol", 7)
# Reading one is the generic conversion, so it lands as ``object``.
typed_ascii_sedol_value: object = typed_ascii_sedol.default_scalar().as_py()
ascii_member_name: str = StringEnum.member_name("n/a")


# A width base is a class built at runtime, so it is bound to a name rather
# than spelled inline: a dynamic base is not a base a checker can follow.
ascii_width_base: type[AsciiCode] = fixed_ascii(4)


class TypedCcy(CcyCode):
    USD = "USD"
    EUR = "EUR"


ascii_declared_code: int = int(TypedCcy.USD)
ascii_declared_value: str = TypedCcy.USD.into_str()
ascii_parsed: TypedCcy = TypedCcy.from_str("JPY")
ascii_by_code: TypedCcy = TypedCcy.from_code(0x55534400)
ascii_declared_dtype: DataType = TypedCcy.dtype()
ascii_declared_enum: StringEnum = TypedCcy.as_enum()
ascii_declared_field: Field = TypedCcy.into_field("ccy", nullable=False)
ascii_recovered_class: type[AsciiCode] = AsciiCode.from_field(ascii_declared_field)
ascii_base: type[AsciiCode] = TypedCcy

ascii_declaration: StringEnum = StringEnum("Side", {"BUY": "B"})
ascii_declaration_json: str = ascii_declaration.into_json()
ascii_declaration_parsed: StringEnum = StringEnum.from_json(ascii_declaration_json)
ascii_declaration_name: str = ascii_declaration.name
ascii_declaration_members: dict[str, str] = ascii_declaration.members
ascii_declaration_value: str | None = ascii_declaration.get("BUY")
ascii_declaration_member: str | None = ascii_declaration.get_member("B")
ascii_declaration_prior: str | None = ascii_declaration.insert("SELL", "S")
ascii_declaration_removed: str | None = ascii_declaration.remove("SELL")
ascii_declaration_codes: list[tuple[str, int]] = ascii_declaration.into_members(
    DataType.fixed_ascii(4)
)
ascii_declaration_intenum: type[IntEnum] = ascii_declaration.into_intenum(DataType.fixed_ascii(4))
ascii_declaration_intenum_member: IntEnum = ascii_declaration_intenum["BUY"]
ascii_field_enum: StringEnum | None = ascii_declared_field.string_enum

byte_chunks: Iterator[bytes] = IOBase.from_bytes(b"payload").pstream_bytes(
    position=1, batch_size=3
)
io_kind: Literal[
    "memory", "file", "directory", "table", "namespace", "catalog", "unknown"
] = IOBase.from_bytes().kind
cursor_chunks: Iterator[bytes] = IOBase.from_bytes(b"payload").cursor().stream_bytes(
    batch_size=3
)

# Every storage role is an ``IOBase``; the wrappers descend one layer at a time.
role_path: LocalPath = LocalPath("trades.txt")
role_file: LocalFile = LocalFile("trades.bin")
role_folder: LocalFolder = LocalFolder("lake")
role_temporary: LocalFolder = LocalFolder.temporary()
role_home: LocalFolder = LocalFolder.home()
role_config: LocalFolder = LocalFolder.config()
role_cached: IOBase = IOBase.from_bytes(b"payload").buffered(page_size=8)
role_fs_path: FsPath = FsPath(pa_fs.LocalFileSystem(), "trades.txt")
role_fs_file: FsFile = FsFile(pa_fs.LocalFileSystem(), "trades.bin")
role_fs_folder: FsFolder = FsFolder(pa_fs.LocalFileSystem(), "lake")
role_created: FsFolder = role_fs_folder.create_dir(recursive=True)
coding_roles: list[type[Coded]] = [Identity, Gzip, Zlib, Zstd]
encoding_roles: list[type[Media]] = [Ipc, Parquet, Avro]
storage_roles: list[type[IOBase]] = [Buffer, Buffered, Text]

# These are deliberate negative checks. Under ``mypy --strict``, each ignore
# becomes unused if a typed view regresses to ``Any`` or answers a default as
# anything but the one ``Scalar`` every value crosses as.
field_default_is_a_scalar_not_its_value: int = (
    nullable_item.default_scalar()  # type: ignore[assignment]
)
nested_default_is_a_scalar_not_its_children: list[int] = (
    typed_fixed.dtype.default_scalar()  # type: ignore[assignment]
)
struct_default_is_not_a_mapping: Mapping[str, object] = (
    typed_struct.dtype.default_scalar()  # type: ignore[assignment]
)
dynamic_default_needs_narrowing: int = (
    DataType("int32").default_scalar().as_py()  # type: ignore[assignment]
)
dynamic_field_default_needs_narrowing: str = (
    field.default_scalar().as_py()  # type: ignore[assignment]
)
hint_is_a_runtime_typing_object: type[int] = (
    typed_id.default_pyhint()  # type: ignore[assignment]
)
invalid_time_unit = DataType.time(1)  # type: ignore[arg-type]
# ``mysql`` is a metadata namespace, not one of the five compatibility targets.
invalid_compatibility_target = field.into_scheme_compat("mysql")  # type: ignore[arg-type]
inferred_dictionary: Field = yggdryl.dictionary("labels", int, str)
inferred_mapping: Field = yggdryl.map_of("counts", str, pa.int32())
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
digest_properties: ProtocolField = field.digest
identity_properties: ProtocolField = field.identity
partition_properties: ProtocolField = field.partition
python_properties: ProtocolField = field.python
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

digest_root: Field = Field(
    "row",
    DataType.from_fields([yggdryl.int32("id", nullable=False)]),
    nullable=False,
)
digest_children: list[Field] = digest_root.digest_fields
digest_names: list[str] = digest_root.digest_field_names
digest_count: int = digest_root.digest_field_len
digest_only: Field = digest_root.only_digest_fields()

partitioned: Field = Field(
    "row",
    DataType.from_fields([yggdryl.int32("year", nullable=False)]),
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
assert identity_properties is not None
assert partition_properties is not None
assert postgres_properties
assert protocol_scheme == "iceberg"
assert protocol_prefix == "iceberg"
assert protocol_key == "ICEBERG:doc"
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
assert default_field_scalar
assert len(default_field_rows) == 3
assert default_dtype_native_scalar.as_py() == 0
assert default_field_native_scalar.as_py() == ""
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


record_options: RecordOptions | TextOptions = record_handle.record_options()
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
record_options.merge_by = ["id"]
avro_record_options = RecordOptions("trades.avro")
avro_block_codec: str | None = avro_record_options.block_codec
avro_record_options.block_codec = "zstandard"
avro_sync_marker: bytes | None = avro_record_options.sync_marker
avro_record_options.sync_marker = memoryview(b"0123456789abcdef")
avro_record_options.sync_marker = None
record_match_key: list[str] = record_options.merge_by.names
record_handle.merge_arrow_reader(record_batches, options=record_options)

text_record_options = TextOptions()
text_record_options.framing = True
text_record_options.leading_fragment = "drop"
text_record_options.max_record_byte_size = 4096
text_record_options.rowheader = r"\[(?<level>[A-Z]+)\]"
text_record_options.start_rownum = 1
text_record_options.parse_mtime = True
text_record_options.lstrip = r"^\s+"
text_record_options.rstrip = r"\s+$"
text_record_options.linesep = memoryview(b"\r\n")
text_record_options.autotype = True
text_record_options.timezone = Timezone.UTC
text_framing: bool = text_record_options.framing
text_leading_fragment: Literal["keep", "drop", "error"] = (
    text_record_options.leading_fragment
)
text_max_record_byte_size: int | None = text_record_options.max_record_byte_size
text_header: str | None = text_record_options.rowheader
text_lstrip: str | None = text_record_options.lstrip
text_rstrip: str | None = text_record_options.rstrip
text_linesep: bytes | None = text_record_options.linesep
text_autotype: bool = text_record_options.autotype
text_timezone: Timezone | None = text_record_options.timezone
text_rownum: int | None = text_record_options.start_rownum
text_parse_mtime: bool = text_record_options.parse_mtime
regex_dtype: DataType = DataType.from_regex(r"(?<id>\d+)")
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
record_options.merge_by = []
record_handle.overwrite_records([{"id": 1}], options=record_options)
record_handle.append_records([{"id": 2}], options=record_options)
record_options.merge_by = ["id"]
record_handle.merge_records([{"id": 2}], options=record_options)
record_handle.write_records([{"id": 3}], "merge", options=record_options)
plain_records: Iterator[dict[str, Any]] = record_handle.read_records(
    options=record_options
)


class TypedRecord:
    id: int


typed_records: Iterator[TypedRecord] = record_handle.read_records(TypedRecord)
record_options.merge_by = []
record_handle.append_arrow_reader(generic_batches)
pandas_frames: Iterator[Any] = record_handle.read_pandas()
pandas_frame: Any = record_handle.read_pandas_frame(options=record_options)
record_handle.overwrite_pandas(pandas_frames)
record_handle.append_pandas(pandas_frames)
record_options.merge_by = ["id"]
record_handle.merge_pandas(pandas_frames, options=record_options)
record_handle.write_pandas(pandas_frames, "merge", options=record_options)
record_options.merge_by = []
record_handle.overwrite_pandas_frame(pandas_frame, options=record_options)
record_handle.append_pandas_frame(pandas_frame)
record_options.merge_by = ["id"]
record_handle.merge_pandas_frame(pandas_frame, options=record_options)
record_handle.write_pandas_frame(pandas_frame, "append")
polars_frames: Iterator[Any] = record_handle.read_polars()
polars_frame: Any = record_handle.read_polars_frame(options=record_options)
record_handle.overwrite_polars(polars_frames)
record_handle.append_polars(polars_frames)
record_options.merge_by = ["id"]
record_handle.merge_polars(polars_frames, options=record_options)
record_handle.write_polars(polars_frames, "overwrite")
record_options.merge_by = []
record_handle.overwrite_polars_frame(polars_frame, options=record_options)
record_handle.append_polars_frame(polars_frame)
record_options.merge_by = ["id"]
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
selected_options: RecordOptions | TextOptions = record_handle.record_options()
selected_options.select = ["id"]
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
iceberg_write_parallelism: int = iceberg.IcebergOptions(write_parallelism=2).write_parallelism
iceberg_write_staging: str | None = iceberg.IcebergOptions(write_staging="off").write_staging

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

# The same coding read in place, from the name or from an explicit argument.
decoded_view: IOBase = IOBase(Path("app.log.gz")).into_coded()
levelled_view: IOBase = IOBase(Path("app.log")).into_coded("zstd", 9)

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
term: Term = Term("ccy = 'EUR' and price > 100")
term_parsed: Term = Term.parse("ccy = 'EUR'")
term_restored: Term = Term.from_json(term.into_json())
term_named: Term = Term.column("ccy")
term_constant: Term = Term.literal("EUR")
term_late: Term = Term.parameter("floor")
term_true: Term = Term.always_true()
term_false: Term = Term.always_false()
term_columns: list[str] = term.columns()
term_parameters: list[str] = term_late.parameters()
term_conjuncts: list[Term] = term.conjuncts()
term_depth: int = term.depth()
term_document: str = term.into_json()
term_simplified: Term = term.simplify()
term_explained: str = term.explain()
term_sliced: Term = term_named.slice(1, None)
term_both: Term = term_named & term_constant
term_either: Term = term_named | "price > 1"
term_negated: Term = ~term_named
term_field: Field = term_named.field(expression_schema)
term_bound: Bound = term.bind(expression_schema)
term_bound_text: Term = term_bound.term
term_bound_field: Field = term_bound.field
term_is_predicate: bool = term_bound.is_predicate
term_bound_columns: list[str] = term_bound.columns
term_reads_rows: bool = term_bound.reads_rows
term_matches: bool = term_bound.matches(["EUR", None])
term_value: object = term_bound.eval({"ccy": "EUR"})
term_split: tuple[Filter, Filter] = term_bound.partition_split()
term_bound_explained: str = term_bound.explain()

filter: Filter = Filter("ccy = 'EUR'")
filter_from_term: Filter = Filter(term_named.eq("'EUR'"))
filter_restored: Filter = Filter.from_json(filter.into_json())
filter_term: Term = filter.term
filter_conjuncts: list[Filter] = filter.conjuncts()
filter_both: Filter = filter & "price > 1"
filter_negated: Filter = ~filter
filter_bound: Bound = filter.bind(expression_schema)
filter_field: Field = filter.apply_field(expression_schema)
filter_explained: str = filter.explain()

selector: Selector = Selector("ccy, price * 2 as doubled")
selector_from_parts: Selector = Selector([term_named, (term_named, "alias")])
selector_all: Selector = Selector.all()
selector_except: Selector = Selector.all_except(["price"])
selector_columns: Selector = Selector.from_columns(["ccy"])
selector_from_field: Selector = Selector.from_field(expression_schema)
selector_names: list[str] = selector.names
selector_projections: list[str] = selector.projections
selector_excluded: list[str] = selector_except.excluded
selector_is_all: bool = selector_all.is_all
selector_field: Field = selector.apply_field(expression_schema)
selector_stored: Field = selector.into_field(expression_schema)
bound_selector: BoundSelector = selector.bind(expression_schema)
bound_selector_schema: Field = bound_selector.schema
bound_selector_output: Field = bound_selector.output
bound_selector_projections: list[Bound] = bound_selector.projections
bound_selector_identity: bool = bound_selector.is_identity
bound_selector_row: dict[str, object] = bound_selector.apply_row({"ccy": "EUR", "price": 1})
selector_explained: str = selector.explain()

plan: Plan = Plan("select ccy from t where ccy = 'EUR' order by ccy desc limit 10")
plan_restored: Plan = Plan.from_json(plan.into_json())
plan_from_field: Plan = Plan.from_field(expression_schema)
plan_built: Plan = (
    Plan()
    .with_create("id int64 not null", "trades")
    .with_write("upsert into", "'file:///lake/trades.parquet'", ["id"])
    .with_select(selector)
    .with_source("raw")
    .with_filter(filter)
    .with_ordering([(term_named, "desc", "last"), "price"])
    .with_limit(10)
    .with_offset(1)
)
plan_selector: Selector = plan.selector
plan_filter: Filter = plan.filter
plan_source: str | None = plan.source
plan_source_plan: Plan | None = plan.source_plan
plan_ordering: list[tuple[Term, Literal["asc", "desc"], Literal["first", "last"]]] = plan.ordering
plan_limit: int | None = plan.limit
plan_offset: int | None = plan.offset
plan_verb: str | None = plan_built.verb
plan_target: str | None = plan_built.write_target
plan_schema: Selector | None = plan_built.schema
plan_merge_by: Selector = plan_built.merge_by
plan_field: Field | None = plan_built.field()
plan_columns: list[str] = plan.columns()
plan_read_columns: list[str] | None = plan.read_columns()
plan_is_identity: bool = plan.is_identity
plan_read: Plan = plan.read_sections()
plan_expression: Expression = plan.into_expression()
plan_explained: str = plan.explain()
plan_batch = pa.record_batch({"ccy": ["EUR"], "price": [1]})
plan_projected_batch: pa.RecordBatch = plan.read_sections().apply_arrow(plan_batch)
plan_projected_table: pa.Table = plan.read_sections().apply_arrow(pa.Table.from_batches([plan_batch]))
plan_projected_reader: pa.RecordBatchReader = plan.read_sections().apply_arrow(
    pa.RecordBatchReader.from_batches(plan_batch.schema, [plan_batch])
)
plan_records: Records = plan.read_sections().apply_records([{"ccy": "EUR", "price": 1}])
plan_rows: list[dict[str, object]] = plan_records.collect()

expression: Expression = Expression("select ccy where ccy = 'EUR'")
expression_parsed: Expression = Expression.parse("select ccy")
expression_restored: Expression = Expression.from_json(expression.into_json())
expression_select: Expression = Expression.select(selector)
expression_where: Expression = Expression.filter(filter)
expression_plan: Expression = Expression.plan(plan)
expression_sequence: Expression = Expression.sequence([expression_select, expression_where])
expression_kind: Literal["select", "where", "plan", "sequence"] = expression.kind
expression_steps: list[Expression] = expression_sequence.steps
expression_selector: Selector | None = expression_select.as_selector()
expression_filter: Filter | None = expression_where.as_filter()
expression_as_plan: Plan | None = expression.as_plan()
expression_columns: list[str] = expression.columns()
expression_field: Field = expression_select.apply_field(expression_schema)
expression_batch: pa.RecordBatch = expression.apply_arrow_batch(plan_batch)
expression_reader: pa.RecordBatchReader = expression.apply_arrow_reader(
    pa.RecordBatchReader.from_batches(plan_batch.schema, [plan_batch])
)
expression_records: Records = expression.apply_records([{"ccy": "EUR", "price": 1}])
expression_records_reader: pa.RecordBatchReader = expression_records.into_arrow_reader()
expression_explained: str = expression.explain()

assert str(term)
assert term_parsed and term_restored
assert term_named and term_constant
assert term_late and term_true and term_false
assert term_columns == ["ccy", "price"]
assert term_parameters == ["floor"]
assert term_conjuncts and term_depth >= 1
assert term_document and term_simplified and term_explained and term_sliced
assert term_both and term_either and term_negated and term_field
assert term_bound_text and term_bound_field and term_is_predicate
assert term_bound_columns and not term_reads_rows is None
assert term_matches or not term_matches
assert term_value is None or term_value
assert term_split and term_bound_explained
assert filter and filter_from_term and filter_restored and filter_term
assert filter_conjuncts and filter_both and filter_negated
assert filter_bound and filter_field and filter_explained
assert selector and selector_from_parts and selector_all and selector_except
assert selector_columns and selector_from_field
assert selector_names and selector_projections and selector_excluded
assert selector_is_all and selector_field and selector_stored
assert bound_selector and bound_selector_schema and bound_selector_output
assert bound_selector_projections == [] or bound_selector_projections
assert bound_selector_identity or not bound_selector_identity
assert bound_selector_row and selector_explained
assert plan and plan_restored and plan_from_field and plan_built
assert plan_selector and plan_filter
assert plan_source is None or plan_source
assert plan_source_plan is None or plan_source_plan
assert plan_ordering == [] or plan_ordering
assert plan_limit is None or plan_limit
assert plan_offset is None or plan_offset
assert plan_verb is None or plan_verb
assert plan_target is None or plan_target
assert plan_schema is None or plan_schema
assert plan_merge_by is not None
assert plan_field is None or plan_field
assert plan_columns and plan_read_columns is None or plan_read_columns
assert plan_is_identity or not plan_is_identity
assert plan_read and plan_expression and plan_explained
assert plan_projected_batch and plan_projected_table and plan_projected_reader
assert plan_records is not None and plan_rows == [] or plan_rows
assert expression and expression_parsed and expression_restored
assert expression_select and expression_where and expression_plan and expression_sequence
assert expression_kind and expression_steps
assert expression_selector is None or expression_selector
assert expression_filter is None or expression_filter
assert expression_as_plan is None or expression_as_plan
assert expression_columns and expression_field
assert expression_batch is not None and expression_reader is not None
assert expression_records is not None and expression_records_reader is not None
assert expression_explained

fix_field: Field = Field("OrderQty", "decimal128(20, 8)")
fix_field.fix.tag = 38
fix_field.fix.tags = [1088]
fix_field.fix.names = ["Qty", "Quantity"]
fix_field.fix.description = "Quantity ordered."
fix_tag: int | None = fix_field.fix.tag
fix_tags: list[int] = fix_field.fix.tags
fix_names: list[str] = fix_field.fix.names
fix_description: str | None = fix_field.fix.description
fix_field.fix.branches = ["cme", "Bloomberg"]
fix_field.fix.add_branch("ice")
fix_branches: list[str] = fix_field.fix.branches
fix_has_branch: bool = fix_field.fix.has_branch("BLOOMBERG")
fix_id: int | None = fix_field.fix.id

python_field: Field = Field("Quote", "int64", nullable=False)
python_field.python.class_metadata = PythonMetadata(
    "trading.book", "Book.Quote", "dataclass"
)
python_field.python.module = "trading.execution"
python_field.python.qualname = "Book.Fill"
python_field.python.kind = "field"
python_declared: PythonMetadata | None = python_field.python.class_metadata
python_module: str | None = python_field.python.module
python_qualname: str | None = python_field.python.qualname
python_class_name: str | None = python_field.python.class_name
python_kind: str | None = python_field.python.kind
python_import_path: str | None = python_field.python.import_path
python_declaration: PythonMetadata = PythonMetadata("trading.book", "Quote")
python_declared_module: str = python_declaration.module
python_declared_qualname: str = python_declaration.qualname
python_declared_class_name: str = python_declaration.class_name
python_declared_kind: str = python_declaration.kind
python_declared_path: str = python_declaration.import_path
python_declared_importable: bool = python_declaration.is_importable
python_declared_keyword: bool = python_declaration.is_keyword_constructed
python_declared_properties: dict[str, str] = python_declaration.properties
python_declared_hash: int = python_declaration.stable_hash()
python_from_type: PythonMetadata = PythonMetadata.from_type(Field, "class")

fix_vendor: Field = Field("TradeID", "utf8")
fix_vendor.fix.tag = 5001
fix_vendor.fix.branches = ["cme"]
fix_vendor_id: int | None = fix_vendor.fix.id

fix_registry: fix.FixRegistry = fix.FixRegistry()
fix_registry_from_fields: fix.FixRegistry = fix.FixRegistry.from_fields([fix_field])
fix_registry_loaded: fix.FixRegistry = fix.FixRegistry.from_handle(
    Path("config") / "fix"
)
fix_registry_from_url: fix.FixRegistry = fix.FixRegistry.from_handle(
    Url("file:///dictionary")
)
fix_registry_from_handle: fix.FixRegistry = fix.FixRegistry.from_handle(
    IOBase("file:///dictionary")
)
fix_registry_from_text: fix.FixRegistry = fix.FixRegistry.from_handle(
    "file:///dictionary"
)
fix_field_id: int = fix_id if fix_id is not None else 0
fix_by_id: Field = fix_registry_from_fields.field_by_id(fix_field_id)
fix_maybe_by_id: Field | None = fix_registry_from_fields.get_field_by_id(fix_field_id)
fix_by_tag: Field = fix_registry_from_fields.field_by_tag(38)
fix_maybe_by_tag: Field | None = fix_registry_from_fields.get_field_by_tag(38)
fix_by_name: Field = fix_registry_from_fields.field_by_name("qty")
fix_maybe_by_name: Field | None = fix_registry_from_fields.get_field_by_name("qty")
fix_by_path: Field = fix_registry_from_fields.field_by_path("OrderQty")
fix_maybe_by_path: Field | None = fix_registry_from_fields.get_field_by_path(
    "OrderQty"
)
fix_dialects: list[str] = fix_registry_from_fields.dialects()
fix_bytes_protocol: MimeType = MimeType.infer_bytes(b"35=D|")
fix_text_protocol: MimeType = MimeType.infer_text("35=D|")
fix_bytes_msgtype: bytes | None = fix.FixCodec.infer_msgtype_bytes(b"35=D|")
fix_text_msgtype: str | None = fix.FixCodec.infer_msgtype_text("35=D|")
fix_generic: Field = fix_registry_from_fields.field(38)
fix_maybe_generic: Field | None = fix_registry_from_fields.get_field("OrderQty")
fix_item: Field = fix_registry_from_fields[38]
fix_default: object = fix_registry_from_fields.get("nope", None)
fix_replaced: Field | None = fix_registry.insert(fix_field)
fix_registry.update(fix_field)
fix_removed: Field | None = fix_registry.remove(38)
fix_removed_by_id: Field | None = fix_registry.remove_by_id(fix_field_id)
fix_size: int = len(fix_registry_from_fields)
fix_has: bool = 38 in fix_registry_from_fields
fix_field_names: list[str] = [entry.name for entry in fix_registry_from_fields]
fix_registry_from_fields.write_into(Path("build") / "fix")


fix_root: Field = Field(
    "NewOrderSingle", DataType.from_fields([fix_field]), nullable=False
)
fix_message: fix.FixMsg = fix.FixMsg(fix_root, {"OrderQty": 100})
fix_message_explicit: fix.FixMsg = fix.FixMsg(
    fix_root, {"OrderQty": 100}, fix_registry_from_fields
)
fix_message_registry: fix.FixRegistry = fix_message.registry
fix_message_field: Field = fix_message.field
fix_message_value: Scalar = fix_message.value
fix_message_by_id: Scalar = fix_message.by_id(fix_field_id)
fix_message_maybe_id: Scalar | None = fix_message.get_by_id(fix_field_id)
fix_message_by_tag: Scalar = fix_message.by_tag(38)
fix_message_maybe_tag: Scalar | None = fix_message.get_by_tag(38)
fix_message_by_name: Scalar = fix_message.by_name("qty")
fix_message_maybe_name: Scalar | None = fix_message.get_by_name("qty")
fix_message_by_path: Scalar = fix_message.by_path("OrderQty")
fix_message_maybe_path: Scalar | None = fix_message.get_by_path("OrderQty")
fix_message_item: Scalar = fix_message["OrderQty"]
fix_message_default: object = fix_message.get(9999, None)
fix_message_pairs: list[tuple[str, Scalar]] = list(fix_message)
fix_message_len: int = len(fix_message)
fix_message_hash: int = fix_message.stable_hash()
fix_message_digest: bytes = fix_message.digest()
fix_message_event: fix.OperationEventData = fix_message.event()
fix_message_header: fix.FixHeader = fix_message.header()
fix_message_capture: fix.FixCapture = fix_message.capture()
fix_message_text: str | None = fix_message.text
fix_message_msgcat: int | None = fix_message.msgcat
fix_message_metadata: dict[str, str] = fix_message.metadata
fix_message_curruuid: Scalar = fix_message.curruuid
fix_message_crossuuid: Scalar = fix_message.crossuuid
fix_message_crosscode: str = fix_message.crosscode
fix_message_currhashcode: int = fix_message.currhashcode
fix_message_crosshashcode: int = fix_message.crosshashcode
fix_message_currunix: int = fix_message.currunix
fix_message_state: Scalar = fix_message.state
fix_message_seqnum: int = fix_message.seqnum
fix_message_prevuuid: Scalar | None = fix_message.prevuuid
fix_message_srcuuids: list[Scalar] = fix_message.srcuuids
fix_message_marketoperationid: int | None = fix_message.marketoperationid
fix_message_price: Scalar | None = fix_message.price
fix_message_currency: Scalar = fix_message.currency
fix_message_quantity: Scalar | None = fix_message.quantity
fix_message_unit: str = fix_message.unit
fix_message_side: Scalar = fix_message.side
fix_message_securityids: dict[str, str] = fix_message.securityids
fix_message_cficode: Scalar | None = fix_message.cficode
fix_message_miccode: Scalar | None = fix_message.miccode
fix_message_lastpx: Scalar | None = fix_message.lastpx
fix_message_lastqty: Scalar | None = fix_message.lastqty
fix_message_avgpx: Scalar | None = fix_message.avgpx
fix_message_cumqty: Scalar | None = fix_message.cumqty
fix_message_leavesqty: Scalar | None = fix_message.leavesqty
fix_message_prevpx: Scalar | None = fix_message.prevpx
fix_message_prevqty: Scalar | None = fix_message.prevqty
fix_message_spotrate: Scalar | None = fix_message.spotrate
fix_message_forwardpoints: Scalar | None = fix_message.forwardpoints
fix_message_ticker: str | None = fix_message.ticker
fix_message_tif: str | None = fix_message.tif
fix_message_tradable: bool | None = fix_message.tradable
fix_message_accountids: dict[str, str] = fix_message.accountids
fix_message_userids: dict[str, str] = fix_message.userids
fix_message_altids: dict[str, str] = fix_message.altids
fix_message_bid: dict[str, Scalar | str | None] | None = fix_message.bid
fix_message_ask: dict[str, Scalar | str | None] | None = fix_message.ask
fix_message_entries: list[FixEntryTuple] = fix_message.entries()
fix_message_wire: bytes = fix_message.into_bytes(124)
fix_message_wire_text: str = fix_message.into_text("|")
fix_message.set(38, 200)
fix_message.set("OrderQty", None)
fix_message_removed: Scalar | None = fix_message.remove(38)
fix_message_read_back: fix.FixMsg = fix.FixMsg.from_row(fix_root, {"OrderQty": 100})
fix_message_read_back_explicit: fix.FixMsg = fix.FixMsg.from_row(
    fix_root, fix_message_value, fix_registry_from_fields
)

fix_header_beginstring: str = fix_message_header.beginstring
fix_header_msgtype: str = fix_message_header.msgtype
fix_header_sendercompid: str | None = fix_message_header.sendercompid
fix_header_targetcompid: str | None = fix_message_header.targetcompid
fix_header_msgseqnum: int | None = fix_message_header.msgseqnum
fix_header_sendingtime: int = fix_message_header.sendingtime
fix_header_stated: bool = fix_message_header.stated_sendingtime
fix_header_possdupflag: bool | None = fix_message_header.possdupflag
fix_header_msgdirection: str | None = fix_message_header.msgdirection

fix_capture_msgpluginid: str | None = fix_message_capture.msgpluginid
fix_capture_msgctxid: str | None = fix_message_capture.msgctxid
fix_capture_msgsessionid: str | None = fix_message_capture.msgsessionid
fix_capture_msgsesseventid: str | None = fix_message_capture.msgsesseventid

fix_event_curruuid: Scalar = fix_message_event.curruuid
fix_event_crossuuid: Scalar = fix_message_event.crossuuid
fix_event_crosscode: str = fix_message_event.crosscode
fix_event_currhashcode: int = fix_message_event.currhashcode
fix_event_crosshashcode: int = fix_message_event.crosshashcode
fix_event_srcuuids: list[Scalar] = fix_message_event.srcuuids
fix_event_currunix: int = fix_message_event.currunix
fix_event_state: Scalar = fix_message_event.state
fix_event_seqnum: int = fix_message_event.seqnum
fix_event_creaunix: int | None = fix_message_event.creaunix
fix_event_execunix: int | None = fix_message_event.execunix
fix_event_recdunix: int | None = fix_message_event.recdunix
fix_event_exprtime: int | None = fix_message_event.exprtime
fix_event_prevunix: int | None = fix_message_event.prevunix
fix_event_prevuuid: Scalar | None = fix_message_event.prevuuid
fix_event_snapunix: int | None = fix_message_event.snapunix
fix_event_marketoperationid: int | None = fix_message_event.marketoperationid
fix_event_price: Scalar | None = fix_message_event.price
fix_event_quantity: Scalar | None = fix_message_event.quantity
fix_event_lastpx: Scalar | None = fix_message_event.lastpx
fix_event_lastqty: Scalar | None = fix_message_event.lastqty
fix_event_avgpx: Scalar | None = fix_message_event.avgpx
fix_event_cumqty: Scalar | None = fix_message_event.cumqty
fix_event_leavesqty: Scalar | None = fix_message_event.leavesqty
fix_event_prevpx: Scalar | None = fix_message_event.prevpx
fix_event_prevqty: Scalar | None = fix_message_event.prevqty
fix_event_tif: str | None = fix_message_event.tif
fix_event_tradable: bool | None = fix_message_event.tradable
fix_event_ticker: str | None = fix_message_event.ticker
fix_event_currency: Scalar = fix_message_event.currency
fix_event_unit: str = fix_message_event.unit
fix_event_side: Scalar = fix_message_event.side
fix_event_securityids: dict[str, str] = fix_message_event.securityids
fix_event_cficode: Scalar | None = fix_message_event.cficode
fix_event_miccode: Scalar | None = fix_message_event.miccode
fix_event_spotrate: Scalar | None = fix_message_event.spotrate
fix_event_forwardpoints: Scalar | None = fix_message_event.forwardpoints
fix_event_metadata: dict[str, str] = fix_message_event.metadata
fix_event_accountids: dict[str, str] = fix_message_event.accountids
fix_event_userids: dict[str, str] = fix_message_event.userids
fix_event_altids: dict[str, str] = fix_message_event.altids
fix_event_bid: dict[str, Scalar | str | None] | None = fix_message_event.bid
fix_event_ask: dict[str, Scalar | str | None] | None = fix_message_event.ask

fix_reader: fix.FixCodec = fix.FixCodec(fix_registry_from_fields)
fix_reader_pinned: fix.FixCodec = fix.FixCodec(
    fix_registry_from_fields,
    default_sending_time=datetime.datetime(2024, 1, 2, 10, 15, 30, tzinfo=datetime.timezone.utc),
    separator=124,
    payload_column="line",
    null_values=["<none>"],
    direction="R",
    batch_byte_size=1 << 20,
    snapshot_ns=1_000_000_000,
    official_time_delay_ms=250,
)
fix_reader_registry: fix.FixRegistry = fix_reader.registry
fix_reader_separator: int | None = fix_reader_pinned.separator
fix_reader_payload_column: str = fix_reader_pinned.payload_column
fix_reader_null_values: list[str] = fix_reader_pinned.null_values
fix_reader_direction: str | None = fix_reader_pinned.direction
fix_reader_batch_byte_size: int = fix_reader_pinned.batch_byte_size
fix_reader_snapshot_ns: int | None = fix_reader_pinned.snapshot_ns
fix_reader_official_time_delay_ms: int = fix_reader_pinned.official_time_delay_ms
fix_reader_default_sending_time: Scalar | None = fix_reader_pinned.default_sending_time
fix_reader_native_clock: fix.FixCodec = fix.FixCodec(
    fix_registry_from_fields,
    default_sending_time=DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000),
)
fix_reader_unpinned_clock: fix.FixCodec = fix.FixCodec(default_sending_time=None)
fix_read_messages: fix.FixMessages = fix_reader.parse_line(b"8=FIX.4.4|35=D|10=0|")
fix_read_text: fix.FixMsg = next(fix_read_messages)
fix_read_bytes: fix.FixMsg = next(fix_reader.parse_line(b"8=FIX.4.4|35=D|10=0|"))
fix_read_lines: fix.FixMessages = fix_reader.parse_lines([b"8=FIX.4.4|35=D|10=0|", bytearray()])
fix_read_line: fix.FixMessages = fix_reader.parse_text_line(TextLine(0, b"35=D|"))
fix_read_text_lines: fix.FixMessages = fix_reader.parse_text_lines([TextLine(0, b"35=D|")])
text_line_text: TextLine = TextLine(1, "35=D|", ["FIX.4.4", None])
text_line_under_options: TextLine = TextLine(2, "35=D|", None, TextOptions())
text_line_mtime: int | None = text_line_under_options.mtime
text_line_bodytype: MimeType = text_line_under_options.bodytype
text_line_identity: Scalar = text_line_text.curruuid
text_line_cross: Scalar = text_line_text.crossuuid
text_line_crosscode: str = text_line_text.crosscode
text_line_hashcode: int = text_line_text.currhashcode
text_line_crosshash: int = text_line_text.crosshashcode
text_line_unix: int = text_line_text.currunix
text_line_seqnum: int = text_line_text.seqnum
text_line_mutable_index: TextLine = TextLine(0, "body")
text_line_mutable_index.index = 1
assert text_line_mtime is None and isinstance(text_line_bodytype, MimeType)
assert isinstance(text_line_identity, Scalar) and isinstance(text_line_cross, Scalar)
assert isinstance(text_line_crosscode, str) and text_line_hashcode >= 0
assert text_line_crosshash >= 0 and text_line_unix == 0
assert text_line_seqnum >= 0 and text_line_mutable_index.seqnum == 1
text_line_body: str = text_line_text.body
text_line_decoded: int = text_line_text.decoded_byte_size
text_line_captures: tuple[str | None, ...] = text_line_text.captures
# A located read answers one identifier at both, narrowed to a `Url` at
# `sourceuri`; a read under a name answers it at `sourceuri` alone, and a line
# a caller holds itself was read under nothing and answers neither.
text_line_sourceuri: Uri | None = text_line_text.sourceuri
text_line_sourceurl: Url | None = text_line_text.sourceurl
assert text_line_sourceuri is None and text_line_sourceurl is None
text_line_text.set_entry_by_path("58", "text")
text_line_text.set_entry_by_path("58", memoryview(b"bytes"))
text_entry_value: str = text_line_text.entry_by_path("58").value
text_entry_value_bytes: bytes = text_line_text.entry_by_path("58").value_bytes
text_entry_key: str = text_line_text.entry_by_path("58").key
text_entry_key_bytes: bytes = text_line_text.entry_by_path("58").key_bytes
fix_read_frame: fix.FixMsg = fix_reader.parse_fix_line(b"8=FIX.4.4")
fix_read_bridge: fix.FixMsg = fix_reader.parse_ullink_line(b"#SYMBOL=TTF")
fix_read_fixml: fix.FixMsg = fix_reader.parse_fixml_line(b"<Order ClOrdID='A'/>")
fix_read_pairs: fix.FixMsg = fix_reader.parse_pairs([("55", "AAPL")])
fix_walked: fix.FixMessages = fix_reader.lifecycle(iter([fix_read_text]))
fix_capture_table: pa.Table = pa.table(
    {"body": pa.array([b"8=FIX.4.4|35=D|10=0|"], pa.binary())}
)
fix_parsed: pa.RecordBatchReader = fix_reader.parse_text_arrow_reader(fix_capture_table)
fix_walked_rows: pa.RecordBatchReader = fix_reader.lifecycle_arrow_reader(fix_parsed)
fix_read_back: fix.FixMessages = fix_reader.messages(fix_walked_rows)
fix_rows: pa.RecordBatchReader = fix_reader.arrow_reader(fix_root, fix_read_back)
fix_book_rows: pa.RecordBatchReader = fix_reader.book_arrow_reader(
    [fix_read_text], snapshot_millis=0, global_=False
)
fix_written: int = fix_reader.write_arrow_reader(fix_rows, io.BytesIO())

fix_counter: Field = Field("nopartyids", "int32")
fix_counter.fix.tag = 453
fix_component: Field = Field("party", DataType.from_fields([fix_field]), nullable=False)
fix_group: Field = yggdryl.serie("parties", fix_component)
fix_group.fix.counter = 453
fix_group.fix.component = "party"
fix_reference: Field = Field("partyid", "null")
fix_reference.fix.field_ref = "partyid"
fix_reference.fix.group = None
fix_reference.fix.component = None
fix_counter_tag: int | None = fix_group.fix.counter
fix_component_name: str | None = fix_group.fix.component
fix_field_reference: str | None = fix_reference.fix.field_ref
fix_group_reference: str | None = fix_reference.fix.group
fix_root.fix.msgtype = "D"
fix_message_code: str | None = fix_root.fix.msgtype
fix_root.fix.msgcat = "ORDR"
fix_message_category: str | None = fix_root.fix.msgcat
fix_root.fix.msgcat = None
fix_root.fix.identifiers = ("38",)
fix_root.fix.identifiers = (name for name in ["OrderQty"])
fix_identifiers: list[str] = fix_root.fix.identifiers
fix_direction: Field = Field("MsgDirection", "utf8")
fix_direction.fix.tag = 385
fix_direction.fix.directions = [
    {"code": "S", "patterns": ["(?i)^TX\\b"]},
    {"code": "R", "patterns": ["(?i)^RX\\b"]},
]
fix_directions: list[FixDirection] = fix_direction.fix.directions
fix_direction_code: str = fix_directions[0]["code"]
fix_direction_patterns: list[str] = fix_directions[0]["patterns"]
fix_side: Field = Field("Side", "utf8")
fix_side.fix.tag = 54
fix_side.fix.codeset = "sidecodeset"
fix_codeset_name: str | None = fix_side.fix.codeset
fix_derived: Field = Field("leavesqty", "float64")
fix_derived.fix.tag = 151
fix_derived.fix.derivation = "orderqty - cumqty"
fix_derivation: str | None = fix_derived.fix.derivation
fix_derived.fix.derivation = None
fix_catalog = fix.FixRegistry.from_fields([fix_counter])
fix_inserted_component: Field | None = fix_catalog.insert(fix_component)
fix_inserted_group: Field | None = fix_catalog.insert(fix_group)
fix_catalog.update(fix_group)
fix_definition: Field = fix_catalog.field_by_name("parties")
fix_optional_definition: Field | None = fix_catalog.get_field_by_name("party")
fix_group_by_counter: Field = fix_catalog.field_by_counter(453)
fix_optional_group: Field | None = fix_catalog.get_field_by_counter(453)
fix_removed_definition: Field | None = fix_catalog.remove("parties")
fix_catalog_snapshot: str = fix_catalog.into_json()
fix_catalog_restored: fix.FixRegistry = fix.FixRegistry.from_json(fix_catalog_snapshot)
fix_catalog_hash: int = fix_catalog.stable_hash()
fix_catalog_copy: fix.FixRegistry = fix_catalog.__copy__()
fix_catalog_pickle: tuple[object, tuple[str]] = fix_catalog.__reduce__()
fix_catalog.set_codeset("sidecodeset", [{"value": "1", "name": "Buy"}])
fix_catalog.merge_codeset(
    "sidecodeset", [{"value": "2", "name": "Sell", "aliases": ["Sold"]}]
)
fix_codeset: list[FixCode] = fix_catalog.codeset("sidecodeset")
fix_optional_codeset: list[FixCode] | None = fix_catalog.get_codeset("sidecodeset")
fix_codeset_of: list[FixCode] | None = fix_catalog.codeset_of(fix_side)
fix_codeset_names: list[str] = fix_catalog.codeset_names()
fix_code_value: str = fix_codeset[0]["value"]
fix_code_name: str = fix_codeset[0]["name"]
fix_code_description: str | None = fix_codeset[0]["description"]
fix_code_aliases: list[str] = fix_codeset[0]["aliases"]
fix_code_group: str | None = fix_codeset[0]["group"]
fix_taken_codeset: list[FixCode] | None = fix_catalog.remove_codeset("sidecodeset")
fix_registered_type: fix.MsgType = fix_catalog.register_msgtype("U1", "CustomMessage")
fix_msgtype: fix.MsgType = fix_registry_loaded.msgtype("D")
fix_optional_msgtype: fix.MsgType | None = fix_registry_loaded.get_msgtype("D")
fix_msgtype_name: str = fix_msgtype.name
fix_msgtype_value: str = fix_msgtype.value
fix_msgtype_category: str | None = fix_msgtype.msgcat
fix_msgtype_field: Field = fix_msgtype.field
fix_msgtype_group: Field | None = fix_msgtype.get_group_by_tag(453)
fix_identifier_values: list[tuple[Field, Scalar]] = fix_msgtype.identifier_values(fix_message)
fix_identifier_field: Field = fix_identifier_values[0][0]
fix_identifier_value: Scalar = fix_identifier_values[0][1]
fix_msgtype_hash: int = fix_msgtype.stable_hash()
fix_msgtype_ordered: bool = fix_msgtype <= fix_msgtype
fix_msgtype_pickle: tuple[object, tuple[str, str]] = fix_msgtype.__reduce__()
fix_fixed_schema: Field = fix.fix_schema(fix_registry_from_fields, "FixMessage")
fix_formatted_rows: list[Scalar] = fix_reader.format_messages([fix_message], fix_fixed_schema)
fix_fixed_tags: list[int] = fix.fix_schema_tags()
fix_crated: list[Field] = fix.fix_crate_fields()
fix_cblock_read: tuple[fix.FixRegistry, list[Field]] = fix.FixRegistry.from_cfb_file(
    "cblocks/bloomberg.cfb"
)
fix_cblock: list[Field] = list(fix_cblock_read[0])
fix_added_field: bool = fix_registry_from_fields.add_field(fix_field)
fix_folded: tuple[int, int] = fix_registry_from_fields.add_fields(fix_cblock)
fix_combined: tuple[int, int] = fix_registry_from_fields.merge_with(fix_registry_loaded)
fix_ingested: tuple[int, int] = fix_registry_from_fields.add_cfb_file(
    Path("cblocks") / "bloomberg.cfb", "bloomberg"
)
fix_globbed: tuple[int, int, int] = fix_registry_from_fields.add_cfb_files(
    Path("cblocks"), "*.cfb"
)
fix_snapshot_folded: tuple[int, int] = fix_registry_from_fields.add_json_file(
    Path("dictionaries") / "venue.json"
)
fix_read_cblock: tuple[fix.FixRegistry, list[Field]] = fix.FixRegistry.from_cfb_file(
    Path("cblocks") / "bloomberg.cfb", dialect="bloomberg"
)
fix_carried_schema: Field = fix.fix_schema_carrying(fix_root, fix_fixed_schema)
fix_column_at: int | None = fix_fixed_schema.index_of("msgtype")
fix_fixed_row: Scalar = fix_read_text.into_row(fix_fixed_schema)

fix_global: fix.FixRegistry = fix.global_registry()
fix.install_global_registry(fix_registry_from_fields)

assert fix_tag == 38 and fix_tags and fix_names and fix_description
assert fix_branches == ["bloomberg", "cme", "ice"] and fix_has_branch
assert fix_dialects
assert fix_id is not None and fix_vendor_id is not None
assert fix_direction_code == "S"
assert fix_direction_patterns == ["(?i)^TX\\b"] and len(fix_directions) == 2
assert fix_derivation == "orderqty - cumqty" and fix_derived.fix.derivation is None
assert fix_read_cblock[0] is not None
assert python_declared is not None and python_declared.kind == "field"
assert python_module == "trading.execution" and python_qualname == "Book.Fill"
assert python_class_name == "Fill" and python_kind == "field"
assert python_import_path == "trading.execution.Book.Fill"
assert python_declared_module == "trading.book" and python_declared_kind == "class"
assert python_declared_qualname == python_declared_class_name == "Quote"
assert python_declared_path == "trading.book.Quote" and python_declared_importable
assert not python_declared_keyword and python_declared_hash
assert python_declared_properties["PYTHON:module"] == "trading.book"
assert python_from_type.class_name == "Field"
assert python_properties.scheme == "python"
assert fix_by_id and fix_maybe_by_id
assert fix_registry_loaded is not None and fix_registry_from_url is not None
assert fix_registry_from_handle is not None and fix_registry_from_text is not None
assert fix_by_tag and fix_maybe_by_tag and fix_by_name and fix_maybe_by_name
assert fix_by_path and fix_maybe_by_path and fix_generic and fix_maybe_generic
assert fix_bytes_protocol and fix_text_protocol
assert fix_bytes_msgtype is None or fix_bytes_msgtype
assert fix_text_msgtype is None or fix_text_msgtype
assert fix_item and fix_default is None or fix_default
assert fix_replaced is None or fix_replaced
assert len(fix_message_digest) == 16 and isinstance(fix_message_wire, bytes)
assert isinstance(fix_message_wire_text, str)
assert isinstance(fix_message_event, fix.OperationEventData)
assert isinstance(fix_message_header, fix.FixHeader)
assert isinstance(fix_message_capture, fix.FixCapture)
assert fix_message_text is None or fix_message_text
assert fix_message_msgcat is None or isinstance(fix_message_msgcat, int)
assert fix_message_marketoperationid is None or isinstance(fix_message_marketoperationid, int)
assert isinstance(fix_message_metadata, dict) and isinstance(fix_message_altids, dict)
assert isinstance(fix_message_accountids, dict) and isinstance(fix_message_userids, dict)
assert isinstance(fix_message_securityids, dict) and isinstance(fix_message_unit, str)
assert fix_message_cficode is None or fix_message_cficode
assert fix_message_miccode is None or fix_message_miccode
assert fix_message_lastpx is None or isinstance(fix_message_lastpx, Scalar)
assert fix_message_lastqty is None or isinstance(fix_message_lastqty, Scalar)
assert fix_message_avgpx is None or isinstance(fix_message_avgpx, Scalar)
assert fix_message_cumqty is None or isinstance(fix_message_cumqty, Scalar)
assert fix_message_leavesqty is None or isinstance(fix_message_leavesqty, Scalar)
assert fix_message_prevpx is None or isinstance(fix_message_prevpx, Scalar)
assert fix_message_prevqty is None or isinstance(fix_message_prevqty, Scalar)
assert fix_message_spotrate is None or isinstance(fix_message_spotrate, Scalar)
assert fix_message_forwardpoints is None or isinstance(fix_message_forwardpoints, Scalar)
assert fix_message_ticker is None or isinstance(fix_message_ticker, str)
assert fix_message_tif is None or isinstance(fix_message_tif, str)
assert fix_message_tradable is None or isinstance(fix_message_tradable, bool)
assert fix_message_bid is None or isinstance(fix_message_bid, dict)
assert fix_message_ask is None or isinstance(fix_message_ask, dict)
assert isinstance(fix_message_curruuid, Scalar) and isinstance(fix_message_crossuuid, Scalar)
assert isinstance(fix_message_currhashcode, int) and isinstance(fix_message_crosshashcode, int)
assert isinstance(fix_message_currunix, int) and isinstance(fix_message_seqnum, int)
assert isinstance(fix_message_state, Scalar) and isinstance(fix_message_side, Scalar)
assert isinstance(fix_message_price, Scalar) and isinstance(fix_message_quantity, Scalar)
assert isinstance(fix_message_currency, Scalar)
assert fix_message_prevuuid is None or fix_message_prevuuid
assert isinstance(fix_message_crosscode, str)
assert isinstance(fix_message_srcuuids, list) and isinstance(fix_event_srcuuids, list)
assert isinstance(fix_message_entries, list)
assert isinstance(fix_header_beginstring, str) and isinstance(fix_header_msgtype, str)
assert isinstance(fix_header_sendingtime, int) and isinstance(fix_header_stated, bool)
assert fix_header_sendercompid is None or fix_header_sendercompid
assert fix_header_targetcompid is None or fix_header_targetcompid
assert fix_header_msgseqnum is None or fix_header_msgseqnum
assert fix_header_possdupflag is None or fix_header_possdupflag
assert fix_header_msgdirection is None or fix_header_msgdirection
assert fix_capture_msgpluginid is None or fix_capture_msgpluginid
assert fix_capture_msgctxid is None or fix_capture_msgctxid
assert fix_capture_msgsessionid is None or fix_capture_msgsessionid
assert fix_capture_msgsesseventid is None or fix_capture_msgsesseventid
assert isinstance(fix_event_currunix, int) and isinstance(fix_event_crosscode, str)
assert isinstance(fix_event_currhashcode, int) and isinstance(fix_event_crosshashcode, int)
assert isinstance(fix_event_seqnum, int) and isinstance(fix_event_unit, str)
assert fix_event_creaunix is None or isinstance(fix_event_creaunix, int)
assert fix_event_execunix is None or isinstance(fix_event_execunix, int)
assert fix_event_recdunix is None or isinstance(fix_event_recdunix, int)
assert fix_event_exprtime is None or isinstance(fix_event_exprtime, int)
assert fix_event_prevunix is None or isinstance(fix_event_prevunix, int)
assert fix_event_snapunix is None or isinstance(fix_event_snapunix, int)
assert fix_event_marketoperationid is None or isinstance(fix_event_marketoperationid, int)
assert fix_event_prevuuid is None or fix_event_prevuuid
assert fix_event_lastpx is None or isinstance(fix_event_lastpx, Scalar)
assert fix_event_lastqty is None or isinstance(fix_event_lastqty, Scalar)
assert fix_event_avgpx is None or isinstance(fix_event_avgpx, Scalar)
assert fix_event_cumqty is None or isinstance(fix_event_cumqty, Scalar)
assert fix_event_leavesqty is None or isinstance(fix_event_leavesqty, Scalar)
assert fix_event_prevpx is None or isinstance(fix_event_prevpx, Scalar)
assert fix_event_prevqty is None or isinstance(fix_event_prevqty, Scalar)
assert fix_event_tif is None or isinstance(fix_event_tif, str)
assert fix_event_tradable is None or isinstance(fix_event_tradable, bool)
assert fix_event_ticker is None or isinstance(fix_event_ticker, str)
assert fix_event_cficode is None or fix_event_cficode
assert fix_event_miccode is None or fix_event_miccode
assert fix_event_spotrate is None or isinstance(fix_event_spotrate, Scalar)
assert fix_event_forwardpoints is None or isinstance(fix_event_forwardpoints, Scalar)
assert fix_event_bid is None or isinstance(fix_event_bid, dict)
assert fix_event_ask is None or isinstance(fix_event_ask, dict)
assert isinstance(fix_event_securityids, dict) and isinstance(fix_event_metadata, dict)
assert isinstance(fix_event_accountids, dict) and isinstance(fix_event_userids, dict)
assert isinstance(fix_event_altids, dict)
assert isinstance(fix_event_curruuid, Scalar) and isinstance(fix_event_crossuuid, Scalar)
assert isinstance(fix_event_state, Scalar) and isinstance(fix_event_side, Scalar)
assert isinstance(fix_event_price, Scalar) and isinstance(fix_event_quantity, Scalar)
assert isinstance(fix_event_currency, Scalar)
assert fix_walked is not None and fix_walked_rows is not None and fix_book_rows is not None
assert fix_written >= 0
assert fix_reader_registry is not None and fix_reader_pinned is not None
assert fix_read_text and fix_read_bytes and fix_read_frame
assert fix_read_bridge is not None and fix_read_pairs is not None
assert fix_fixed_schema and fix_fixed_tags and fix_crated
assert fix_carried_schema is not None and fix_column_at is not None
assert fix_fixed_row is not None
assert fix_removed is None or fix_removed
assert fix_removed_by_id is None or fix_removed_by_id
assert fix_size >= 0 and fix_has or not fix_has
assert fix_field_names == [] or fix_field_names
assert fix_message_registry and fix_message_field and fix_message_value
assert fix_message_by_id and fix_message_maybe_id
assert fix_message_by_tag and fix_message_maybe_tag
assert fix_message_by_name and fix_message_maybe_name
assert fix_message_by_path and fix_message_maybe_path
assert fix_message_item and (fix_message_default is None or fix_message_default)
assert fix_message_pairs == [] or fix_message_pairs
assert fix_message_len >= 0 and fix_message_hash
assert fix_global is not None and fix_message_explicit == fix_message_explicit
assert role_path is not None and role_file is not None and role_folder is not None
assert role_temporary is not None and role_home is not None and role_config is not None
assert role_cached is not None
assert coding_roles and encoding_roles and storage_roles
assert role_fs_path is not None and role_fs_file is not None
assert role_fs_folder is not None and role_created is not None

# The typed door for a value. A typed field alias already names the width,
# unit, scale and zone, so `.scalar(value)` is how a caller reaches an exact
# temporal without a second constructor vocabulary - and it has to narrow, not
# just execute.
typed_instant_field = temporal.datetime64("at", "ns", "UTC")
typed_instant: Scalar = typed_instant_field.scalar(
    datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
)
typed_instant_dtype: Scalar = typed_instant_field.dtype.scalar(1)
assert typed_instant.kind == "datetime64"
assert typed_instant_dtype.kind == "datetime64"

# A serie: a run by construction, a column by its field, and every nested
# column handed out as the class its leaf is named for.
serie_run: yggdryl.Serie = yggdryl.Serie([1, "a", None])
serie_column: yggdryl.Serie = yggdryl.Serie.from_scalars(Field("price", "int64"), [1, 2])
serie_row: Scalar = serie_column[0]
serie_window: yggdryl.Serie = serie_column[1:]
serie_arrow: pa.Array = serie_column.into_arrow_array()
serie_rows: list[Scalar] = serie_column.rows()
serie_field: Field | None = serie_column.field
serie_legs = yggdryl.Serie.from_arrow_array(pa.array([[1, 2], [3]], pa.list_(pa.int64())))
assert isinstance(serie_legs, yggdryl.SerieSerie)
serie_offsets: list[int] = serie_legs.offsets
serie_leg: yggdryl.Serie | None = serie_legs.row(0)
serie_range: tuple[int, int] | None = serie_legs.range(1)
serie_books = yggdryl.Serie.from_arrow_array(
    pa.array([[("AAPL", 1)]], pa.map_(pa.utf8(), pa.int64()))
)
assert isinstance(serie_books, yggdryl.MapSerie)
serie_entries: yggdryl.StructSerie = serie_books.entries
serie_names: list[str] = serie_entries.names
serie_scalar_serie: yggdryl.Serie | None = Scalar.from_([1]).as_serie()
assert len(serie_run) == 3 and serie_row is not None and serie_window is not None
assert serie_arrow is not None and serie_rows and serie_field is not None
assert serie_offsets and serie_leg is not None and serie_range is not None
assert serie_names and serie_scalar_serie is not None

# A chunked serie: many columns under one field, held apart - a chunked
# array, or a table of one batch per chunk.
chunked_prices: yggdryl.ChunkedSerie = yggdryl.ChunkedSerie.from_arrow_chunked_array(
    pa.chunked_array([[1, 2], [3]]), Field("price", "int64"), safe=False
)
chunked_table: yggdryl.ChunkedSerie = yggdryl.ChunkedSerie.from_arrow_reader(
    pa.Table.from_batches([source_batch, source_batch]), nullability="strict"
)
chunked_from: yggdryl.ChunkedSerie = yggdryl.ChunkedSerie.from_(source_batch)
chunked_series: yggdryl.ChunkedSerie = yggdryl.ChunkedSerie.from_series(
    [serie_column], Field("price", "int64"), representation="bits"
)
chunked_empty: yggdryl.ChunkedSerie = yggdryl.ChunkedSerie.empty(Field("price", "int64"))
chunked_one: yggdryl.ChunkedSerie = yggdryl.ChunkedSerie.from_serie(serie_column)
chunked_chunks: list[yggdryl.Serie] = chunked_prices.chunks
chunked_chunk: yggdryl.Serie | None = chunked_prices.chunk(0)
chunked_count: int = chunked_prices.num_chunks
chunked_field: Field = chunked_prices.field
chunked_dtype: DataType = chunked_prices.dtype
chunked_row: Scalar = chunked_prices[0]
chunked_window: yggdryl.ChunkedSerie = chunked_prices[1:]
chunked_slice: yggdryl.ChunkedSerie = chunked_prices.slice(0, 2)
chunked_column: yggdryl.ChunkedSerie | None = chunked_table.child("value")
chunked_children: list[yggdryl.ChunkedSerie] = chunked_table.children()
chunked_path: yggdryl.ChunkedSerie | None = chunked_table.get_child_by_path("value")
chunked_joined: yggdryl.Serie = chunked_prices.into_serie()
chunked_cast: yggdryl.ChunkedSerie = chunked_prices.cast(DataType("float64"), safe=False)
chunked_arrow: pa.ChunkedArray = chunked_prices.into_arrow_chunked_array()
chunked_arrow_table: pa.Table = chunked_table.into_arrow_table()
chunked_arrow_reader: pa.RecordBatchReader = chunked_table.into_arrow_reader()
chunked_rows: list[Scalar] = chunked_prices.rows()
chunked_values: list[Any] = chunked_prices.as_py()
chunked_get: Scalar | None = chunked_prices.get(5)
chunked_prices.push_chunk(serie_column, nullability="strict")
chunked_reader: SerieReader = SerieReader.from_chunked(chunked_prices)
chunked_plan: yggdryl.ChunkedSerie = ArrowCastPlan(
    Field("price", "int64"), Field("price", "float64")
).apply(chunked_prices)
chunked_serie_plan: Serie = ArrowCastPlan(
    Field("price", "int64"), Field("price", "float64")
).apply(serie_column)
chunked_equal: bool = chunked_prices == serie_column
chunked_ordered: bool = chunked_prices < serie_column
assert chunked_count == 2 and chunked_chunks and chunked_chunk is not None
assert chunked_field is not None and chunked_dtype is not None and chunked_row is not None
assert len(chunked_window) == 2 and len(chunked_slice) == 2 and chunked_column is not None
assert chunked_children and chunked_path is not None and chunked_joined is not None
assert chunked_cast is not None and chunked_arrow is not None and chunked_rows
assert chunked_arrow_table is not None and chunked_arrow_reader is not None
assert chunked_values and chunked_get is None and chunked_reader is not None
assert chunked_plan is not None and chunked_serie_plan is not None
assert chunked_from is not None and chunked_series is not None and chunked_empty.is_empty()
assert chunked_one is not None and not chunked_equal and not chunked_ordered
