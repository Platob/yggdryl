from __future__ import annotations

import copy
import decimal
import enum
import inspect
import pickle
import uuid
from typing import Optional

import pyarrow as pa
import pytest

from yggdryl import AsciiEnum, DataType, Field


def test_dtype_infers_native_string_and_arrow_values() -> None:
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

    assert DataType.decimal(18) == DataType("decimal128(18,0)")
    assert str(inspect.signature(DataType.decimal)) == "(precision, scale=0)"
    assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
    assert DataType.decimal("39", "-4") == DataType("decimal256(39,-4)")
    assert DataType.decimal(IndexValue(39), IndexValue(-4)) == DataType(
        "decimal256(39,-4)"
    )
    assert str(DataType.decimal("18", 2)) == "decimal128(18,2)"

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

    assert value.kind == "struct"
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
    assert variant.kind == "variant"
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


def test_the_guid_is_sixteen_bytes_spelled_as_one_identifier() -> None:
    guid = DataType("guid")

    assert DataType("uuid") == guid
    assert guid.id == "guid"
    assert guid.kind == "guid"
    assert str(guid) == "guid"
    assert guid.ascii_width is None

    # The identity is the sixteen bytes; the spelling is a rendering of them.
    text = "01912d68-783e-7c9a-b1f2-0123456789ab"
    packed = 0x01912D68783E7C9AB1F20123456789AB
    field = Field("id", guid, nullable=False)
    assert field.arrow_scalar(text) == pa.scalar(
        packed.to_bytes(16, "big"), pa.binary(16)
    )
    assert field.arrow_scalar(text.upper()) == field.arrow_scalar(text)
    assert field.arrow_scalar(text.replace("-", "")) == field.arrow_scalar(text)
    assert field.arrow_scalar(packed.to_bytes(16, "big")) == field.arrow_scalar(text)
    assert field.default_pyvalue() == "00000000-0000-0000-0000-000000000000"

    # Storage is the canonical `arrow.uuid` extension over sixteen bytes, so
    # PyArrow rebuilds its own registered extension type from the projection.
    arrow = field.into_arrow()
    assert arrow.type == pa.uuid()
    assert arrow.type.storage_type == pa.binary(16)
    assert Field.from_arrow(arrow) == field

    # A cast into the type validates; the stored column read as text spells it.
    stored = field.cast_arrow_array(pa.array([text, text.upper()]))
    # PyArrow reads its own registered extension back as `uuid.UUID`.
    assert stored.to_pylist() == [uuid.UUID(text)] * 2
    assert stored.storage.to_pylist() == [packed.to_bytes(16, "big")] * 2
    # A recognized identifier column renders as its spelling, exactly as a
    # recognized ASCII column renders as its trimmed text.
    batch = pa.record_batch([stored], schema=pa.schema([arrow]))
    spelled = DataType.from_fields([Field("id", "utf8")])
    assert spelled.cast_arrow_batch(batch).column(0).to_pylist() == [text, text]

    with pytest.raises(ValueError, match="36-character"):
        field.cast_arrow_array(pa.array(["not-a-guid"]))


def test_ascii_is_one_variable_form_and_one_fixed_width() -> None:
    ascii_text = DataType("ascii")
    fixed = DataType.ascii(3)

    # Variable ASCII stores the bytes it is given, so it has no width.
    assert ascii_text.id == "ascii"
    assert ascii_text.kind == "string"
    assert str(ascii_text) == "ascii"
    assert ascii_text.ascii_width is None
    assert eval(repr(ascii_text), {"DataType": DataType}) == ascii_text

    # A fixed width is the width, so two widths are two datatypes and neither
    # is the variable form.
    assert fixed.id == "fixed_ascii"
    assert fixed.kind == "string"
    assert str(fixed) == "ascii(3)"
    assert fixed.ascii_width == 3
    assert DataType("ascii(3)") == fixed
    assert eval(repr(fixed), {"DataType": DataType}) == fixed
    assert DataType.ascii(4) != fixed
    assert fixed != ascii_text
    # Any width of at least one byte is storage; only the packed integer
    # stops at sixteen bytes.
    assert DataType.ascii(64).ascii_width == 64
    assert DataType("utf8").ascii_width is None

    # A name is one more spelling of a datatype, and it folds case, `_`, `-`,
    # and spaces the way the grammar folds them.
    names = DataType.logical_names()
    assert names["price"] == DataType("decimal64(18,8)")
    assert DataType("Price") == names["price"]
    assert DataType.from_logical_name("UTC_Timestamp") == DataType('timestamp(ns,"UTC")')
    # The base-type spellings the Arrow/SQL grammar owns keep their meaning.
    assert DataType("int") == DataType("int32")
    assert DataType("float") == DataType("float32")

    with pytest.raises(ValueError, match="currency"):
        DataType.from_logical_name("isin")
    with pytest.raises(ValueError, match="at least 1 byte, got 0"):
        DataType.ascii(0)


