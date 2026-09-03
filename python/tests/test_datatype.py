from __future__ import annotations

import copy
import decimal
import enum
import inspect
import pickle
from typing import Optional

import pyarrow as pa
import pytest

from yggdryl import AsciiDictionary, DataType, Field


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


def test_ascii_widths_select_once_and_register_currency() -> None:
    ascii32 = DataType("ascii32")

    assert DataType.ascii(3) == ascii32
    assert DataType.ascii(8) == DataType("ascii64")
    assert DataType.ascii(12) == DataType("ascii128")
    assert DataType("ascii(3)") == ascii32
    assert DataType("currency") == ascii32
    assert str(ascii32) == "ascii32"
    assert eval(repr(ascii32), {"DataType": DataType}) == ascii32
    assert ascii32.id == "ascii32"
    assert ascii32.kind == "string"
    assert ascii32.ascii_width == 4
    assert DataType("ascii128").ascii_width == 16
    assert DataType("utf8").ascii_width is None

    assert DataType.from_logical_name("Currency") == ascii32
    assert DataType.logical_names() == {"currency": ascii32}
    assert list(DataType.logical_names()) == ["currency"]

    with pytest.raises(ValueError, match="currency"):
        DataType.from_logical_name("isin")
    with pytest.raises(ValueError, match="from 1 to 16 bytes, got 17"):
        DataType.ascii(17)
    with pytest.raises(ValueError, match="from 1 to 16 bytes, got 0"):
        DataType.ascii(0)
    with pytest.raises(ValueError):
        DataType("ascii")


def test_ascii_widths_pad_into_arrow_storage_and_trim_out_of_it() -> None:
    ascii32 = DataType("ascii32")
    ccy = Field("ccy", "ascii32")

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
        ccy.arrow_scalar("€")
    # Only text and bytes are ASCII values; nothing is stringified.
    with pytest.raises(ValueError, match="expected ascii32, got i64"):
        ascii32.arrow_scalar(3)
    with pytest.raises(ValueError, match="expected ascii32, got boolean"):
        ascii32.arrow_scalar(True)
    with pytest.raises(ValueError, match="expected ascii32"):
        ccy.cast(1.5)


def test_ascii_dictionary_registers_values_in_first_appearance_order() -> None:
    currencies = AsciiDictionary("ascii32")

    assert currencies.push("USD") == 0
    assert currencies.push("EUR") == 1
    assert currencies.push("USD") == 0
    # The padded spelling storage holds is the same value, in either shape.
    assert currencies.push("USD\x00") == 0
    assert currencies.push(b"USD\x00") == 0

    assert currencies.values == ["USD", "EUR"]
    assert len(currencies) == 2
    assert list(currencies) == ["USD", "EUR"]
    assert "EUR" in currencies and "JPY" not in currencies
    assert currencies.get(1) == "EUR"
    assert currencies.get(7) is None
    assert currencies.get_code("USD") == 0
    assert currencies.get_code("USD\x00") == 0
    assert currencies.get_code("JPY") is None

    assert currencies.dtype == DataType("dictionary(int32,ascii32)")
    assert currencies.key == DataType("int32")
    assert currencies.values_dtype == DataType("ascii32")

    seeded = AsciiDictionary.from_values("ascii32", ["USD", "EUR", "USD"])
    assert seeded.values == currencies.values
    assert seeded == currencies
    assert copy.copy(currencies) == currencies
    assert copy.deepcopy(currencies) == currencies

    wide = AsciiDictionary("ascii64", key="int64")
    assert wide.push("SEDOL1") == 0
    assert wide.dtype == DataType("dictionary(int64,ascii64)")


def test_ascii_dictionary_refuses_what_the_width_and_the_key_refuse() -> None:
    currencies = AsciiDictionary("ascii32")

    with pytest.raises(ValueError, match="at most 4 bytes, got 5 bytes"):
        currencies.push("EURO!")
    with pytest.raises(ValueError, match="at most 4 bytes, got a non-ASCII byte"):
        currencies.push("€")
    with pytest.raises(ValueError, match="at most 4 bytes, got a NUL byte at 2"):
        currencies.push("US\x00D")
    # Bytes meet the same width rule, never a decoding error of their own.
    with pytest.raises(ValueError, match="at most 4 bytes, got a non-ASCII byte 0xFF"):
        currencies.push(b"\xff\xfe")
    assert currencies.get_code(b"\xff") is None
    assert b"\xff" not in currencies
    # Only text and bytes are ASCII values; nothing is stringified.
    with pytest.raises(TypeError, match="must be str or bytes"):
        currencies.push(3)
    assert currencies.values == []

    # A column is an iterable of values: one string is one value, not its
    # characters.
    with pytest.raises(TypeError, match="not one string"):
        AsciiDictionary.from_values("ascii32", "USD")
    with pytest.raises(TypeError, match="not one string"):
        currencies.into_arrow_array("USD")
    assert currencies.values == []

    with pytest.raises(ValueError, match="an ASCII width"):
        AsciiDictionary("utf8")
    with pytest.raises(ValueError, match="an int32 or int64 key datatype"):
        AsciiDictionary("ascii32", key="int16")
    with pytest.raises(ValueError, match="at most 4 bytes"):
        AsciiDictionary.from_values("ascii32", ["USD", "EURO!"])


