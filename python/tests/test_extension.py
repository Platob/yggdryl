"""`yggdryl/extension.py`: the pyarrow extension types of the core's
`yggdryl.*` names, registered on import, and the crossings that carry them
both ways."""

from __future__ import annotations

import io
import pickle

import pyarrow as pa
import pyarrow.ipc
import pytest

from yggdryl import ChunkedSerie, DataType, Field, Scalar, Serie, SerieReader, extension
from yggdryl.extension import YggdrylArray, YggdrylType

#: One datatype per `yggdryl.*` name the core writes, each spelled as a
#: caller writes it.
SPELLED = {
    "yggdryl.string": "fixed_ascii(4)",
    "yggdryl.bytes": "sized_binary(16)",
    "yggdryl.decimal": "decimal",
    "yggdryl.bigdecimal": "bigdecimal",
    "yggdryl.version": "version",
    "yggdryl.url": "url",
    "yggdryl.urn": "urn",
    "yggdryl.timezone": "timezone",
    "yggdryl.mimetype": "mimetype",
    "yggdryl.mediatype": "mediatype",
    "yggdryl.state": "state",
    "yggdryl.marketdatakind": "marketdatakind",
    "yggdryl.marketdatatype": "marketdatatype",
    "yggdryl.side": "side",
    "yggdryl.timeinforce": "timeinforce",
    "yggdryl.country": "country",
    "yggdryl.ccy": "ccy",
    "yggdryl.mic": "mic",
    "yggdryl.cfi": "cfi",
    "yggdryl.isin": "isin",
    "yggdryl.cusip": "cusip",
    "yggdryl.sedol": "sedol",
    "yggdryl.figi": "figi",
    "yggdryl.ric": "ric",
    "yggdryl.bbg": "bbg",
    "yggdryl.unit": "unit",
    "yggdryl.forex": "forex",
    "yggdryl.lei": "lei",
    "yggdryl.bic": "bic",
    "yggdryl.elf": "elf",
    "yggdryl.dti": "dti",
    "yggdryl.fisn": "fisn",
}


def test_every_yggdryl_name_the_core_lists_is_registered() -> None:
    names = [name for name in DataType.ARROW_EXTENSION_NAMES if name.startswith("yggdryl.")]
    assert sorted(extension.names()) == sorted(names)
    assert sorted(SPELLED) == sorted(names), "one spelled datatype per name"
    # The canonical and community names are not ours to register.
    for name in ("arrow.uuid", "geoarrow.wkb", "arrow.parquet.variant"):
        assert name in DataType.ARROW_EXTENSION_NAMES
        assert name not in extension.names()


@pytest.mark.parametrize("name", sorted(SPELLED))
def test_a_datatype_crosses_as_its_extension_type_and_back(name: str) -> None:
    dtype = DataType(SPELLED[name])
    arrow = dtype.into_arrow()
    assert isinstance(arrow, YggdrylType), arrow
    assert arrow.extension_name == name
    assert arrow.datatype == dtype
    assert DataType.from_arrow(arrow) == dtype
    assert DataType(arrow) == dtype
    # A field crosses the same way, the name in its type rather than in its
    # metadata.
    field = Field("x", dtype)
    exported = field.into_arrow()
    assert exported.type == arrow
    assert not exported.metadata
    assert Field.from_arrow(exported) == field
    # The type pickles, document included.
    assert pickle.loads(pickle.dumps(arrow)) == arrow


@pytest.mark.parametrize("name", sorted(SPELLED))
def test_a_type_prints_as_pyarrows_own_extension_types_do(name: str) -> None:
    dtype = DataType(SPELLED[name])
    arrow = dtype.into_arrow()
    # A document tells two leaves of one storage apart, so it prints as the
    # datatype it states; a bare name prints alone, as `arrow.uuid` does.
    expected = f"extension<{name}[{dtype}]>" if arrow.document else f"extension<{name}>"
    assert str(arrow) == expected
    assert repr(arrow) == f"YggdrylType({expected})"
    column = Serie.from_scalars(Field("x", SPELLED[name]), []).into_arrow_array()
    assert str(column.type) == expected