def test_a_registered_code_is_its_own_datatype() -> None:
    # ISO 3166-1 is two letters, ISO 4217 three, ISO 10383 four, and ISO 10962
    # six: each is a datatype storing exactly that, not a name over a width.
    # The four are the registrations whose name answers a type of its own.
    currency = DataType.from_logical_name("Currency")
    assert currency == DataType("currency")
    assert DataType.logical_names()["currency"] == currency
    assert DataType.from_logical_name("Exchange") == DataType("mic")

    assert currency.id == "currency"
    assert currency.kind == "string"
    assert str(currency) == "currency"
    assert currency.ascii_width == 3
    assert currency != DataType.ascii(3)
    assert DataType(" CURRENCY ") == currency
    assert eval(repr(currency), {"DataType": DataType}) == currency

    for name, width in [("country", 2), ("currency", 3), ("mic", 4), ("cfi", 6)]:
        dtype = DataType(name)
        assert (dtype.id, dtype.ascii_width, dtype.kind) == (name, width, "string")

    # The packed integer is the value's own bytes, exactly as for a width.
    assert currency.ascii_packed("USD") == DataType.ascii(3).ascii_packed("USD")
    assert currency.ascii_value(0x555344) == "USD"
    with pytest.raises(ValueError, match="at most 2 bytes"):
        DataType("country").ascii_packed("USD")
    with pytest.raises(ValueError, match="unknown datatype"):
        DataType("isin")


def test_a_registered_code_carries_its_identity_across_arrow() -> None:
    ccy = Field("ccy", "currency")
    arrow_field = ccy.into_arrow()

    assert arrow_field.type == pa.binary(3)
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.currency",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow_field) == ccy

    # The same three bytes under the width's own name are the width, and
    # under no name at all are a plain fixed binary.
    assert Field.from_arrow(Field("ccy", DataType.ascii(3)).into_arrow()) == Field(
        "ccy", DataType.ascii(3)
    )
    assert Field.from_arrow(pa.field("ccy", pa.binary(3))) == Field(
        "ccy", "fixed_size_binary(3)"
    )

    assert ccy.arrow_scalar("USD") == pa.scalar(b"USD", pa.binary(3))
    assert ccy.cast_arrow_array(pa.array(["USD", "EU"])).to_pylist() == [
        b"USD",
        b"EU\x00",
    ]
    with pytest.raises(ValueError, match="at most 3 bytes"):
        ccy.cast_arrow_array(pa.array(["EURO"]))


def test_a_fixed_ascii_width_pads_into_arrow_storage_and_trims_out_of_it() -> None:
    ascii32 = DataType.ascii(4)
    ccy = Field("ccy", ascii32)

    assert ascii32.into_arrow() == pa.binary(4)
    arrow_field = ccy.into_arrow()
    assert arrow_field.type == pa.binary(4)
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.ascii",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow_field) == ccy
    assert Field.from_arrow(pa.field("ccy", pa.binary(4))) == Field(
        "ccy", "fixed_size_binary(4)"
    )

    assert ascii32.arrow_scalar("USD") == pa.scalar(b"USD\x00", pa.binary(4))
    assert ascii32.arrow_scalar(b"USD\x00") == pa.scalar(b"USD\x00", pa.binary(4))
    assert ascii32.arrow_scalar(None) == pa.scalar(None, pa.binary(4))
    assert ccy.arrow_scalar("EUR") == pa.scalar(b"EUR\x00", pa.binary(4))
    assert ascii32.default_pyvalue() == ""
    assert ascii32.default_pyhint() is str
    assert ascii32.default_arrow_scalar() == pa.scalar(b"\x00" * 4, pa.binary(4))

    padded = ccy.cast_arrow_array(pa.array(["USD", None]))
    assert padded.type == pa.binary(4)
    assert padded.to_pylist() == [b"USD\x00", None]
    # A datatype casts as a required column: nulls fill with the default.
    filled = ascii32.cast_arrow_array(pa.array(["USD", None]))
    assert filled.to_pylist() == [b"USD\x00", b"\x00" * 4]

    row = DataType.from_fields([Field("ccy", "utf8")])
    stored = pa.record_batch([padded], schema=pa.schema([arrow_field]))
    assert row.cast_arrow_batch(stored).column(0).to_pylist() == ["USD", None]

    with pytest.raises(ValueError, match="at most 4 bytes"):
        ascii32.cast_arrow_array(pa.array(["EURO!"]))
    with pytest.raises(ValueError, match="at most 4 bytes"):
        ascii32.arrow_scalar("EURO!")
    with pytest.raises(ValueError, match="non-ASCII"):
        ccy.arrow_scalar("\u20ac")
    # Only text and bytes are ASCII values; nothing is stringified.
    with pytest.raises(ValueError, match="got i64"):
        ascii32.arrow_scalar(3)
    with pytest.raises(ValueError, match="got boolean"):
        ascii32.arrow_scalar(True)
    with pytest.raises(ValueError):
        ccy.cast(1.5)


