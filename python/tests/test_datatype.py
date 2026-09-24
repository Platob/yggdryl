"""The native datatype view, and the factories that spell one."""

from __future__ import annotations

import copy
import decimal
import enum
import gc
import inspect
import json
import operator
import pickle
import uuid
from collections.abc import Iterator
from typing import Optional

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import (
    BytesParameters,
    DataType,
    Field,
    Serie,
    StringEnum,
    StringParameters,
    Version,
    enums,
)

def test_dtype_infers_native_string_and_arrow_scalars() -> None:
    expected = DataType("int64")

    assert DataType(expected) == expected
    assert DataType.from_value("int64") == expected
    assert DataType.from_value(pa.int64()) == expected
    assert DataType.from_arrow(pa.int64()) == expected
    assert expected.into_arrow() == pa.int64()


def test_dtype_builds_and_casts_exact_arrow_scalars() -> None:
    dtype = DataType("int8")
    exact = pa.scalar(7, type=pa.int8())

    assert dtype.arrow_scalar(exact) is exact
    assert dtype.arrow_scalar(7).equals(exact)
    assert dtype.arrow_scalar(None).equals(pa.scalar(None, type=pa.int8()))
    assert dtype.arrow_scalar(pa.scalar(7, type=pa.int64())).equals(exact)

    with pytest.raises((pa.ArrowInvalid, OverflowError)):
        dtype.arrow_scalar(130)
    assert dtype.arrow_scalar(130, safe=False).as_py() == -126
    assert dtype.arrow_scalar("7", safe=False).equals(exact)

    with pytest.raises(TypeError):
        dtype.arrow_scalar(7, False)  # type: ignore[misc]


def test_dtype_arrow_scalar_handles_nested_dictionary_map_and_run_end() -> None:
    nested = DataType.from_arrow(
        pa.struct([pa.field("items", pa.list_(pa.int8()), nullable=False)])
    )
    nested_scalar = nested.arrow_scalar({"items": [1, 2]}, safe=False)
    assert nested_scalar.type.equals(nested.into_arrow())
    assert nested_scalar.as_py() == {"items": [1, 2]}

    mapping = DataType.from_arrow(pa.map_(pa.string(), pa.int8()))
    map_scalar = mapping.arrow_scalar([("left", 1), ("right", 2)], safe=False)
    assert map_scalar.type.equals(mapping.into_arrow())
    assert map_scalar.as_py() == [("left", 1), ("right", 2)]

    dictionary = DataType.from_arrow(pa.dictionary(pa.int8(), pa.string()))
    dictionary_scalar = dictionary.arrow_scalar("ready", safe=False)
    assert dictionary_scalar.type.equals(dictionary.into_arrow())
    assert dictionary_scalar.as_py() == "ready"

    run_end = DataType.from_arrow(pa.run_end_encoded(pa.int16(), pa.int64()))
    run_end_scalar = run_end.arrow_scalar(42)
    assert run_end_scalar.type.equals(run_end.into_arrow())
    assert run_end_scalar.as_py() == 42


def test_dtype_infers_python_types_without_stringifying_objects() -> None:
    assert DataType(str) == DataType("utf8")
    assert DataType(bool) == DataType("bool")
    assert DataType(int) == DataType("int64")
    assert DataType(float) == DataType("float64")
    assert DataType(bytes) == DataType("binary")
    assert DataType(decimal.Decimal) == DataType("decimal128(38,18)")
    assert DataType(list[str]) == DataType.from_pyhint(list[str])
    assert DataType(Optional[int]) == DataType("int64")

    with pytest.raises(TypeError, match="unsupported Python type hint"):
        DataType(object())


def test_decimal_infers_storage_width_and_integer_like_arguments() -> None:
    class IndexValue:
        def __init__(self, value: int) -> None:
            self.value = value

        def __index__(self) -> int:
            return self.value

    assert DataType.decimal(9) == DataType("decimal32(9,0)")
    assert DataType.decimal(18) == DataType("decimal64(18,0)")
    assert str(inspect.signature(DataType.decimal)) == "(precision, scale=0)"
    assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
    assert DataType.decimal("39", "-4") == DataType("decimal256(39,-4)")
    assert DataType.decimal(IndexValue(39), IndexValue(-4)) == DataType(
        "decimal256(39,-4)"
    )
    assert str(DataType.decimal("18", 2)) == "decimal64(18,2)"

    with pytest.raises(TypeError, match="precision.*not bool"):
        DataType.decimal(True, 0)
    with pytest.raises(TypeError, match="scale.*float"):
        DataType.decimal(18, 2.0)
    with pytest.raises(TypeError, match="scale.*NoneType"):
        DataType.decimal(18, None)  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="precision.*base-10 integer string"):
        DataType.decimal("18.0", 2)
    with pytest.raises(ValueError, match="positive scale cannot exceed precision"):
        DataType.decimal(2, "3")
    with pytest.raises(OverflowError, match="precision.*unsigned byte"):
        DataType.decimal(256, 0)
    with pytest.raises(OverflowError, match="precision.*supported integer range"):
        DataType.decimal("9" * 100, 0)
    with pytest.raises(OverflowError, match="scale.*supported integer range"):
        DataType.decimal(18, "-" + "9" * 100)


def test_time_infers_storage_width_from_native_unit_aliases() -> None:
    assert str(inspect.signature(DataType.time)) == "(unit)"
    assert DataType.time("s") == DataType("time32(s)")
    assert DataType.time("milli seconds") == DataType("time32(ms)")
    assert DataType.time("µs") == DataType("time64(us)")
    assert DataType.time("NANO-SECONDS") == DataType("time64(ns)")

    with pytest.raises(ValueError, match="temporal resolution"):
        DataType.time("year_month")
    with pytest.raises(ValueError, match="unknown temporal resolution"):
        DataType.time("fortnight")
    with pytest.raises(TypeError):
        DataType.time(1)  # type: ignore[arg-type]


def test_dtype_string_json_order_hash_and_pickle_protocols() -> None:
    value = DataType.from_arrow(pa.decimal128(18, 4))

    assert DataType.from_str(str(value)) == value
    assert DataType.from_json(value.into_json()) == value
    assert eval(repr(value), {"DataType": DataType}) == value
    assert copy.copy(value) == value
    assert pickle.loads(pickle.dumps(value)) == value
    assert hash(value) == hash(DataType(value))
    assert value.stable_hash() == DataType(value).stable_hash()
    assert DataType("int32") < DataType("int64") or DataType("int64") < DataType("int32")


def test_dtype_is_a_read_only_nested_field_collection() -> None:
    arrow_type = pa.struct(
        [
            pa.field("symbol", pa.string(), nullable=False),
            pa.field("levels", pa.list_(pa.float64())),
        ]
    )
    value = DataType.from_value(arrow_type)

    assert len(value) == 2
    assert [field.name for field in value] == ["symbol", "levels"]
    assert value[0].name == "symbol"
    assert value[-1].name == "levels"
    assert value["levels"].dtype.into_arrow() == pa.list_(pa.float64())
    assert 0 in value
    assert "symbol" in value
    assert value["symbol"] in value
    with pytest.raises(IndexError):
        _ = value[2]
    with pytest.raises(KeyError):
        _ = value["missing"]


def test_dtype_from_fields_builds_exact_native_struct() -> None:
    fields = (
        Field("small", DataType("uint8"), nullable=False, metadata={"unit": "items"}),
        Field("wide", DataType.decimal(39, 4), nullable=True),
    )

    consumed: list[str] = []

    def one_shot() -> object:
        for field in fields:
            consumed.append(field.name)
            yield field

    value = DataType.from_fields(one_shot())

    assert value.kind == "nested"
    assert consumed == ["small", "wide"]
    assert tuple(value) == fields
    arrow = value.into_arrow()
    assert arrow.equals(pa.struct([field.into_arrow() for field in fields]))
    assert arrow.field(0).equals(
        pa.field(
            "small",
            pa.uint8(),
            nullable=False,
            metadata={b"unit": b"items"},
        ),
        check_metadata=True,
    )

    empty = DataType.from_fields(iter(()))
    assert empty.id == "struct"
    assert len(empty) == 0
    assert empty.into_arrow() == pa.struct([])

    with pytest.raises(TypeError, match="field at index 1"):
        DataType.from_fields([fields[0], object()])
    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.from_fields([fields[0], fields[0]])


def test_dtype_variant_assigns_dense_type_ids_in_member_order() -> None:
    members = (
        Field(
            "count",
            "int64",
            nullable=False,
            metadata={"source": "integer"},
        ),
        Field("label", "utf8", nullable=False),
        Field("missing", "null", nullable=True),
    )
    consumed: list[str] = []

    def one_shot() -> object:
        for member in members:
            consumed.append(member.name)
            yield member

    variant = DataType.variant(one_shot())
    arrow = variant.into_arrow()

    assert str(inspect.signature(DataType.variant)) == "(fields=None)"
    assert consumed == ["count", "label", "missing"]
    assert variant.id == "union"
    assert tuple(variant) == members
    assert arrow.mode == "dense"
    assert tuple(arrow.type_codes) == (0, 1, 2)
    assert arrow.field(0).metadata == {b"source": b"integer"}
    assert variant == DataType._union(enumerate(members), "dense")

    with pytest.raises(TypeError, match="field at index 1"):
        DataType.variant([members[0], object()])
    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.variant([members[0], members[0]])
    with pytest.raises(ValueError):
        DataType.variant(
            Field(f"member_{index}", "null") for index in range(129)
        )