def test_two_leaves_of_one_storage_print_apart() -> None:
    four = DataType("sized_ascii(4)").into_arrow()
    eight = DataType("sized_ascii(8)").into_arrow()
    assert str(four) == "extension<yggdryl.string[sized_ascii(4)]>"
    assert str(eight) == "extension<yggdryl.string[sized_ascii(8)]>"
    assert four != eight


def test_a_column_crosses_as_an_extension_array_and_lands_as_its_datatype() -> None:
    serie = Serie.from_scalars(Field("ccy", "ccy"), ["EUR", None, "USD"])
    arrow = serie.into_arrow_array()
    assert isinstance(arrow, pa.ExtensionArray)
    assert arrow.type.extension_name == "yggdryl.ccy"
    assert arrow.storage.to_pylist() == ["EUR", None, "USD"]
    # The scalar reads as the core reads it.
    assert arrow.to_pylist() == ["EUR", None, "USD"]
    assert arrow[0].as_py() == "EUR"
    # Back with no field: the type states the datatype.
    back = Serie.from_arrow_array(arrow)
    assert back.field.dtype == DataType("ccy")
    assert back.as_py() == ["EUR", None, "USD"]
    assert Serie.from_(arrow).field.dtype == DataType("ccy")
    # The PyCapsule interface carries it too.
    assert pa.array(serie).type == arrow.type
    # A chunked array lands as one chunk per array, of the datatype.
    chunked = pa.chunked_array([arrow, arrow])
    assert yggdryl_chunked_dtype(chunked) == DataType("ccy")


def yggdryl_chunked_dtype(value: pa.ChunkedArray) -> DataType:
    from yggdryl import ChunkedSerie

    assert ChunkedSerie.from_(value).field.dtype == ChunkedSerie.from_arrow_chunked_array(value).field.dtype
    return ChunkedSerie.from_arrow_chunked_array(value).field.dtype


def test_an_enum_column_reads_its_members_and_a_scalar_its_datatype() -> None:
    serie = Serie.from_scalars(Field("side", "side"), ["BUYS", "SELL"])
    arrow = serie.into_arrow_array()
    assert arrow.type.storage_type == pa.uint8()
    assert arrow.storage.to_pylist() == [1, 2]
    assert [value.as_py() for value in arrow] == [
        Scalar.from_(arrow[0]).as_py(),
        Scalar.from_(arrow[1]).as_py(),
    ]
    assert Scalar.from_(arrow[0]) == serie.scalar(0)


def test_a_batch_and_an_ipc_stream_carry_the_types_both_ways() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("ccy", pa.string()), pa.field("isin", pa.string())]), "row"
    )
    typed = Field("row", DataType.from_fields([Field("ccy", "ccy"), Field("isin", "isin")]))
    serie = Serie.from_scalars(typed, [["EUR", "US0378331005"]])
    batch = serie.into_arrow_batch()
    assert [field.type.extension_name for field in batch.schema] == [
        "yggdryl.ccy",
        "yggdryl.isin",
    ]
    sink = io.BytesIO()
    with pa.ipc.new_stream(sink, batch.schema) as writer:
        writer.write_batch(batch)
    read = pa.ipc.open_stream(sink.getvalue()).read_all()
    assert read.schema == batch.schema
    back = Serie.from_arrow_batch(read.to_batches()[0])
    assert back.field.dtype == typed.dtype
    assert root.dtype != typed.dtype


def test_a_document_over_storage_it_does_not_describe_reads_as_the_storage() -> None:
    # pyarrow keeps whatever the type states; the core decides what it is.
    foreign = extension.extension_type("yggdryl.ccy", pa.large_string())
    assert DataType.from_arrow(foreign) == DataType("large_utf8")
    with pytest.raises(KeyError, match="yggdryl extension name"):
        extension.extension_type("someorg.thing", pa.string())


#: The leaves laid out over a view under a `yggdryl.*` name.
VIEWS = [
    "ascii_view",
    "large_ascii_view",
    "cp1252_view",
    "large_cp1252_view",
    "large_utf8_view",
    "large_binary_view",
]

#: Longer than the twelve bytes a view holds inline, so its data buffer is
#: read.
LONG = "a value longer than twelve bytes"

#: Whether this pyarrow exports an extension over a view whole.
VIEWS_EXPORT_NESTED = int(pa.__version__.split(".")[0]) >= 21


def view_rows(spelled: str) -> list[object]:
    if "binary" in spelled:
        return [b"x", None, LONG.encode()]
    return ["x", None, LONG]


