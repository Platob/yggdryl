from __future__ import annotations

import os
import io
from collections.abc import Iterator, Mapping
from pathlib import Path
from typing import Any, Literal

import pyarrow as pa  # type: ignore[import-untyped]

from yggdryl import (
    Bound,
    DataType,
    Expression,
    Field,
    IOBase,
    MediaType,
    MimeType,
    ProtocolMetadata,
    Record,
    RecordOptions,
    Statement,
    Uri,
    Url,
    Urn,
    fields,
    gzip,
    iceberg,
    json,
    toml,
    yaml,
    zlib,
    zstd,
)
from yggdryl.fields import (
    FixedSizeListField,
    Int32Field,
    ListField,
    TimeField,
    VariantField,
)

file_uri: Uri = Uri.from_path(Path("data/events.parquet"))
file_url: Url = file_uri.to_url()
path: str = file_uri.to_path()
path_protocol: str = os.fspath(file_url)

urn: Urn = Uri("urn:isbn:9780131103627").to_urn()
uri_again: Uri = urn.to_uri()
mime_type: MimeType = file_uri.mime_type
media_type: MediaType = file_uri.media_type
mime_format: Literal["json", "json_lines", "yaml", "toml"] | None = mime_type.format
content_coding: Literal["gzip", "compress", "deflate", "br", "zstd"] | None = (
    MimeType.GZIP.content_coding
)
stem: str | None = file_uri.stem
file_uri.set_file_name("events.parquet")
file_uri.set_stem("events")
file_uri.set_extension("json")
file_uri.set_extensions(["json", "gz"])
file_uri.set_mime_type(MimeType.JSON)
file_uri.set_media_type(MediaType.from_parts(MimeType.CSV, [MimeType.GZIP]))
removed_extension: bool = file_uri.remove_extension()
cleared_extensions: bool = file_uri.clear_extensions()

field = Field("event", "string", nullable=False)
field.set_alias("payload")
field.set_catalog_name("analytics")
field.set_schema_name("public")
field.set_table_name("events")
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
data_type_scalar: pa.Scalar = DataType("int32").arrow_scalar(1)
field_scalar: pa.Scalar = field.arrow_scalar("payload")
default_data_type_scalar: pa.Scalar = DataType("int32").default_arrow_scalar()
default_field_scalar: pa.Scalar = field.default_arrow_scalar()
source_array = pa.array([1, 2], type=pa.int32())
cast_data_type_array: pa.Array = DataType("int64").cast_arrow_array(source_array)
cast_field_array: pa.Array = Field("value", "int64").cast_arrow_array(source_array)
source_batch = pa.record_batch([source_array], names=["value"])
cast_data_type_batch: pa.RecordBatch = DataType.from_fields(
    [Field("value", "int64")]
).cast_arrow_batch(source_batch)
cast_field_batch: pa.RecordBatch = Field(
    "rows", DataType.from_fields([Field("value", "int64")]), nullable=False
).cast_arrow_batch(source_batch)
default_data_type_value: object = DataType("int32").default_pyvalue()
default_field_value: object = field.default_pyvalue()
default_data_type_hint: object = DataType("int32").default_pyhint()
default_field_hint: object = field.default_pyhint()
arrow_compatible: DataType = DataType("uint32").to_scheme_compat("arrow")
spark_compatible: Field = field.to_scheme_compat("spark")
polars_compatible: Field = field.to_scheme_compat("polars")
pandas_compatible: Field = field.to_scheme_compat("pandas")
iceberg_compatible: Field = field.to_scheme_compat("iceberg")
typed_id: Int32Field = fields.int32("id", nullable=False)
typed_id_kind: Literal["int32"] = typed_id.data_type.id
typed_id_value: int | None = typed_id.default_pyvalue()
typed_id_data_type_value: int = typed_id.data_type.default_pyvalue()
typed_id_hint: object = typed_id.default_pyhint()
typed_id_data_type_hint: object = typed_id.data_type.default_pyhint()
typed_clock: TimeField = fields.time("clock", "microseconds", nullable=False)
typed_ids: ListField[int] = fields.list("ids", typed_id)
nullable_item: Int32Field = fields.int32("item")
typed_fixed: FixedSizeListField[int] = fields.fixed_size_list(
    "fixed", nullable_item, 2, nullable=False
)
typed_fixed_value: list[int | None] | None = typed_fixed.default_pyvalue()
typed_fixed_data_type_value: list[int | None] = (
    typed_fixed.data_type.default_pyvalue()
)
typed_struct = fields.struct("row", [typed_id], nullable=False)
typed_struct_value: Record | Mapping[str, object] | None = (
    typed_struct.default_pyvalue()
)
typed_struct_data_type_value: Record | Mapping[str, object] = (
    typed_struct.data_type.default_pyvalue()
)
variant_data_type: DataType = DataType.variant(
    [
        fields.int64("integer", nullable=False),
        fields.utf8("text", nullable=False),
    ]
)
typed_variant: VariantField = fields.variant(
    "payload",
    tuple(variant_data_type),
    nullable=False,
)
typed_variant_kind: Literal["union"] = typed_variant.data_type.id
typed_variant_value: object = typed_variant.default_pyvalue()