def test_bare_variant_is_the_self_describing_datatype_not_the_union_sugar() -> None:
    variant = DataType.variant()

    assert variant.id == "variant"
    assert variant.kind == "nested"
    assert str(variant) == "variant"
    assert variant == DataType("variant")
    assert DataType(str(variant)) == variant
    # The parenthesis disambiguates: members keep building the dense union.
    assert DataType.variant([Field("only", "int64")]).id == "union"
    assert DataType("variant(only:int64)").id == "union"


def test_geometry_and_geography_fill_and_display_their_defaults() -> None:
    geometry = DataType.geometry()

    assert geometry.id == "geometry"
    assert geometry.kind == "geospatial"
    assert str(geometry) == "geometry"
    assert geometry == DataType.geometry("OGC:CRS84")
    assert DataType("geometry") == geometry

    projected = DataType.geometry("EPSG:3857")
    assert str(projected) == 'geometry("EPSG:3857")'
    assert DataType(str(projected)) == projected

    geography = DataType.geography()
    assert geography.id == "geography"
    assert geography.kind == "geospatial"
    assert str(geography) == "geography"
    assert geography == DataType.geography("OGC:CRS84", "spherical")

    vincenty = DataType.geography("OGC:CRS84", "vincenty")
    assert str(vincenty) == 'geography("OGC:CRS84","vincenty")'
    assert DataType("geography('OGC:CRS84', 'vincenty')") == vincenty
    assert DataType(str(vincenty)) == vincenty

    with pytest.raises(ValueError, match="expected a coordinate reference system"):
        DataType.geometry("")
    with pytest.raises(ValueError, match="expected one of spherical"):
        DataType.geography("OGC:CRS84", "euclidean")


def test_dtype_arrow_roundtrip_preserves_nested_map_and_dictionary_flags() -> None:
    sorted_map = pa.map_(pa.string(), pa.int64(), keys_sorted=True)
    arrow_type = pa.struct(
        [
            pa.field(
                "items",
                pa.list_(
                    pa.field(
                        "entry",
                        pa.struct([pa.field("lookup", sorted_map, False)]),
                        False,
                    )
                ),
                False,
            ),
            pa.field(
                "codes",
                pa.dictionary(pa.int16(), pa.string(), ordered=True),
                False,
            ),
        ]
    )

    projected = DataType.from_arrow(arrow_type).into_arrow()
    nested_map = projected.field("items").type.value_type.field("lookup").type
    assert nested_map.keys_sorted is True
    assert projected.field("codes").type.ordered is True


def test_the_uuid_is_sixteen_bytes_spelled_as_one_identifier() -> None:
    uuid_type = DataType("uuid")

    assert DataType("uuid") == uuid_type
    assert uuid_type.id == "uuid"
    assert uuid_type.kind == "uuid"
    assert str(uuid_type) == "uuid"
    assert uuid_type.fixed_byte_width == 16
    assert uuid_type.string_parameters is None
    assert uuid_type.bytes_parameters is None

    # The identity is the sixteen bytes; the spelling is a rendering of them.
    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    field = Field("id", uuid_type, nullable=False)
    assert field.arrow_scalar(text) == pa.scalar(
        packed.to_bytes(16, "big"), pa.binary(16)
    )
    assert field.arrow_scalar(text.upper()) == field.arrow_scalar(text)
    assert field.arrow_scalar(text.replace("-", "")) == field.arrow_scalar(text)
    assert field.arrow_scalar(packed.to_bytes(16, "big")) == field.arrow_scalar(text)
    assert field.default_scalar().as_py() == "00000000-0000-0000-0000-000000000000"

    # Storage is the canonical `arrow.uuid` extension over sixteen bytes, so
    # PyArrow rebuilds its own registered extension type from the projection.
    arrow = field.into_arrow()
    assert arrow.type == pa.uuid()
    assert arrow.type.storage_type == pa.binary(16)
    assert Field.from_arrow(arrow) == field

    # A cast into the type validates; the stored column read as text spells it.
    stored = Serie.from_arrow_array(pa.array([text, text.upper()]), field).into_arrow_array()
    # PyArrow reads its own registered extension back as `uuid.UUID`.
    assert stored.to_pylist() == [uuid.UUID(text)] * 2
    assert stored.storage.to_pylist() == [packed.to_bytes(16, "big")] * 2
    # A recognized identifier column renders as its spelling, exactly as a
    # recognized ASCII column renders as its trimmed text.
    batch = pa.record_batch([stored], schema=pa.schema([arrow]))
    spelled = DataType.from_fields([Field("id", "utf8")])
    assert Serie.from_arrow_batch(
        batch, Field("row", spelled, nullable=False)
    ).into_arrow_batch().column(0).to_pylist() == [text, text]

    with pytest.raises(ValueError, match="36-character"):
        Serie.from_arrow_array(pa.array(["not-a-uuid"]), field).into_arrow_array()


def test_version_is_numeric_with_an_arrow_string_projection() -> None:
    dtype = DataType("version")
    field = Field("version", dtype, nullable=False)

    assert dtype.id == "version"
    assert dtype.kind == "text"
    assert str(dtype) == "version"
    assert dtype.fixed_byte_width is None
    assert dtype.string_parameters is None
    assert field.default_scalar().as_py() == Version(0)
    assert field.arrow_scalar("5.0.01") == pa.scalar("5.0.1")
    assert Serie.from_arrow_array(
        pa.array(["5.0.01", "5.0.10"]), field
    ).into_arrow_array().to_pylist() == [
        "5.0.1",
        "5.0.10",
    ]

    arrow = field.into_arrow()
    assert arrow.type == pa.string()
    assert Field.from_arrow(arrow) == field
    # A tail states no number and folds into the patch rather than failing, so
    # the projection refuses on the major it cannot read, not on the tail.
    assert Serie.from_arrow_array(
        pa.array(["5.0+"]), field
    ).into_arrow_array().to_pylist() == [str(Version.from_str("5.0+"))]
    with pytest.raises(ValueError, match="version"):
        Serie.from_arrow_array(pa.array(["FIX.5.0"]), field).into_arrow_array()


def test_url_is_a_validated_canonical_location_over_utf8_text() -> None:
    dtype = DataType("url")
    field = Field("url", dtype, nullable=False)

    assert dtype.id == "url"
    assert dtype.kind == "text"
    assert str(dtype) == "url"
    assert dtype.fixed_byte_width is None
    assert dtype.string_parameters is None
    assert DataType("url") == dtype
    assert eval(repr(dtype), {"DataType": DataType}) == dtype

    # The value is a location, not the text that spelled it: the scheme folds
    # to lower case, a percent escape takes its canonical upper-case digits,
    # and a bare path is the `file:` URL that names it.
    assert Serie.from_arrow_array(
        pa.array(["HTTPS://example.com/a%2fb", "/lake/part.txt"]), field
    ).into_arrow_array().to_pylist() == [
        "https://example.com/a%2Fb",
        "file:///lake/part.txt",
    ]
    assert field.arrow_scalar("HTTPS://example.com/a%2fb") == pa.scalar(
        "https://example.com/a%2Fb"
    )
    assert field.arrow_scalar("/lake/part.txt") == pa.scalar("file:///lake/part.txt")

    # Nothing relative is a location, so none of them read as one.
    for relative in ("./rel", "example.com/x"):
        with pytest.raises(ValueError, match="does not read as url"):
            Serie.from_arrow_array(pa.array([relative]), field).into_arrow_array()
        with pytest.raises(ValueError, match="expected url"):
            field.arrow_scalar(relative)

    # The empty text names nothing at all, so it is not a spelling to refuse
    # but an absence: null before the reader runs, which this required column
    # repairs with its default, or refuses by path when strict.
    assert Serie.from_arrow_array(
        pa.array([""]), field
    ).into_arrow_array().to_pylist() == ["file:///"]
    with pytest.raises(
        ValueError, match=r"required Arrow field \$\.url holds 1 null values"
    ):
        Serie.from_arrow_array(pa.array([""]), field, nullability="strict").into_arrow_array()
    with pytest.raises(ValueError, match="null"):
        field.arrow_scalar("")
    assert dtype.scalar("").is_null()

    # Storage is Utf8 under the `yggdryl.url` extension name, so a projection
    # round-trips through Arrow without losing which datatype it is.
    arrow = field.into_arrow()
    assert arrow.type == pa.string()
    assert arrow.metadata == {
        b"ARROW:extension:name": b"yggdryl.url",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow) == field

    # A column of locations is nullable by default, because a handle that is
    # nowhere has no location to state.
    located = yggdryl.url("location", metadata={"role": "source"})
    assert type(located) is Field
    assert located.dtype == dtype
    assert located.nullable is True
    assert located.metadata["role"] == "source"
    assert yggdryl.url("location", nullable=False).nullable is False
    assert Serie.from_arrow_array(
        pa.array(["HTTPS://example.com/a%2fb", None]), located
    ).into_arrow_array().to_pylist() == ["https://example.com/a%2Fb", None]