def test_ascii_dictionary_generates_an_intenum_from_the_core_member_listing() -> None:
    codes = AsciiDictionary.from_values("ascii32", ["USD", "n/a", "42", ""])

    Currency = codes.into_intenum("Currency")

    assert issubclass(Currency, enum.IntEnum)
    assert Currency.__name__ == "Currency"
    assert [(member.name, member.value) for member in Currency] == [
        ("USD", 0),
        ("N_A", 1),
        ("_42", 2),
        ("_", 3),
    ]
    assert Currency.USD == 0
    assert Currency(0).name == "USD"
    assert Currency["N_A"] == 1

    # A sixteen-byte vocabulary is text, not enum members.
    with pytest.raises(ValueError, match="ascii32 or ascii64 values"):
        AsciiDictionary.from_values("ascii128", ["USD"]).into_intenum("Wide")
    # A collision is named, never silently renamed.
    with pytest.raises(ValueError, match="both name the member N_A"):
        AsciiDictionary.from_values("ascii32", ["n/a", "n-a"]).into_intenum("Bad")
    # A name that opens and closes with `_` would be a reserved `_sunder_` or
    # `__dunder__`, so the trailing run goes and every member is a member.
    Shape = AsciiDictionary.from_values("ascii64", ["-a-", "--b--", "-"]).into_intenum(
        "Shape"
    )
    assert [(member.name, member.value) for member in Shape] == [
        ("_A", 0),
        ("__B", 1),
        ("_", 2),
    ]
    # The enum needs a name, the way the JavaScript binding needs one.
    with pytest.raises(ValueError, match="non-empty enum name"):
        codes.into_intenum("")


def test_ascii_dictionary_encodes_arrow_columns_under_continuing_codes() -> None:
    currencies = AsciiDictionary("ascii32")

    first = currencies.into_arrow_array(["USD", None, "EUR", "USD"])
    assert first.type == pa.dictionary(pa.int32(), pa.binary(4))
    assert first.indices.to_pylist() == [0, None, 1, 0]
    assert first.dictionary.to_pylist() == [b"USD\x00", b"EUR\x00"]
    assert first.to_pylist() == [b"USD\x00", None, b"EUR\x00", b"USD\x00"]

    # A second column continues the same codes, and an Arrow holder is a column.
    second = currencies.into_arrow_array(pa.array(["JPY", "EUR"]))
    assert second.indices.to_pylist() == [2, 1]
    assert currencies.values == ["USD", "EUR", "JPY"]

    recovered = AsciiDictionary.from_arrow_array(second)
    assert recovered.values == currencies.values
    assert recovered == currencies

    keyed = AsciiDictionary("ascii32", key="int64")
    wide = keyed.into_arrow_array(["USD"])
    assert wide.type == pa.dictionary(pa.int64(), pa.binary(4))
    assert AsciiDictionary.from_arrow_array(wide) == keyed

    with pytest.raises(ValueError, match="a dictionary array of int32 or int64"):
        AsciiDictionary.from_arrow_array(pa.array(["USD"]))
    with pytest.raises(ValueError, match="a dictionary array of int32 or int64"):
        AsciiDictionary.from_arrow_array(
            pa.array(["USD"]).dictionary_encode()
        )
    with pytest.raises(ValueError, match="at most 4 bytes"):
        currencies.into_arrow_array(["EURO!"])
    # A refused column registers nothing: the mutation fails atomically.
    with pytest.raises(ValueError, match="at most 4 bytes"):
        currencies.into_arrow_array(["GBP", "EURO!"])
    assert currencies.values == ["USD", "EUR", "JPY"]
    assert currencies.push("GBP") == 3

    # A vocabulary Arrow allows but a code cannot name: a repeat would shift
    # every later code.
    repeated = pa.DictionaryArray.from_arrays(
        pa.array([2, 0], type=pa.int32()),
        pa.array([b"USD\x00", b"USD\x00", b"EUR\x00"], type=pa.binary(4)),
    )
    with pytest.raises(ValueError, match="a vocabulary with no repeated value"):
        AsciiDictionary.from_arrow_array(repeated)


def test_ascii_dictionary_equality_is_the_width_the_key_and_the_value_order() -> None:
    left = AsciiDictionary.from_values("ascii32", ["USD", "EUR"])

    assert left == AsciiDictionary.from_values("ascii32", ["USD", "EUR"])
    assert left != AsciiDictionary.from_values("ascii32", ["EUR", "USD"])
    assert left != AsciiDictionary.from_values("ascii64", ["USD", "EUR"])
    assert left != AsciiDictionary.from_values("ascii32", ["USD", "EUR"], key="int64")
    assert left != "USD"

    assert repr(left) == (
        "AsciiDictionary.from_values(\"ascii32\", ['USD', 'EUR'], key=\"int32\")"
    )
    assert eval(repr(left), {"AsciiDictionary": AsciiDictionary}) == left
    # Registration moves the vocabulary, so the value carries no hash.
    with pytest.raises(TypeError):
        hash(left)