# These are deliberate negative checks. Under ``mypy --strict``, each ignore
# becomes unused if a typed view regresses to ``Any`` or drops its nullable /
# Record branch.
field_default_cannot_be_assumed_present: int = (
    nullable_item.default_pyvalue()  # type: ignore[assignment]
)
fixed_children_cannot_be_assumed_present: list[int] = (
    typed_fixed.data_type.default_pyvalue()  # type: ignore[assignment]
)
struct_default_is_not_always_a_mapping: Mapping[str, object] = (
    typed_struct.data_type.default_pyvalue()  # type: ignore[assignment]
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
invalid_compatibility_target = field.to_scheme_compat("mysql")  # type: ignore[arg-type]
inferred_dictionary: Field = fields.dictionary("labels", int, str)
inferred_mapping: Field = fields.map_of("counts", str, pa.int32())
field_differences: list[str] = list(field.show_diffs(typed_id, False))
json_source: json.Source = io.BytesIO(b'{"value":42}')
yaml_source: yaml.Source = io.StringIO("value: 42\n")
toml_source: toml.Source = io.StringIO("value = 42\n")
json_destination: json.Destination = io.BytesIO()
yaml_destination: yaml.Destination = Path("value.yaml")
toml_destination: toml.Destination = io.StringIO()
decoded_json: dict[str, int] = json.load(json_source)
decoded_yaml: dict[str, int] = yaml.loads(yaml_source)
decoded_toml: dict[str, int] = toml.load(toml_source)
encoded_json: bytes = json.dumps(decoded_json)
encoded_yaml: bytes = yaml.dumps(decoded_yaml)
encoded_toml: bytes = toml.dumps(decoded_toml)
json.dump(decoded_json, json_destination)
yaml.dump(decoded_yaml, yaml_destination)
toml.dump(decoded_toml, toml_destination)

alias: str | None = field.alias
location: Url | None = field.location
property_value: str | None = field.get_property("postgres", "type")
properties: list[tuple[str, str]] = list(field.property_iter("postgres"))
iceberg_properties: ProtocolMetadata = field.iceberg
postgres_properties: ProtocolMetadata = field.protocol("POSTGRES")
protocol_scheme: str = iceberg_properties.scheme
protocol_prefix: str = iceberg_properties.prefix
protocol_key: str = iceberg_properties.key("doc")
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
assert data_type_scalar
assert field_scalar
assert default_data_type_scalar
assert default_field_scalar
assert default_data_type_value == 0
assert default_field_value == ""
assert default_data_type_hint
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
record_options: RecordOptions = record_handle.record_options()
record_options.batch_size = 1024
record_options.root_name = "trade"
record_options.safe = True
record_mime_type: MimeType = record_options.mime_type
declared_schema: Field | None = record_options.schema
record_options.schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
record_options.merge_by_names = ["id"]
record_match_key: list[str] = record_options.merge_by_names
record_batches: pa.RecordBatchReader = record_handle.read_arrow_batch_reader(
    options=record_options,
)
stored_root: Field = record_handle.read_arrow_field()
record_handle.write_arrow_batch_reader(record_batches, options=record_options)
record_handle.append_arrow_batch_reader(record_batches, options=record_options)

line_batches: pa.RecordBatchReader = record_handle.read_arrow_lines(
    r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<level>[^\]]+)\]",
    batch_size=512,
    custom_fields={"venue": "XNAS", "session": 7},
    timestamp_capture=None,
)

generic_batches: pa.RecordBatchReader = record_handle.read_arrow(options=record_options)
record_handle.write_arrow(pa.table({"id": [1]}))
record_handle.write_arrow([{"id": 1}], options=record_options)
record_handle.append_arrow(generic_batches)
pandas_frames: Iterator[Any] = record_handle.read_pandas()
pandas_frame: Any = record_handle.read_pandas_frame(options=record_options)
record_handle.write_pandas(pandas_frames)
record_handle.write_pandas_frame(pandas_frame, options=record_options)
polars_frames: Iterator[Any] = record_handle.read_polars()
polars_frame: Any = record_handle.read_polars_frame(options=record_options)
record_handle.write_polars(polars_frames)
record_handle.write_polars_frame(polars_frame, options=record_options)
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