def test_urn_is_a_validated_canonical_name_over_utf8_text() -> None:
    dtype = DataType("urn")
    field = Field("urn", dtype, nullable=False)

    assert dtype.id == "urn"
    assert dtype.kind == "text"
    assert str(dtype) == "urn"
    assert dtype.fixed_byte_width is None
    assert dtype.string_parameters is None
    assert DataType("urn") == dtype
    assert eval(repr(dtype), {"DataType": DataType}) == dtype

    # A name, not a location: the scheme and the namespace fold to lower case,
    # and what a `url` column holds is exactly what a `urn` column refuses.
    assert Serie.from_arrow_array(
        pa.array(["URN:ISBN:0451450523", "urn:example:a%20b"]), field
    ).into_arrow_array().to_pylist() == [
        "urn:isbn:0451450523",
        "urn:example:a%20b",
    ]
    assert field.arrow_scalar("URN:ISBN:0451450523") == pa.scalar("urn:isbn:0451450523")
    for location in ("https://example.com/a", "/lake/part.txt"):
        with pytest.raises(ValueError, match="does not read as urn"):
            Serie.from_arrow_array(pa.array([location]), field).into_arrow_array()
        with pytest.raises(ValueError, match="expected urn"):
            field.arrow_scalar(location)
    with pytest.raises(ValueError, match="does not read as url"):
        Serie.from_arrow_array(
            pa.array(["urn:isbn:0451450523"]), Field("url", DataType("url"), nullable=False)
        ).into_arrow_array()

    # The empty text is an absence, as it is for every non-text column, and
    # a name has no zero: the default is the nil name.
    assert Serie.from_arrow_array(
        pa.array([""]), field
    ).into_arrow_array().to_pylist() == ["urn:nil:nil"]
    assert dtype.scalar("").is_null()
    assert field.default_scalar().as_py() == "urn:nil:nil"

    # Storage is Utf8 under the `yggdryl.urn` extension name, so a projection
    # comes back a urn column rather than text or a url column.
    assert field.into_arrow().type == pa.utf8()
    assert field.into_arrow().metadata[b"ARROW:extension:name"] == b"yggdryl.urn"
    assert Field.from_arrow(field.into_arrow()) == field
    assert Field.from_arrow(field.into_arrow()).dtype == dtype


def test_every_string_is_one_datatype_and_one_of_eighteen_leaves() -> None:
    ascii_text = DataType("ascii")
    fixed = DataType.fixed_ascii(3)

    # `ascii` is the plain shape in the US-ASCII charset: any length, so no
    # width. The leaf is its own datatype id, and `string(us-ascii)` is the
    # charset-free spelling restated in that charset's family.
    assert ascii_text == DataType.ascii()
    assert ascii_text == DataType.string(charset="us-ascii")
    assert ascii_text.id == "ascii"
    assert ascii_text.kind == "text"
    assert str(ascii_text) == "ascii"
    assert ascii_text.charset == "us-ascii"
    assert ascii_text.fixed_byte_width is None
    assert ascii_text.string_parameters == StringParameters("ascii")
    assert ascii_text.string_parameters == StringParameters("string", "us-ascii")
    assert ascii_text.string_parameters.bound is None
    assert eval(repr(ascii_text), {"DataType": DataType}) == ascii_text

    # A fixed width is the fixed leaf, so two widths are two datatypes and
    # neither is the variable form.
    assert fixed == DataType.string("fixed_string", "us-ascii", 3)
    assert fixed == DataType.string("fixed_ascii", bound=3)
    # Only a charset-free spelling takes a charset: a charset-named one
    # already says it, so a second charset beside it is refused.
    with pytest.raises(ValueError, match="expected no charset on fixed_ascii"):
        DataType.string("fixed_ascii", "us-ascii", 3)
    with pytest.raises(ValueError, match="expected no charset on utf8"):
        StringParameters("utf8", "windows-1252")
    assert fixed.id == "fixed_ascii"
    assert fixed.kind == "text"
    assert str(fixed) == "fixed_ascii(3)"
    assert fixed.fixed_byte_width == 3
    assert fixed.string_parameters.fixed == 3
    assert fixed.string_parameters.max is None
    assert fixed.string_parameters.layout == "fixed_ascii"
    assert DataType("fixed_ascii(3)") == fixed
    assert DataType("fixed_string(us-ascii,3)") == fixed
    assert eval(repr(fixed), {"DataType": DataType}) == fixed
    assert DataType.fixed_ascii(4) != fixed
    assert fixed != ascii_text
    # Any width of at least one byte is storage; only the packed integer
    # stops at sixteen bytes.
    assert DataType.fixed_ascii(64).fixed_byte_width == 64
    assert DataType("utf8").fixed_byte_width is None

    # A maximum is the sized leaf: `ascii(4)` is `sized_ascii(4)` written
    # short, because plain storage is exactly what a bounded column fills,
    # and `fixed_ascii(4)` is the width.
    bounded = DataType("ascii(4)")
    assert bounded == DataType.string(charset="us-ascii", bound=4)
    assert bounded == DataType.string("sized_ascii", bound=4)
    assert bounded.id == "sized_ascii"
    assert bounded.string_parameters.max == 4
    assert bounded.string_parameters.fixed is None
    assert bounded.string_parameters.layout == "sized_ascii"
    assert bounded.fixed_byte_width is None
    assert str(bounded) == "sized_ascii(4)"
    assert bounded != DataType.fixed_ascii(4)

    # The six UTF-8 leaves, each under its canonical name; the charset-free
    # spellings name the UTF-8 leaf of their shape.
    assert DataType.utf8() == DataType("utf8") == DataType("string")
    assert DataType.utf8().id == "utf8"
    assert DataType.utf8().string_parameters == StringParameters()
    assert DataType.large_utf8() == DataType("large_string")
    assert DataType.large_utf8().id == "large_utf8"
    assert DataType.utf8_view() == DataType("utf8_view") == DataType("string_view")
    assert DataType.utf8_view().id == "utf8_view"
    assert DataType.string("large_string_view").id == "large_utf8_view"
    assert str(DataType.string("large_string_view")) == "large_utf8_view"
    assert DataType.fixed_utf8(8) == DataType("char(8)")
    assert DataType.fixed_utf8(8).id == "fixed_utf8"
    assert str(DataType.fixed_utf8(8)) == "fixed_utf8(8)"
    assert DataType.fixed_utf8(8).charset == "utf-8"
    assert DataType("varchar(32)") == DataType.string(bound=32)
    assert DataType.string(bound=32).id == "sized_utf8"
    assert str(DataType.string(bound=32)) == "sized_utf8(32)"
    # A large leaf is just a large leaf: only the fixed and sized leaves take
    # a number, so a maximum beside any other is refused by the name of the
    # leaf that carries one.
    with pytest.raises(ValueError, match="sized_utf8"):
        DataType.string("large_utf8", bound=64)
    with pytest.raises(ValueError, match="sized_utf8"):
        DataType("large_utf8(64)")
    with pytest.raises(ValueError, match="sized_cp1252"):
        DataType.string("large_string", "windows-1252", 32)

    # The windows-1252 leaves are the third family; the charset-free
    # spelling with that charset lands on them and renders the leaf.
    latin = DataType.string(charset="windows-1252", bound=32)
    assert latin.id == "sized_cp1252"
    assert str(latin) == "sized_cp1252(32)"
    assert DataType("string(windows-1252,32)") == latin
    assert DataType("sized_cp1252(32)") == latin
    assert latin.charset == "windows-1252"
    assert latin.string_parameters == StringParameters("string", "windows-1252", 32)
    assert latin.string_parameters.layout == "sized_cp1252"
    assert DataType.string(charset="cp1252") == DataType("cp1252")
    assert DataType("cp1252").id == "cp1252"
    assert DataType("large_cp1252_view").id == "large_cp1252_view"
    assert latin.is_string
    assert not latin.is_code

    # The parameters are a frozen value: equal, hashable, ordered, picklable.
    parameters = latin.string_parameters
    assert hash(parameters) == hash(StringParameters("sized_cp1252", bound=32))
    assert parameters.stable_hash() == latin.stable_hash()
    assert pickle.loads(pickle.dumps(parameters)) == parameters
    assert copy.deepcopy(parameters) == parameters
    assert str(parameters) == "sized_cp1252(32)"
    assert repr(parameters) == 'StringParameters("sized_cp1252", bound=32)'
    assert repr(StringParameters("string", "us-ascii")) == 'StringParameters("ascii")'
    assert eval(repr(parameters), {"StringParameters": StringParameters}) == parameters
    assert StringParameters() < parameters
    assert {parameters: 1}[StringParameters("string", "windows-1252", 32)] == 1
    with pytest.raises(AttributeError):
        parameters.bound = 3  # type: ignore[misc]

    # A name is one more spelling of a datatype, and it folds case, `_`, `-`,
    # and spaces the way the grammar folds them.
    names = DataType.logical_names()
    assert names["price"] == DataType("float64")
    assert DataType("Price") == names["price"]
    assert DataType.from_logical_name("UTC_Timestamp") == DataType('datetime64(ns,"UTC")')
    # The base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert DataType("int") == DataType("int32")
    assert DataType("float") == DataType("float32")

    assert DataType.from_logical_name("sedol") == DataType("sedol")
    with pytest.raises(ValueError, match="at least one byte, got 0"):
        DataType.fixed_ascii(0)
    # A leaf that is its number, stated with none, is refused by name.
    with pytest.raises(ValueError, match="fixed_utf8"):
        DataType.string("fixed_string")
    with pytest.raises(ValueError, match="fixed_utf8"):
        StringParameters("fixed_utf8")
    with pytest.raises(ValueError, match="sized_ascii"):
        DataType("sized_ascii")
    with pytest.raises(ValueError):
        DataType.string("utf9")
    # Only the three charsets with a string datatype are accepted.
    with pytest.raises(ValueError, match="utf-8, us-ascii or windows-1252"):
        DataType.string(charset="utf-16le")
    with pytest.raises(ValueError, match="charset"):
        DataType.string(charset="utf-16")
    with pytest.raises(ValueError, match="utf-8, us-ascii or windows-1252"):
        DataType("string(iso-8859-1)")