def test_variable_ascii_stores_the_bytes_it_is_given() -> None:
    note = DataType("ascii")
    field = Field("note", note)

    # No width, so no padding: variable ASCII is Arrow's variable binary under
    # the same extension name, told apart from the fixed form by its storage.
    assert note.into_arrow() == pa.binary()
    arrow_field = field.into_arrow()
    assert arrow_field.type == pa.binary()
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.ascii",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow_field) == field
    assert Field.from_arrow(pa.field("note", pa.binary())) == Field("note", "binary")

    assert note.arrow_scalar("free text") == pa.scalar(b"free text", pa.binary())
    assert note.default_pyvalue() == ""
    assert note.default_pyhint() is str
    assert note.default_arrow_scalar() == pa.scalar(b"", pa.binary())

    stored = field.cast_arrow_array(pa.array(["a", "much longer note", None]))
    assert stored.to_pylist() == [b"a", b"much longer note", None]
    row = DataType.from_fields([Field("note", "utf8")])
    batch = pa.record_batch([stored], schema=pa.schema([arrow_field]))
    assert row.cast_arrow_batch(batch).column(0).to_pylist() == [
        "a",
        "much longer note",
        None,
    ]

    # The value contract is the width's, minus the width itself.
    with pytest.raises(ValueError, match="non-ASCII"):
        note.arrow_scalar("\u20ac")
    with pytest.raises(ValueError, match="NUL"):
        note.arrow_scalar("a\x00b")
    # A packed integer needs a width, so the variable form has none.
    with pytest.raises(ValueError, match="at most 16 bytes"):
        note.ascii_packed("USD")




def test_a_prebuilt_vocabulary_names_the_iso_codes_a_column_carries() -> None:
    prebuilt = AsciiEnum.prebuilt()
    assert set(prebuilt) == {"currency", "country", "mic", "exchange"}
    # `exchange` is FIX's name for the ISO 10383 code, so it is one list.
    assert prebuilt["mic"] == prebuilt["exchange"]

    countries = AsciiEnum.from_logical_name("Country")
    assert countries.name == "country"
    assert len(countries) == len(prebuilt["country"])
    # An ISO code names itself, so the member and its value are one spelling.
    assert countries.get("FR") == "FR"
    assert countries.get_member("FR") == "FR"
    assert "FR" in countries
    # A prebuilt listing is a constant, so a second build is the same enum.
    assert AsciiEnum.from_logical_name("country") == countries
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
    assert len(AsciiEnum.from_logical_name("tenor")) == 0
    with pytest.raises(ValueError, match="currency"):
        AsciiEnum.from_logical_name("isin")


def test_an_enum_member_name_is_the_one_rule_both_runtimes_apply() -> None:
    assert AsciiEnum.member_name("n/a") == "N_A"
    assert AsciiEnum.member_name("3M") == "_3M"
    assert AsciiEnum.member_name("") == "_"
    # A name that opens and closes with `_` would be a reserved `_sunder_` or
    # `__dunder__`, so the trailing run goes and every member is a member.
    assert AsciiEnum.member_name("-a-") == "_A"
    assert AsciiEnum.member_name("--b--") == "__B"

    codes = AsciiEnum("Currency", {"USD": "USD", "N_A": "n/a"})
    width = DataType.ascii(4)
    assert codes.into_members(width) == [
        ("N_A", 0x6E2F6100),
        ("USD", 0x55534400),
    ]
    # A value the width could not store is refused by the width, never
    # silently truncated into a member.
    with pytest.raises(ValueError, match="at most 2 bytes"):
        codes.into_members(DataType.ascii(2))