assert record_mime_type
assert declared_schema is None or declared_schema
assert stored_root
assert row_group_size
assert footer_metadata is None or footer_metadata == {}
assert iceberg_spec.is_unpartitioned()
assert iceberg_scan
assert iceberg_snapshot is None or iceberg_snapshot.operation
assert iceberg_files == [] or iceberg_files
assert iceberg_manifests == [] or iceberg_manifests
assert iceberg_evolved >= 0

iceberg_document: dict[str, object] = iceberg.schema_to_json(iceberg_schema)
iceberg_reread: Field = iceberg.schema_from_json("row", iceberg_document)

assert iceberg_document
assert iceberg_reread

# The flattened record-option keywords type-check as real named parameters.
record_handle.write_arrow(
    pa.table({"id": [1]}),
    merge_by_names=["id"],
    select_by_names=["id"],
    batch_size=1024,
    safe=False,
    root_name="record",
)
record_handle.append_arrow(pa.table({"id": [1]}), filter_partitions={"venue": "XNAS"})
kwargs_reader: pa.RecordBatchReader = record_handle.read_arrow(
    select_by_names=["id"], batch_size=1024
)
kwargs_root: Field = record_handle.read_arrow_field(root_name="record")
record_handle.write_arrow_batch_reader(
    pa.table({"id": [1]}),
    options=record_options,
    compression="zstd(3)",
    max_row_group_size=1024,
    key_value_metadata={"writer": "typing"},
)

assert kwargs_reader
assert kwargs_root

# Iceberg keeps its own options type, flattened the same way.
iceberg_options: iceberg.IcebergOptions = iceberg.IcebergOptions(
    commit_retries=2, target_file_size=1024, data_format="avro"
)
iceberg_retries: int = iceberg_options.commit_retries
iceberg_format: str = iceberg_options.data_format
iceberg_options.data_format = "parquet"
iceberg_table.append(pa.table({"id": [1]}), options=iceberg_options, commit_retries=1)
iceberg_table.overwrite(pa.table({"id": [1]}), data_format="avro")
iceberg_table.set_options(target_file_size=2048)
iceberg_resolved: iceberg.IcebergOptions = iceberg_table.options()
iceberg_options_scan: pa.RecordBatchReader = iceberg_table.scan(
    options=iceberg_options, read_parallelism=2
)

assert iceberg_retries >= 0
assert iceberg_format
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
    "orders", pa.table({"id": [1]}), data_format="avro"
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
    [("venue", "XNAS")], iceberg_schema, options=iceberg_options, read_parallelism=2
)
unfiltered_scan: pa.RecordBatchReader = iceberg_table.scan_where()
branch_scan: pa.RecordBatchReader = iceberg_table.scan_ref("nightly")
branch_projection: pa.RecordBatchReader = iceberg_table.scan_ref(
    "nightly", {"venue": "XNAS"}, iceberg_schema, data_format="avro"
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

# The scoped writes take the filters first and the same flattened keywords.
iceberg_table.overwrite_where(
    {"venue": "XNAS"}, pa.table({"id": [1]}), data_format="avro"
)
iceberg_table.overwrite_where(None, pa.table({"id": [1]}))
iceberg_table.merge(pa.table({"id": [1]}), ["id"])
iceberg_table.merge(pa.table({"id": [1]}), ["id"], safe=False, commit_retries=1)
iceberg_table.merge_where(
    [("venue", "XNAS")],
    pa.table({"id": [1]}),
    ["id"],
    safe=True,
    options=iceberg_options,
    target_file_size=1024,
)

# Maintenance answers with the identifiers it acted on, or with nothing.
expired_snapshots: list[int] = iceberg_table.expire_snapshots(0)
iceberg_table.fast_forward("nightly", 1)
snapshot_manifests: list[iceberg.ManifestFile] = iceberg_table.manifests_at(1)

assert expired_snapshots == [] or expired_snapshots
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
expression_restored: Expression = Expression.from_json(expression.to_json())
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
expression_document: str = expression.to_json()
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
statement_restored: Statement = Statement.from_json(statement.to_json())
statement_projections: list[str] = statement.projections
statement_predicate: Expression | None = statement.predicate
statement_limit: int | None = statement.limit

expression_matched: list[IOBase] = IOBase("file:///lake").children_matching(
    "&holder.partition['year'] = '2024'"
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
assert statement_limit is None or statement_limit
assert expression_matched == [] or expression_matched