def test_every_byte_column_is_one_datatype_with_a_layout_and_a_bound() -> None:
    plain = DataType("binary")
    assert plain == DataType.binary() == DataType.bytes()
    assert plain.id == "binary"
    assert plain.kind == "bytes"
    assert plain.is_binary
    assert plain.bytes_parameters == BytesParameters()
    assert plain.bytes_parameters.layout == "binary"
    assert plain.bytes_parameters.bound is None
    assert plain.string_parameters is None
    assert plain.charset is None
    assert plain.fixed_byte_width is None

    # A maximum is `sized_binary`, and the exact width is `fixed_binary`.
    # `binary(16)` is the sized leaf written short, because plain binary is
    # exactly the storage a bounded column fills.
    bounded = DataType("binary(16)")
    assert bounded == DataType.bytes(bound=16) == DataType("varbinary(16)")
    assert bounded.bytes_parameters.max == 16
    assert bounded.bytes_parameters.fixed is None
    assert str(bounded) == "sized_binary(16)"
    assert bounded.id == "sized_binary"
    fixed = DataType.fixed_size_binary(16)
    assert fixed == DataType("fixed_size_binary(16)") == DataType.bytes("fixed_binary", 16)
    assert fixed.id == "fixed_binary"
    assert fixed.fixed_byte_width == 16
    assert fixed.bytes_parameters == BytesParameters("fixed_binary", 16)
    assert fixed.bytes_parameters.fixed == 16
    assert fixed.bytes_parameters.max is None
    assert fixed != bounded
    assert DataType.large_binary().id == "large_binary"
    assert DataType.binary_view().id == "binary_view"
    assert DataType("large_binary_view").id == "large_binary_view"
    # A large binary is just a large binary: only the two leaves that *are*
    # a number take one, so a maximum beside any other leaf is refused by
    # name rather than silently becoming something narrower.
    with pytest.raises(ValueError, match="sized_binary"):
        DataType.bytes("large_binary", 8)
    with pytest.raises(ValueError, match="sized_binary"):
        DataType("large_binary(8)")

    parameters = fixed.bytes_parameters
    assert repr(parameters) == 'BytesParameters("fixed_binary", 16)'
    assert repr(BytesParameters()) == 'BytesParameters("binary")'
    assert str(parameters) == "fixed_binary(16)"
    assert pickle.loads(pickle.dumps(parameters)) == parameters
    assert hash(parameters) == hash(BytesParameters("fixed_binary", 16))
    assert parameters.stable_hash() == fixed.stable_hash()
    assert BytesParameters() < parameters

    with pytest.raises(ValueError, match="at least one byte, got 0"):
        DataType.fixed_size_binary(0)
    with pytest.raises(ValueError, match="got none"):
        DataType.bytes("fixed_size_binary")
    with pytest.raises(ValueError):
        BytesParameters("blob_view")


def test_a_registered_code_is_its_own_datatype() -> None:
    # ISO 3166-1 is two letters, ISO 4217 three, ISO 10383 four, ISO 10962 six
    # and ISO 6166 twelve: each is a datatype of its own holding its values to
    # exactly that, not a name over a width. The five are the registrations
    # whose name answers a type of its own.
    ccy = DataType.from_logical_name("Ccy")
    assert ccy == DataType("ccy")
    assert DataType.logical_names()["ccy"] == ccy
    assert DataType.from_logical_name("Exchange") == DataType("mic")

    assert ccy.id == "ccy"
    assert ccy.kind == "code"
    assert str(ccy) == "ccy"
    # The width bounds a value; a code stores as the text it is, so it names
    # no fixed layout.
    assert ccy.code_width == 3
    assert ccy.fixed_byte_width is None
    assert ccy.string_parameters is None
    assert ccy.charset is None
    assert not ccy.is_string
    assert ccy != DataType.fixed_ascii(3)
    assert DataType(" CCY ") == ccy
    assert eval(repr(ccy), {"DataType": DataType}) == ccy
    with pytest.raises(ValueError):
        DataType("currency")
    with pytest.raises(ValueError):
        DataType.from_logical_name("Currency")

    for name, width in [
        ("country", 2),
        ("ccy", 3),
        ("mic", 4),
        ("cfi", 6),
        ("isin", 12),
        ("cusip", 9),
        ("sedol", 7),
    ]:
        dtype = DataType(name)
        assert (dtype.id, dtype.code_width, dtype.kind) == (name, width, "code")
        assert dtype.fixed_byte_width is None

    # The packed integer is the value's bytes padded to the code's own width,
    # exactly as for a fixed US-ASCII string of it. The padding is the
    # packing's; the column stores the text alone.
    assert ccy.ascii_packed("USD") == DataType.fixed_ascii(3).ascii_packed("USD")
    assert ccy.ascii_value(0x555344) == "USD"
    with pytest.raises(ValueError, match="at most 2 bytes"):
        DataType("country").ascii_packed("USD")
    figi = DataType("figi")
    assert (figi.id, figi.code_width, figi.kind) == ("figi", 12, "code")
    assert figi.scalar("bbg000blnq16").as_py() == "BBG000BLNQ16"
    with pytest.raises(ValueError, match="check digit"):
        figi.scalar("BBG000BLNQ17")

    # An ISIN is closed by its own check digit: a spelling one digit off is a
    # typo and is refused rather than stored as a security, and lower case
    # folds to the upper case it spells because the check digit cannot tell
    # the two apart.
    isin = DataType("isin")
    apple = isin.scalar("us0378331005")
    assert apple.as_py() == "US0378331005"
    assert apple.kind == "isin"
    assert pickle.loads(pickle.dumps(apple)) == apple
    assert isin.ascii_packed("US0378331005") == DataType.fixed_ascii(12).ascii_packed("US0378331005")
    with pytest.raises(ValueError, match="check digit does not close"):
        isin.scalar("US0378331006")
    with pytest.raises(ValueError, match="expected twelve characters"):
        isin.scalar("US037833100")

    # A CUSIP and a SEDOL are closed by their own check digits the same way:
    # nine and seven characters, a typo refused, lower case folded.
    cusip = DataType("cusip")
    apple_cusip = cusip.scalar("037833100")
    assert apple_cusip.as_py() == "037833100"
    assert apple_cusip.kind == "cusip"
    assert cusip.scalar("38259p508").as_py() == "38259P508"
    assert pickle.loads(pickle.dumps(apple_cusip)) == apple_cusip
    assert cusip.ascii_packed("037833100") == DataType.fixed_ascii(9).ascii_packed("037833100")
    with pytest.raises(ValueError, match="check digit does not close"):
        cusip.scalar("037833101")
    with pytest.raises(ValueError, match="expected nine characters"):
        cusip.scalar("03783310")
    sedol = DataType("sedol")
    shell = sedol.scalar("b0ybkj7")
    assert shell.as_py() == "B0YBKJ7"
    assert shell.kind == "sedol"
    assert pickle.loads(pickle.dumps(shell)) == shell
    assert sedol.ascii_packed("B0YBKJ7") == DataType.fixed_ascii(7).ascii_packed("B0YBKJ7")
    with pytest.raises(ValueError, match="check digit does not close"):
        sedol.scalar("B0YBKJ8")
    with pytest.raises(ValueError, match="expected seven characters"):
        sedol.scalar("B0YBKJ")
    # Two identifiers are two values, and neither is the string it spells.
    assert cusip.scalar("037833100") != DataType("utf8").scalar("037833100")
    assert cusip.scalar("037833100") != sedol.scalar("B0YBKJ7")


def test_a_code_and_a_uuid_read_into_every_string_and_byte_datatype() -> None:
    # Both families are one cast away in each direction, and the refusal on
    # the way names the row rather than leaving pyarrow to complain about a
    # slice length. A column's extension name is what a reading reads it
    # under, so each source travels as the batch that carries it.
    def row(field: Field) -> Field:
        return Field("row", DataType.from_fields([field]), nullable=False)

    def through(source: Field, stored: pa.Array, target: Field) -> pa.Array:
        batch = pa.RecordBatch.from_arrays(
            [stored], schema=pa.schema([source.into_arrow()])
        )
        return Serie.from_arrow_batch(batch, row(target), safe=False).into_arrow_batch().column(0)

    ccy = Field("ccy", "ccy")
    stored = Serie.from_arrow_array(pa.array(["USD"]), ccy, safe=False).into_arrow_array()
    for spelling in ["utf8", "ascii", "utf8(3)", "fixed_ascii(3)", "binary", "fixed_size_binary(3)"]:
        read = through(ccy, stored, Field("ccy", DataType(spelling)))
        assert Serie.from_arrow_array(
            read, ccy, safe=False
        ).into_arrow_array().to_pylist() == ["USD"]
    with pytest.raises(ValueError, match="exactly 8 bytes"):
        through(ccy, stored, Field("ccy", "fixed_size_binary(8)"))

    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    uid = Field("id", "uuid")
    identifiers = Serie.from_arrow_array(pa.array([text]), uid, safe=False).into_arrow_array()
    for spelling in ["utf8", "ascii", "utf8(36)", "fixed_ascii(36)", "binary", "fixed_size_binary(16)"]:
        read = through(uid, identifiers, Field("id", DataType(spelling)))
        back = Serie.from_arrow_array(read, uid, safe=False).into_arrow_array()
        assert through(uid, back, Field("id", "utf8")).to_pylist() == [text]
    with pytest.raises(ValueError, match="a fixed binary of 16 bytes"):
        through(uid, identifiers, Field("id", "fixed_size_binary(8)"))