@pytest.mark.parametrize("spelled", VIEWS)
def test_a_view_leaf_crosses_back_whole_on_every_pyarrow(spelled: str) -> None:
    # pyarrow before 21 exports an extension array over a view without its
    # variadic buffers; the column crosses as its storage instead.
    dtype = DataType(spelled)
    rows = view_rows(spelled)
    arrow = Serie.from_scalars(Field("x", spelled), rows).into_arrow_array()
    assert isinstance(arrow.type, YggdrylType)
    assert arrow.to_pylist() == rows
    assert arrow[2].as_py() == rows[2]
    assert Serie.from_(arrow).field.dtype == dtype
    assert Serie.from_(arrow).as_py() == rows
    assert ChunkedSerie.from_(pa.chunked_array([arrow, arrow])).field.dtype == dtype
    batch = pa.record_batch([arrow], names=["x"])
    expected = [{"x": row} for row in rows]
    assert Serie.from_(batch).as_py() == expected
    assert Serie.from_(pa.table({"x": arrow})).as_py() == expected
    reader = pa.RecordBatchReader.from_batches(batch.schema, [batch])
    assert SerieReader.from_(reader).field == Serie.from_(batch).field


@pytest.mark.parametrize("spelled", VIEWS)
def test_a_view_leaf_below_a_column_crosses_from_pyarrow_21(spelled: str) -> None:
    rows = view_rows(spelled)
    arrow = Serie.from_scalars(Field("x", spelled), rows).into_arrow_array()
    items = pa.ListArray.from_arrays(pa.array([0, len(rows)], pa.int32()), arrow)
    nested = pa.record_batch([items], names=["items"])
    if VIEWS_EXPORT_NESTED:
        assert Serie.from_(nested).as_py() == [{"items": rows}]
        assert Serie.from_(items).as_py() == [rows]
    else:
        for value in (nested, items):
            with pytest.raises(ValueError, match="pyarrow 21 and later export it"):
                Serie.from_(value)


def test_an_extension_column_reads_its_values_in_one_pass() -> None:
    arrow = Serie.from_scalars(Field("ccy", "ccy"), ["EUR", None] * 500).into_arrow_array()
    assert isinstance(arrow, YggdrylArray)
    assert arrow.to_pylist() == ["EUR", None] * 500
    assert pa.chunked_array([arrow]).to_pylist() == ["EUR", None] * 500


def test_a_value_holding_an_extension_below_its_top_crosses_through_the_core() -> None:
    items = Field("x", "serie(ccy)")
    scalar = items.arrow_scalar(["EUR", "USD"])
    assert scalar.type.value_type.extension_name == "yggdryl.ccy"
    assert scalar.as_py() == ["EUR", "USD"]
    # The core's value rules hold at any depth.
    with pytest.raises(ValueError):
        items.arrow_scalar(["TOOLONGCCY"])
    row = Field("x", DataType.from_fields([Field("ccy", "ccy"), Field("side", "side")]))
    assert row.arrow_scalar({"ccy": "EUR", "side": "BUYS"})["ccy"].as_py() == "EUR"
    # A pyarrow scalar of such a type reads back as the value it holds.
    column = Serie.from_scalars(items, [["EUR"], ["USD", None]])
    arrow = column.into_arrow_array()
    assert Scalar.from_(arrow[1]) == column.scalar(1)
    assert Scalar.from_(Field("px", "decimal").arrow_scalar("1.5")) == Scalar.from_(
        Field("px", "decimal").arrow_scalar("1.50")
    )


@pytest.mark.parametrize("name", sorted(SPELLED))
def test_a_chunkless_chunked_array_lands_as_its_datatype(name: str) -> None:
    dtype = DataType(SPELLED[name])
    empty = pa.chunked_array([], type=dtype.into_arrow())
    assert ChunkedSerie.from_(empty).field.dtype == dtype
    assert ChunkedSerie.from_(empty).as_py() == []


def test_canonical_types_stay_pyarrows_own() -> None:
    assert DataType("uuid").into_arrow() == pa.uuid()
    assert DataType.from_arrow(pa.uuid()) == DataType("uuid")
    field = Field("id", "uuid")
    assert field.arrow_scalar("00000000-0000-0000-0000-000000000001").type == pa.uuid()