def test_a_registered_code_carries_its_identity_across_arrow() -> None:
    ccy = Field("ccy", "ccy")
    arrow_field = ccy.into_arrow()

    # A code stores as the text it is; the extension name is what carries the
    # identity, so pyarrow sees a string column a reader can already use.
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.ccy",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow_field) == ccy
    retired = pa.field(
        "ccy",
        pa.string(),
        metadata={b"ARROW:extension:name": b"yggdryl.currency"},
    )
    with pytest.raises(ValueError) as error:
        Field.from_arrow(retired)
    message = str(error.value)
    assert "ccy" in message and "yggdryl.currency" in message and "yggdryl.ccy" in message

    # The same text under the string family's name is a bounded string, and
    # under no name at all is plain text.
    bounded = Field("ccy", DataType("ascii(3)"))
    assert Field.from_arrow(bounded.into_arrow()) == bounded
    assert bounded.into_arrow().type == pa.string()
    assert Field.from_arrow(pa.field("ccy", pa.string())) == Field("ccy", "utf8")

    assert ccy.arrow_scalar("USD") == pa.scalar("USD", pa.string())
    assert Serie.from_arrow_array(
        pa.array(["USD", "EU"]), ccy
    ).into_arrow_array().to_pylist() == ["USD", "EU"]
    # A cell the code refuses is null under the default safe cast and an
    # error naming the row when strict, exactly as a string cell is.
    assert Serie.from_arrow_array(pa.array(["EURO"]), ccy).into_arrow_array().to_pylist() == [None]
    with pytest.raises(ValueError, match="at most 3 bytes"):
        Serie.from_arrow_array(pa.array(["EURO"]), ccy, safe=False).into_arrow_array()


def test_a_fixed_ascii_width_pads_into_arrow_storage_and_trims_out_of_it() -> None:
    ascii32 = DataType.fixed_ascii(4)
    ccy = Field("ccy", ascii32)

    # Arrow has no fixed-width string, so the fixed layout rides
    # `FixedSizeBinary` and the declaration rides the `yggdryl.string`
    # document beside it.
    assert ascii32.into_arrow() == pa.binary(4)
    arrow_field = ccy.into_arrow()
    assert arrow_field.type == pa.binary(4)
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": (
            b'{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}'
        ),
    }
    assert Field.from_arrow(arrow_field) == ccy
    assert Field.from_arrow(pa.field("ccy", pa.binary(4))) == Field(
        "ccy", "fixed_size_binary(4)"
    )

    assert ascii32.arrow_scalar("USD") == pa.scalar(b"USD\x00", pa.binary(4))
    assert ascii32.arrow_scalar(b"USD\x00") == pa.scalar(b"USD\x00", pa.binary(4))
    assert ascii32.arrow_scalar(None) == pa.scalar(None, pa.binary(4))
    assert ccy.arrow_scalar("EUR") == pa.scalar(b"EUR\x00", pa.binary(4))
    assert ascii32.default_scalar().as_py() == ""
    assert ascii32.default_pyhint() is str
    assert Serie.from_default(
        Field("value", ascii32, nullable=False)
    ).into_arrow_scalar() == pa.scalar(b"\x00" * 4, pa.binary(4))

    padded = Serie.from_arrow_array(pa.array(["USD", None]), ccy).into_arrow_array()
    assert padded.type == pa.binary(4)
    assert padded.to_pylist() == [b"USD\x00", None]
    # A datatype casts as a required column: nulls fill with the default.
    filled = Serie.from_arrow_array(
        pa.array(["USD", None]), Field("value", ascii32, nullable=False)
    ).into_arrow_array()
    assert filled.to_pylist() == [b"USD\x00", b"\x00" * 4]

    row = DataType.from_fields([Field("ccy", "utf8")])
    stored = pa.record_batch([padded], schema=pa.schema([arrow_field]))
    assert Serie.from_arrow_batch(
        stored, Field("row", row, nullable=False)
    ).into_arrow_batch().column(0).to_pylist() == ["USD", None]

    # A safe cast nulls the cell it cannot write - the required column then
    # fills it with the default - and a strict one names the row.
    assert Serie.from_arrow_array(
        pa.array(["EURO!"]), Field("value", ascii32, nullable=False)
    ).into_arrow_array().to_pylist() == [b"\x00" * 4]
    assert Serie.from_arrow_array(pa.array(["EURO!"]), ccy).into_arrow_array().to_pylist() == [None]
    with pytest.raises(ValueError, match="row 0: expected at most 4 bytes"):
        Serie.from_arrow_array(
            pa.array(["EURO!"]), Field("value", ascii32, nullable=False), safe=False
        ).into_arrow_array()
    with pytest.raises(ValueError, match="at most 4 bytes"):
        ascii32.arrow_scalar("EURO!")
    with pytest.raises(ValueError, match="non-ASCII"):
        ccy.arrow_scalar("\u20ac")
    # A string is a string: a number or a boolean spells itself, exactly as
    # it does into `utf8`, and the width then judges the spelling.
    assert ascii32.arrow_scalar(3) == pa.scalar(b"3\x00\x00\x00", pa.binary(4))
    assert ascii32.arrow_scalar(True) == pa.scalar(b"true", pa.binary(4))
    assert ccy.arrow_scalar(1.5) == pa.scalar(b"1.5\x00", pa.binary(4))
    with pytest.raises(ValueError, match="at most 4 bytes"):
        ccy.arrow_scalar(12345)


def test_variable_ascii_rides_arrow_text_storage_under_its_declaration() -> None:
    note = DataType("ascii")
    field = Field("note", note)

    # ASCII bytes are UTF-8, so US-ASCII rides Arrow's own string layout;
    # what Arrow cannot say - the charset - rides the `yggdryl.string`
    # document. Plain UTF-8 crosses bare.
    assert note.into_arrow() == pa.string()
    arrow_field = field.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": b'{"layout":"ascii","charset":"us-ascii"}',
    }
    assert Field.from_arrow(arrow_field) == field
    assert Field.from_arrow(pa.field("note", pa.string())) == Field("note", "utf8")
    assert Field("text", "utf8").into_arrow().metadata is None

    assert note.arrow_scalar("free text") == pa.scalar("free text", pa.string())
    assert note.default_scalar().as_py() == ""
    assert note.default_pyhint() is str
    assert Serie.from_default(
        Field("value", note, nullable=False)
    ).into_arrow_scalar() == pa.scalar("", pa.string())

    stored = Serie.from_arrow_array(
        pa.array(["a", "much longer note", None]), field
    ).into_arrow_array()
    assert stored.to_pylist() == ["a", "much longer note", None]
    row = DataType.from_fields([Field("note", "utf8")])
    batch = pa.record_batch([stored], schema=pa.schema([arrow_field]))
    assert Serie.from_arrow_batch(
        batch, Field("row", row, nullable=False)
    ).into_arrow_batch().column(0).to_pylist() == [
        "a",
        "much longer note",
        None,
    ]

    # The value contract is the repertoire: no NUL, nothing above 0x7F.
    with pytest.raises(ValueError, match="non-ASCII"):
        note.arrow_scalar("\u20ac")
    with pytest.raises(ValueError, match="NUL"):
        note.arrow_scalar("a\x00b")
    # A packed integer needs a fixed width, so the variable form has none.
    with pytest.raises(ValueError, match="at most 16 bytes"):
        note.ascii_packed("USD")

    # A maximum is checked where a value enters, and the value keeps it:
    # the cell is a `sized_ascii(4)` value.
    bounded = Field("code", DataType("ascii(4)"))
    assert bounded.into_arrow().metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": b'{"layout":"sized_ascii","charset":"us-ascii","max":4}',
    }
    assert Field.from_arrow(bounded.into_arrow()) == bounded
    assert bounded.scalar("USD").dtype == DataType("sized_ascii(4)")
    assert bounded.scalar("USD").kind == "sized_ascii"
    with pytest.raises(ValueError, match="at most 4 bytes"):
        bounded.scalar("EURO!")
    assert Serie.from_arrow_array(
        pa.array(["EURO!"]), bounded
    ).into_arrow_array().to_pylist() == [None]
    with pytest.raises(ValueError, match="at most 4 bytes"):
        Serie.from_arrow_array(pa.array(["EURO!"]), bounded, safe=False).into_arrow_array()

    # The windows-1252 leaves ride binary storage, because their bytes are
    # not UTF-8; the document says which leaf and which charset.
    latin = Field("name", DataType.string(charset="windows-1252"))
    assert latin.into_arrow().type == pa.binary()
    assert latin.into_arrow().metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": b'{"layout":"cp1252","charset":"windows-1252"}',
    }
    assert Field.from_arrow(latin.into_arrow()) == latin
    assert Serie.from_arrow_array(
        pa.array(["caf\u00e9"]), latin
    ).into_arrow_array().to_pylist() == [b"caf\xe9"]
    assert latin.scalar(b"caf\xe9").as_py() == "caf\u00e9"


def test_a_string_and_a_byte_field_cross_json_under_one_tag_each() -> None:
    string = Field("name", DataType.string("fixed_string", "windows-1252", 8))
    document = json.loads(string.into_json())
    # The leaf's name says the charset, so no `charset` key is written.
    assert document["dtype"] == {
        "type": "string",
        "layout": "fixed_cp1252",
        "fixed": 8,
    }
    assert Field.from_json(string.into_json()) == string
    # A document that spells a charset-free layout beside a `charset` key
    # names the same leaf.
    assert (
        Field.from_json(
            '{"name":"name","nullable":true,"dtype":{"type":"string",'
            '"layout":"fixed_string","charset":"windows-1252","fixed":8}}'
        )
        == string
    )
    # Plain UTF-8 states nothing beyond its tag; a maximum is the sized leaf
    # and says `max` beside it.
    assert json.loads(Field("t", "utf8").into_json())["dtype"] == {"type": "string"}
    assert json.loads(Field("t", "utf8(32)").into_json())["dtype"] == {
        "type": "string",
        "layout": "sized_utf8",
        "max": 32,
    }
    assert json.loads(Field("t", "ascii").into_json())["dtype"] == {
        "type": "string",
        "layout": "ascii",
    }

    bytes_field = Field("blob", DataType.bytes("sized_binary", 16))
    assert json.loads(bytes_field.into_json())["dtype"] == {
        "type": "binary",
        "layout": "sized_binary",
        "max": 16,
    }
    assert Field.from_json(bytes_field.into_json()) == bytes_field
    assert json.loads(Field("b", "binary").into_json())["dtype"] == {"type": "binary"}
    assert json.loads(Field("b", "fixed_size_binary(4)").into_json())["dtype"] == {
        "type": "binary",
        "layout": "fixed_binary",
        "fixed": 4,
    }


def test_a_prebuilt_vocabulary_names_the_iso_codes_a_column_carries() -> None:
    prebuilt = StringEnum.prebuilt()
    assert set(prebuilt) == {
        "ccy",
        "country",
        "mic",
        "exchange",
        "side",
        "state",
        "timeinforce",
    }
    # `exchange` is FIX's name for the ISO 10383 code, so it is one list.
    assert prebuilt["mic"] == prebuilt["exchange"]

    countries = StringEnum.from_logical_name("Country")
    assert countries.name == "country"
    assert len(countries) == len(prebuilt["country"])
    # An ISO code names itself, so the member and its value are one spelling.
    assert countries.get("FR") == "FR"
    assert countries.get_member("FR") == "FR"
    assert "FR" in countries
    # A prebuilt listing is a constant, so a second build is the same enum.
    assert StringEnum.from_logical_name("country") == countries
    # `ZZ` is ISO 3166's user-assigned range, so no member names it.
    assert countries.get("ZZ") is None

    # A member's code is the value's own bytes under the datatype the name
    # resolved to, so every reader of the schema answers the same integers.
    country = DataType("country")
    members = dict(countries.into_members(country))
    assert members["FR"] == country.ascii_packed("FR")
    codes = countries.into_intenum(country)
    assert issubclass(codes, enum.IntEnum)
    assert codes.__name__ == "country"
    assert codes["FR"] == country.ascii_packed("FR")

    # A registered name with no prebuilt listing answers an enum of no
    # members, and one that is no registration at all is refused.
    assert len(StringEnum.from_logical_name("tenor")) == 0
    # An open identifier space has no listing to prebuild either.
    assert len(StringEnum.from_logical_name("isin")) == 0
    assert len(StringEnum.from_logical_name("cusip")) == 0
    assert len(StringEnum.from_logical_name("sedol")) == 0
    assert len(StringEnum.from_logical_name("figi")) == 0


def test_an_enum_member_name_is_the_one_rule_both_runtimes_apply() -> None:
    assert StringEnum.member_name("n/a") == "N_A"
    assert StringEnum.member_name("3M") == "_3M"
    assert StringEnum.member_name("") == "_"
    # A name that opens and closes with `_` would be a reserved `_sunder_` or
    # `__dunder__`, so the trailing run goes and every member is a member.
    assert StringEnum.member_name("-a-") == "_A"
    assert StringEnum.member_name("--b--") == "__B"

    codes = StringEnum("Currency", {"USD": "USD", "N_A": "n/a"})
    width = DataType.fixed_ascii(4)
    assert codes.into_members(width) == [
        ("N_A", 0x6E2F6100),
        ("USD", 0x55534400),
    ]
    # A value the width could not store is refused by the width, never
    # silently truncated into a member.
    with pytest.raises(ValueError, match="at most 2 bytes"):
        codes.into_members(DataType.fixed_ascii(2))


def test_the_datatype_restates_a_value_as_the_representation_it_declares() -> None:
    # The one value contract: a value crosses through the datatype, never
    # through PyArrow, which knows none of these rules.
    price = DataType("decimal128(18,4)").scalar(decimal.Decimal("12.5"))
    assert price.kind == "d128"
    assert price.as_py() == decimal.Decimal("12.5000")

    narrowed = DataType("int8").scalar(5)
    assert narrowed.kind == "i8"
    assert narrowed.as_py() == 5

    # An ASCII value is trimmed of its padding, and a value already in the
    # declared representation crosses unchanged.
    assert DataType.fixed_ascii(4).scalar("AAPL").as_py() == "AAPL"
    assert DataType("utf8").scalar("AAPL").as_py() == "AAPL"

    with pytest.raises(ValueError):
        DataType("int8").scalar(9999)


def test_a_datatype_recognizes_and_validates_its_own_default() -> None:
    assert DataType("int64").is_default_value(0)
    assert not DataType("int64").is_default_value(1)
    assert DataType("utf8").is_default_value("")
    assert DataType("int64").validate() is None
    assert DataType("struct<id:int64>").validate() is None


def test_a_datatype_answers_the_family_and_the_layout_of_its_identity() -> None:
    integer = DataType("int32")
    assert integer.is_integer
    assert integer.is_signed_integer
    assert not integer.is_unsigned_integer
    assert integer.is_numeric
    assert integer.is_ordered
    assert integer.fixed_byte_width == 4
    assert not integer.is_parameterized

    assert DataType("uint64").is_unsigned_integer
    assert DataType("float32").is_floating
    assert DataType("decimal128(5,2)").is_decimal
    assert DataType("decimal128(5,2)").is_parameterized
    assert DataType("timestamp(us)").is_temporal
    assert DataType("binary").is_binary
    assert DataType("binary").is_bytes
    assert DataType("utf8").is_string
    assert DataType("utf8").is_bytes
    assert DataType("utf8").fixed_byte_width is None

    # A wrapper encodes another value type, so it is not itself nested.
    encoded = DataType("dictionary<int32,utf8>")
    assert encoded.is_wrapper
    assert not DataType("struct<id:int64>").is_wrapper
    assert DataType("struct<id:int64>").is_nested
    assert not DataType("struct<id:int64>").is_ordered

    assert DataType.PARSE_RECURSION_LIMIT > 0


def test_a_code_datatype_names_the_vocabulary_it_draws_from() -> None:
    country = DataType("country")
    assert country.is_code
    assert country.kind == "code"
    assert country.code_name == "country"
    assert country.is_bytes

    # A bare fixed US-ASCII string of the same width is not a code.
    plain = DataType.fixed_ascii(2)
    assert not plain.is_code
    assert plain.code_name is None
    assert plain.is_string

    assert not DataType("utf8").is_code
    assert DataType("utf8").code_name is None


def test_a_uuid_packs_into_the_integer_its_sixteen_bytes_are() -> None:
    dtype = DataType("uuid")
    value = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")

    packed = dtype.uuid_packed(str(value))
    assert packed == value.int
    assert dtype.uuid_value(packed) == str(value)

    # The hyphenated spelling, bare hex, and the raw bytes are one identifier.
    assert dtype.uuid_packed(value.hex) == packed
    assert dtype.uuid_packed(value.bytes) == packed

    with pytest.raises(ValueError):
        DataType("int64").uuid_packed(str(value))
    # A packed UUID is unsigned, so a negative integer is not one.
    with pytest.raises(OverflowError):
        dtype.uuid_value(-1)


def test_a_parameterized_datatype_answers_each_parameter_it_declares() -> None:
    encoded = DataType("dictionary<int32,utf8>")
    assert str(encoded.dictionary_key) == "int32"
    assert str(encoded.dictionary_value) == "utf8"
    assert DataType("int64").dictionary_key is None

    entries = DataType("map<utf8,int64>")
    assert entries.keys_sorted is False
    assert DataType("int64").keys_sorted is None

    union = DataType("dense_union<a:int32,b:utf8>")
    assert union.union_type_ids == (0, 1)
    assert union.union_mode == "dense"
    assert DataType("int64").union_type_ids is None
    assert DataType("int64").union_mode is None

    geography = DataType.geography()
    assert geography.crs == "OGC:CRS84"
    assert geography.has_default_crs
    assert geography.edge_algorithm == "spherical"

    geometry = DataType.geometry()
    assert geometry.edge_algorithm is None
    assert not DataType.geometry("EPSG:4326").has_default_crs
    assert DataType("int64").crs is None
    assert DataType("int64").has_default_crs is None


def test_a_struct_datatype_projects_the_arrow_schema_a_row_declares() -> None:
    dtype = DataType("struct<id:int64,symbol:utf8>")
    schema = dtype.into_arrow_schema()

    assert isinstance(schema, pa.Schema)
    assert schema.names == ["id", "symbol"]
    assert schema.field("id").type == pa.int64()
    assert DataType.from_arrow(pa.struct(list(schema))) == dtype

    # A schema needs a row, so a leaf datatype refuses.
    with pytest.raises(ValueError):
        DataType("int64").into_arrow_schema()

    default = Serie.from_default(
        Field("value", DataType("int64"), nullable=False)
    ).into_arrow_array()
    assert isinstance(default, pa.Array)
    assert default.to_pylist() == [0]
    assert Serie.from_default(
        Field("value", DataType("utf8"), nullable=False)
    ).into_arrow_array().to_pylist() == [""]


def test_a_nested_datatype_is_rebuilt_with_replacement_children() -> None:
    struct = DataType("struct<id:int64,symbol:utf8>")
    rebuilt = struct.with_fields([Field("id", "int32"), Field("symbol", "utf8")])

    assert str(rebuilt["id"].dtype) == "int32"
    assert [child.name for child in rebuilt] == ["id", "symbol"]

    # The layout is kept, so the child count is part of the contract.
    with pytest.raises(ValueError):
        struct.with_fields([Field("id", "int32")])

    # A serie still holds exactly one item field, and the rebuilt datatype
    # renders as the canonical lossless form the grammar round-trips.
    widened = DataType("serie<int32>").with_fields([Field("item", "int64")])
    assert DataType.from_str(str(widened)) == widened
    assert str(widened["item"].dtype) == "int64"


def test_every_native_datatype_variant_has_a_typed_field_factory() -> None:
    item = yggdryl.int8("item", nullable=False)
    entries = yggdryl.struct(
        "entries",
        [
            yggdryl.utf8("key", nullable=False),
            yggdryl.int64("value"),
        ],
        nullable=False,
    )
    run_ends = yggdryl.int16("run_ends", nullable=False)
    values = yggdryl.utf8("values")

    values_by_kind = {
        "null": yggdryl.null("value"),
        "boolean": yggdryl.boolean("value"),
        "int8": yggdryl.int8("value"),
        "int16": yggdryl.int16("value"),
        "int32": yggdryl.int32("value"),
        "int64": yggdryl.int64("value"),
        "uint8": yggdryl.uint8("value"),
        "uint16": yggdryl.uint16("value"),
        "uint32": yggdryl.uint32("value"),
        "uint64": yggdryl.uint64("value"),
        "float16": yggdryl.float16("value"),
        "float32": yggdryl.float32("value"),
        "float64": yggdryl.float64("value"),
        "datetime64": yggdryl.datetime64("value", "us", "Europe/Paris"),
        "date32": yggdryl.date32("value"),
        "date64": yggdryl.date64("value"),
        "time32": yggdryl.time32("value", "ms"),
        "time64": yggdryl.time64("value", "ns"),
        "duration32": yggdryl.duration32("value", "ms"),
        "duration64": yggdryl.duration64("value", "us"),
        "interval": yggdryl.interval("value", "month_day_nano"),
        "binary": yggdryl.binary("value"),
        "fixed_binary": yggdryl.fixed_size_binary("value", 16),
        "sized_binary": yggdryl.sized_binary("value", 16),
        "large_binary": yggdryl.large_binary("value"),
        "binary_view": yggdryl.binary_view("value"),
        "large_binary_view": yggdryl.large_binary_view("value"),
        "utf8": yggdryl.utf8("value"),
        "large_utf8": yggdryl.large_utf8("value"),
        "utf8_view": yggdryl.utf8_view("value"),
        "large_utf8_view": yggdryl.large_utf8_view("value"),
        "fixed_utf8": yggdryl.fixed_utf8("value", 8),
        "sized_utf8": yggdryl.sized_utf8("value", 32),
        "ascii": yggdryl.ascii("value"),
        "large_ascii": yggdryl.large_ascii("value"),
        "ascii_view": yggdryl.ascii_view("value"),
        "large_ascii_view": yggdryl.large_ascii_view("value"),
        "fixed_ascii": yggdryl.fixed_ascii("value", 4),
        "sized_ascii": yggdryl.sized_ascii("value", 4),
        "cp1252": yggdryl.cp1252("value"),
        "large_cp1252": yggdryl.large_cp1252("value"),
        "cp1252_view": yggdryl.cp1252_view("value"),
        "large_cp1252_view": yggdryl.large_cp1252_view("value"),
        "fixed_cp1252": yggdryl.fixed_cp1252("value", 8),
        "sized_cp1252": yggdryl.sized_cp1252("value", 32),
        "serie": yggdryl.serie("value", item),
        "serie_view": yggdryl.serie_view("value", item),
        "fixed_size_serie": yggdryl.fixed_size_serie("value", item, 3),
        "large_serie": yggdryl.large_serie("value", item),
        "large_serie_view": yggdryl.large_serie_view("value", item),
        "struct": yggdryl.struct("value", [item]),
        "union": yggdryl.union("value", [(3, item)], "dense"),
        "dictionary": yggdryl.dictionary("value", "int16", "utf8"),
        "decimal32": yggdryl.decimal32("value", 9, 2),
        "decimal64": yggdryl.decimal64("value", 18, 2),
        "decimal128": yggdryl.decimal128("value", 38, 2),
        "decimal256": yggdryl.decimal256("value", 76, 2),
        "map": yggdryl.map("value", entries),
        "sorted_map": yggdryl.map("value", entries, keys_sorted=True),
        "run_end_encoded": yggdryl.run_end_encoded(
            "value", run_ends, values
        ),
        "variant": yggdryl.variant("value"),
        "country": yggdryl.country("value"),
        "ccy": yggdryl.ccy("value"),
        "mic": yggdryl.mic("value"),
        "cfi": yggdryl.cfi("value"),
        "isin": yggdryl.isin("value"),
        "cusip": yggdryl.cusip("value"),
        "sedol": yggdryl.sedol("value"),
        "bloomberg": yggdryl.bloomberg("value"),
        "figi": yggdryl.figi("value"),
        "uuid": yggdryl.uuid("value"),
        "version": yggdryl.version("value"),
        "url": yggdryl.url("value"),
        "urn": yggdryl.urn("value"),
        "timezone": yggdryl.timezone("value"),
        "mimetype": yggdryl.mimetype("value"),
        "mediatype": yggdryl.mediatype("value"),
        "side": yggdryl.side("value"),
        "state": yggdryl.state("value"),
        "timeinforce": yggdryl.timeinforce("value"),
        "geometry": yggdryl.geometry("value"),
        "geography": yggdryl.geography("value", "OGC:CRS84", "vincenty"),
    }

    assert set(values_by_kind) == {
        value.dtype.id for value in values_by_kind.values()
    }
    # The factories cover every datatype Arrow has a layout for. `int128` and
    # `uint128` are the two identifiers `Scalar` stores and `DataType` cannot,
    # so no field builds them.
    unbuildable = {"int128", "uint128"}
    assert len(values_by_kind) == len(enums.DATA_TYPE_IDS) - len(unbuildable)
    assert set(values_by_kind) == set(enums.DATA_TYPE_IDS) - unbuildable
    assert all(type(value) is Field for value in values_by_kind.values())
    assert yggdryl.Int32Field is Field
    assert yggdryl.VersionField is Field
    assert yggdryl.UrlField is Field
    assert yggdryl.TimezoneField is Field
    assert yggdryl.MimeTypeField is Field
    assert yggdryl.MediaTypeField is Field
    assert yggdryl.TypedField is Field


def test_the_string_and_bytes_factories_take_the_whole_declaration() -> None:
    # One factory per family takes the layout, the charset, and the bound;
    # the leaf factories are that one with a leaf picked once. A maximum is
    # the sized leaf of the charset, so `max` beside a large layout is
    # refused by the name of the leaf that carries one.
    latin = yggdryl.string("name", layout="string", charset="cp1252", max=32)
    assert latin.dtype == DataType.string("sized_cp1252", bound=32)
    assert str(latin.dtype) == "sized_cp1252(32)"
    assert latin.dtype.string_parameters.max == 32
    assert latin.dtype == yggdryl.sized_cp1252("name", 32).dtype
    with pytest.raises(ValueError, match="sized_cp1252"):
        yggdryl.string("name", layout="large_string", charset="cp1252", max=32)
    fixed = yggdryl.string("code", layout="fixed_string", charset="us-ascii", fixed=4)
    assert fixed.dtype == DataType.fixed_ascii(4)
    assert yggdryl.string("code", layout="fixed_ascii", fixed=4).dtype == fixed.dtype
    assert yggdryl.string("text").dtype == DataType.utf8() == yggdryl.utf8("text").dtype
    assert yggdryl.sized_utf8("text", 32).dtype == DataType("utf8(32)")
    assert yggdryl.large_utf8_view("text").dtype == DataType("large_string_view")
    assert yggdryl.fixed_cp1252("text", 8).dtype == DataType("fixed_string(windows-1252,8)")
    assert yggdryl.string("text", nullable=False, metadata={"k": "v"}).metadata["k"] == "v"
    assert yggdryl.StringField is Field

    bounded = yggdryl.bytes("blob", max=16)
    assert bounded.dtype == DataType.bytes(bound=16) == DataType("binary(16)")
    assert bounded.dtype.bytes_parameters.max == 16
    digest = yggdryl.bytes("digest", layout="fixed_binary", fixed=16)
    assert digest.dtype == DataType.fixed_size_binary(16)
    assert digest.dtype == yggdryl.fixed_size_binary("digest", 16).dtype
    assert yggdryl.bytes("blob").dtype == DataType.binary() == yggdryl.binary("blob").dtype
    assert yggdryl.bytes("blob", layout="binary_view").dtype == DataType.binary_view()
    assert yggdryl.BytesField is Field


def test_nested_factories_preserve_exact_child_field_state() -> None:
    item = yggdryl.dictionary(
        "item",
        "int16",
        "utf8",
        nullable=False,
        metadata={"logical": "status"},
    )
    item.set_dictionary_options(42, True)
    projected_item = item.into_arrow()

    values = yggdryl.serie("values", item, metadata={"owner": "events"})
    child = values.dtype[0]

    assert child.equals(item)
    assert child.dictionary_id == 42
    assert child.dictionary_is_ordered is True
    assert child.metadata["logical"] == "status"
    assert child.into_arrow().equals(projected_item, check_metadata=True)
    assert values.into_arrow().metadata == {b"owner": b"events"}


class _LegacyPickle:
    """Pickles as exactly the call a release before the serie rename wrote."""

    def __init__(self, call: object, arguments: tuple[object, ...]) -> None:
        self.call = call
        self.arguments = arguments

    def __reduce__(self) -> tuple[object, tuple[object, ...]]:
        return self.call, self.arguments


@pytest.mark.parametrize(
    ("legacy", "canonical", "layout"),
    [
        ("list<int64>", "serie<int64>", "serie"),
        ("list_view<int64>", "serie_view<int64>", "serie_view"),
        ("fixed_size_list<int64, 3>", "fixed_size_serie<int64, 3>", "fixed_size_serie"),
        ("large_list<int64>", "large_serie<int64>", "large_serie"),
        ("large_list_view<int64>", "large_serie_view<int64>", "large_serie_view"),
    ],
)
def test_a_serie_layout_still_reads_the_list_spelling_it_had(
    legacy: str, canonical: str, layout: str
) -> None:
    # The five serie layouts were spelled as Arrow's lists before the rename.
    # Every door a datatype enters by still reads that spelling as the layout
    # it named, and every door out renders the serie name.
    dtype = DataType(legacy)
    assert dtype == DataType(canonical)
    assert dtype.id == layout
    assert str(dtype).startswith(f"{layout}(")
    assert DataType.from_str(legacy) == dtype
    assert Field("values", legacy) == Field("values", canonical)
    assert Field("values", legacy).dtype.id == layout

    # A pickle written before the rename carries the old rendering, and
    # loads as the same datatype and field.
    old_text = str(dtype).replace(layout, legacy.split("<")[0], 1)
    assert old_text != str(dtype)
    loaded = pickle.loads(pickle.dumps(_LegacyPickle(DataType.from_str, (old_text,))))
    assert loaded == dtype
    old_field = str(Field("values", dtype)).replace(layout, legacy.split("<")[0], 1)
    loaded_field = pickle.loads(
        pickle.dumps(_LegacyPickle(Field._from_pickle, (old_field, False)))
    )
    assert loaded_field == Field("values", dtype)

    # The internal factory door reads either spelling of the layout.
    item = Field("item", "int64")
    length = 3 if layout == "fixed_size_serie" else None
    assert DataType._serie(legacy.split("<")[0], item, length) == dtype
    assert DataType._serie(layout, item, length) == dtype


def test_the_serie_factories_are_the_package_names_for_the_five_layouts() -> None:
    # `yggdryl.serie` is the factory, as `yggdryl.string` is: the module of
    # the same name is reached by its import path.
    item = yggdryl.int64("item", nullable=False)
    assert callable(yggdryl.serie)
    assert yggdryl.serie("values", item).dtype == DataType("serie<item: int64 not null>")
    assert yggdryl.serie("values", item).dtype.id == "serie"
    assert yggdryl.fixed_size_serie("values", item, 2).dtype.id == "fixed_size_serie"
    for retired in ("list", "list_view", "fixed_size_list", "large_list", "large_list_view"):
        assert not hasattr(yggdryl, retired)
        assert retired not in yggdryl.__all__
    for retired in ("ListSerie", "ListField", "FixedSizeListSerie", "LargeListViewField"):
        assert not hasattr(yggdryl, retired)
    with pytest.raises(TypeError, match="requires a length"):
        DataType._serie("fixed_size_serie", item)
    with pytest.raises(ValueError, match="invalid serie kind"):
        DataType._serie("struct", item)


def test_dense_union_factory_is_a_typed_union_alias_with_native_ids() -> None:
    members = (
        yggdryl.int64("integer", nullable=False, metadata={"branch": "number"}),
        yggdryl.utf8("text", nullable=False),
    )

    value: yggdryl.DenseUnionField = yggdryl.dense_union(
        "payload",
        (member for member in members),
        nullable=False,
        metadata={"logical": "variant"},
    )
    arrow = value.into_arrow()

    assert type(value) is Field
    assert yggdryl.DenseUnionField is Field
    assert value.dtype.id == "union"
    assert tuple(value.dtype) == members
    assert arrow.type.mode == "dense"
    assert tuple(arrow.type.type_codes) == (0, 1)
    assert arrow.metadata == {b"logical": b"variant"}


def test_typed_factory_parameters_use_native_validation() -> None:
    assert yggdryl.decimal("small", 38).dtype.id == "decimal128"
    assert yggdryl.decimal("wide", 39).dtype.id == "decimal256"
    assert yggdryl.ascii("note").dtype == DataType("ascii")
    assert yggdryl.fixed_ascii("iso", 2).dtype == DataType.fixed_ascii(2)
    assert yggdryl.fixed_ascii("ccy", 3).dtype == DataType.fixed_ascii(3)
    # A fixed width past the packed integer is still storage, so it builds.
    assert yggdryl.fixed_ascii("isin", 64, nullable=False).dtype.fixed_byte_width == 64
    assert yggdryl.fixed_utf8("name", 8).dtype == DataType.fixed_utf8(8)
    assert yggdryl.fixed_utf8("name", 8).dtype.string_parameters.fixed == 8
    assert yggdryl.ccy("ccy", metadata={"code": "ISO 4217"}).metadata["code"] == (
        "ISO 4217"
    )
    assert not hasattr(yggdryl, "currency")
    assert not hasattr(yggdryl, "CurrencyField")
    assert yggdryl.time("coarse", "ms").dtype == DataType("time32(ms)")
    assert yggdryl.time("precise", "us").dtype == DataType("time64(us)")
    assert yggdryl.datetime64("event", "us", "Custom/Accepted").dtype.id == (
        "datetime64"
    )

    with pytest.raises(ValueError, match="temporal resolution"):
        yggdryl.time("clock", "day_time")
    with pytest.raises(ValueError, match="temporal resolution"):
        yggdryl.datetime64("event", "year_month")
    with pytest.raises(ValueError, match="interval layout"):
        yggdryl.interval("window", "us")
    with pytest.raises(ValueError, match="precision"):
        yggdryl.decimal32("amount", 10)
    with pytest.raises(ValueError, match="at least one byte"):
        yggdryl.fixed_ascii("narrow", 0)
    with pytest.raises(ValueError, match="at least one byte"):
        yggdryl.fixed_size_binary("narrow", 0)
    with pytest.raises(ValueError, match="fixed_utf8"):
        yggdryl.string("narrow", layout="fixed_string")
    with pytest.raises(ValueError, match="expected no charset on ascii"):
        yggdryl.string("narrow", layout="ascii", charset="utf-8")
    with pytest.raises(TypeError, match="not both"):
        yggdryl.string("narrow", fixed=4, max=8)
    with pytest.raises(TypeError, match="not both"):
        yggdryl.bytes("narrow", fixed=4, max=8)
    with pytest.raises(ValueError, match="run_ends"):
        yggdryl.run_end_encoded(
            "encoded",
            yggdryl.int16("run_ends", nullable=True),
            yggdryl.utf8("values"),
        )


def test_field_and_datatype_equality_can_ignore_recursive_metadata() -> None:
    left_child = yggdryl.int32(
        "id", nullable=False, metadata={"source": "left"}
    )
    right_child = yggdryl.int32(
        "id", nullable=False, metadata={"source": "right"}
    )
    left = yggdryl.struct("row", [left_child], metadata={"root": "left"})
    right = yggdryl.struct("row", [right_child], metadata={"root": "right"})

    assert not left.equals(right)
    assert left.equals(right, with_metadata=False)
    assert not left.dtype.equals(right.dtype)
    assert left.dtype.equals(right.dtype, with_metadata=False)
    assert left.show_diff(left) == "✓ equal"
    assert left.dtype.show_diff(left.dtype) == "✓ equal"

    differences = left.show_diffs(right)
    assert isinstance(differences, Iterator)
    assert iter(differences) is differences
    lines = list(differences)
    assert any("metadata" in line and "≠" in line for line in lines)
    assert all("\x1b" not in line for line in lines)
    assert list(left.show_diffs(right, with_metadata=False)) == []


def test_differences_report_physical_layout_after_metadata_is_ignored() -> None:
    left = Field("value", DataType("int32"), nullable=False)
    right = Field("value", DataType("int64"), nullable=True)

    assert not left.equals(right, with_metadata=False)
    output = left.show_diff(right, with_metadata=False)
    assert "$.nullable" in output
    assert "$.dtype" in output
    assert "≠" in output
    assert output == "\n".join(left.show_diffs(right, with_metadata=False))


def test_wide_difference_iterator_is_lazy_and_outlives_field_sources() -> None:
    left = Field(
        "root",
        DataType.from_fields(
            Field(f"left_{index:04d}", "int32") for index in range(1024)
        ),
    )
    right = Field(
        "root",
        DataType.from_fields(
            Field(f"right_{index:04d}", "int32") for index in range(1024)
        ),
    )

    differences = left.show_diffs(right)
    assert not hasattr(differences, "__length_hint__")
    assert operator.length_hint(differences) == 0
    first = next(differences)
    assert first.startswith("≠ $.dtype.fields[0].name:")

    del left, right
    gc.collect()
    remaining = list(differences)

    assert len(remaining) == 1023
    assert remaining[-1].startswith("≠ $.dtype.fields[1023].name:")


def test_datatype_difference_iterator_outlives_source_wrappers() -> None:
    left = DataType.from_fields(
        [Field("value", "int64", metadata={"source": "left"})]
    )
    right = DataType.from_fields(
        [Field("value", "int64", metadata={"source": "right"})]
    )
    differences = left.show_diffs(right)

    del left, right
    gc.collect()

    assert list(differences) == [
        '≠ $.fields[0].metadata["source"]: "left" → "right"'
    ]


def test_map_factory_projects_exact_arrow_layout() -> None:
    mapping = yggdryl.map_of(
        "labels", "utf8", "int32", keys_sorted=True, nullable=False
    )
    arrow = mapping.into_arrow()

    assert arrow.type.equals(pa.map_(pa.string(), pa.int32(), keys_sorted=True))
    assert arrow.nullable is False


def test_dictionary_and_map_of_infer_python_and_pyarrow_type_inputs() -> None:
    dictionary = yggdryl.dictionary("status", int, str, nullable=False)
    mapping = yggdryl.map_of("labels", str, pa.int16(), nullable=False)

    assert str(dictionary.dtype) == "dictionary(int64,utf8)"
    assert mapping.dtype.id == "map"
    entries = mapping.dtype[0].dtype
    assert [field.dtype.id for field in entries] == ["utf8", "int16"]
