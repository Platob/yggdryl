"""One Arrow cast compiled from two fields, reused across every column of the source layout."""

from __future__ import annotations

import struct
from typing import Any

import pyarrow as pa
import pytest

from yggdryl import (
    ArrowCastPlan,
    ChunkedSerie,
    DataType,
    Field,
    Serie,
    SerieReader,
    StructSerie,
)
from yggdryl.enums import NULLABILITIES, REPRESENTATIONS

ROOT = Field("row", "struct<id:int64,symbol:utf8>", nullable=False)
SOURCE = pa.schema([pa.field("id", pa.int32()), pa.field("symbol", pa.string())])


def quotes(ids: list[int], symbols: list[str]) -> pa.RecordBatch:
    return pa.record_batch(
        {"id": pa.array(ids, pa.int32()), "symbol": pa.array(symbols)},
    )


def test_a_plan_answers_the_fields_it_was_compiled_between() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)

    # A schema is the record root `row` its columns are children of.
    assert plan.target == ROOT
    assert isinstance(plan.source, pa.Field)
    assert plan.source.name == "row"
    assert plan.source.type == pa.struct(list(SOURCE))
    assert not plan.is_identity
    assert plan.safe
    assert plan.nullability == "default"
    assert plan.representation == "value"
    assert "ArrowCastPlan" in repr(plan)

    # A field and a pyarrow field are sources too.
    column = ArrowCastPlan(Field("id", "int32"), Field("id", "int64"))
    assert column.source == pa.field("id", pa.int32())
    assert ArrowCastPlan(pa.field("id", pa.int32()), "id: int64").target == Field("id", "int64")

    # The three cast answers cross explicitly, and each has a vocabulary.
    strict = ArrowCastPlan(SOURCE, ROOT, safe=False, nullability="strict", representation="bits")
    assert not strict.safe
    assert strict.nullability == "strict"
    assert strict.representation == "bits"
    assert set(NULLABILITIES) == {"default", "strict"}
    assert set(REPRESENTATIONS) == {"value", "bits"}

    with pytest.raises(ValueError):
        ArrowCastPlan(SOURCE, ROOT, nullability="lenient")


def test_one_plan_answers_every_column_of_its_source_layout() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)

    first = plan.apply(quotes([1, 2], ["AAPL", "MSFT"]))
    second = plan.apply(Serie.from_arrow_batch(quotes([3], ["AMD"])))

    assert type(first) is StructSerie
    assert first.field == ROOT
    assert second.field == ROOT
    assert first.into_arrow_batch().schema.field("id").type == pa.int64()
    assert first.child("id").as_py() == [1, 2]
    assert second.child("id").as_py() == [3]

    # The same answer the one-shot door gives, so the plan is only reuse.
    assert first == Serie.from_arrow_batch(quotes([1, 2], ["AAPL", "MSFT"]), ROOT)
    assert first == Serie.from_arrow_batch(quotes([1, 2], ["AAPL", "MSFT"])).cast(ROOT)

    # A column of another layout is refused, naming both.
    with pytest.raises(ValueError, match="compiled for"):
        plan.apply(pa.array([1, 2], pa.int64()))


def test_an_identity_plan_shares_the_columns_buffers() -> None:
    plan = ArrowCastPlan(Field("id", "int64"), Field("id", "int64"))
    assert plan.is_identity

    array = pa.array([1, 2, 3], pa.int64())
    cast = plan.apply(array)
    assert cast.field == Field("id", "int64")
    assert cast.into_arrow_array().buffers()[1].address == array.buffers()[1].address

    # The column itself comes back when it is already under the target.
    held = Serie.from_arrow_array(array, Field("id", "int64"))
    assert plan.apply(held).into_arrow_array().buffers()[1].address == array.buffers()[1].address


def test_a_table_or_a_chunked_column_is_cast_chunk_by_chunk() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)
    table = pa.Table.from_batches([quotes([1, 2], ["AAPL", "MSFT"]), quotes([3], ["AMD"])])

    # A table is one chunk per batch, and every chunk crosses the one plan.
    cast = plan.apply(table)
    assert isinstance(cast, ChunkedSerie)
    assert cast.field == ROOT
    assert cast.num_chunks == 2
    assert [len(chunk) for chunk in cast.chunks] == [2, 1]
    out = cast.into_arrow_table()
    assert out.schema.field("id").type == pa.int64()
    assert [batch.num_rows for batch in out.to_batches()] == [2, 1]
    assert cast == Serie.from_(table, ROOT)

    # A native chunked serie answers the same, and so does a chunked array.
    assert plan.apply(ChunkedSerie.from_(table)) == cast
    ids = ArrowCastPlan(Field("item", "int32"), Field("id", "int64"))
    chunked = ids.apply(pa.chunked_array([[1, 2], [3]], pa.int32()))
    assert isinstance(chunked, ChunkedSerie)
    assert chunked.field == Field("id", "int64")
    assert chunked.num_chunks == 2
    assert chunked.as_py() == [1, 2, 3]

    # A chunk the plan was not compiled for is refused, naming both.
    with pytest.raises(ValueError, match="compiled for"):
        plan.apply(pa.chunked_array([[1, 2]], pa.int64()))


def test_an_identity_plan_shares_every_chunks_buffers() -> None:
    plan = ArrowCastPlan(Field("item", "int64"), Field("item", "int64"))
    source = pa.chunked_array([[1, 2], [3]], pa.int64())
    cast = plan.apply(source)
    assert isinstance(cast, ChunkedSerie)
    for chunk, original in zip(cast.chunks, source.chunks):
        assert chunk.into_arrow_array().buffers()[1].address == original.buffers()[1].address


def test_a_plan_refuses_at_compile_time_what_two_fields_alone_decide() -> None:
    required = Field.from_str(
        'field("row",struct<id:int64,symbol:utf8 not null>,nullable=false)'
    )
    partial = pa.schema([pa.field("id", pa.int32())])

    # `default` writes the target's own default into the column the source
    # cannot fill; `strict` refuses, and refuses before any column exists.
    partial_batch = pa.record_batch({"id": pa.array([1], pa.int32())})
    filled = ArrowCastPlan(partial, required).apply(partial_batch)
    assert filled.as_py() == [{"id": 1, "symbol": ""}]
    with pytest.raises(ValueError):
        ArrowCastPlan(partial, required, nullability="strict")

    # A column the target does not declare is dropped rather than refused,
    # so a wider source still compiles against a narrower root.
    wider = pa.schema(
        [
            pa.field("id", pa.int32()),
            pa.field("symbol", pa.string()),
            pa.field("venue", pa.string()),
        ]
    )
    batch = pa.record_batch(
        {"id": pa.array([1], pa.int32()), "symbol": ["AAPL"], "venue": ["XNAS"]}
    )
    assert ArrowCastPlan(wider, ROOT).apply(batch).names == ["id", "symbol"]


def test_preflight_runs_the_plan_over_no_rows() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)
    assert plan.preflight() is None

    # Everything the values themselves decide is left to `apply`, because no
    # value has been seen yet: a plan over a narrowing conversion compiles and
    # preflights, and the out-of-range value is answered per column.
    source = pa.schema([pa.field("id", pa.int64()), pa.field("symbol", pa.string())])
    target = Field("row", "struct<id:int8,symbol:utf8>", nullable=False)
    overflowing = pa.record_batch(
        {"id": pa.array([9999], pa.int64()), "symbol": pa.array(["AAPL"])}
    )

    lenient = ArrowCastPlan(source, target, safe=True)
    assert lenient.preflight() is None
    assert lenient.apply(overflowing).child("id").as_py() == [None]

    checked = ArrowCastPlan(source, target, safe=False)
    assert checked.preflight() is None
    with pytest.raises(ValueError):
        checked.apply(overflowing)


def test_a_one_row_cast_is_a_one_row_column() -> None:
    int64 = Field("value", "int64")
    assert Serie.from_arrow_array(pa.array([1], pa.int32()), int64).into_arrow_scalar().as_py() == 1
    assert DataType("int64").arrow_scalar(5).as_py() == 5
    text = Serie.from_arrow_array(pa.array(["AAPL"]), Field("value", "utf8"))
    assert text.into_arrow_scalar().as_py() == "AAPL"

    # A scalar is exactly one row.
    with pytest.raises(ValueError, match="exactly one row"):
        Serie.from_arrow_array(pa.array([1, 2], pa.int32()), int64).into_arrow_scalar()


def test_an_empty_text_cell_is_null_before_safe_is_asked() -> None:
    empty = pa.array([""], pa.string())

    # A zero-length text cell entering a column that does not hold text is
    # no value: it is null before any spelling is read, so `safe` never sees
    # it and both policies answer the same null.
    nullable = Field("quantity", "int32")
    assert Serie.from_arrow_array(empty, nullable, safe=False).as_py() == [None]
    assert Serie.from_arrow_array(empty, nullable, safe=True).as_py() == [None]

    # A required column then answers its nullability, exactly as it does for
    # a null the source carried: the default repairs it, strictness refuses
    # it naming the path and the count.
    required = Field("quantity", "int32", nullable=False)
    assert Serie.from_arrow_array(empty, required).as_py() == [0]
    with pytest.raises(
        ValueError, match=r"required Arrow field \$\.quantity holds 1 null values"
    ):
        Serie.from_arrow_array(empty, required, nullability="strict")

    # The same plan compiled between two schemas answers the same way.
    source = pa.schema([pa.field("quantity", pa.string())])
    target = Field("row", "struct<quantity:int32 not null>", nullable=False)
    batch = pa.record_batch({"quantity": empty})
    assert ArrowCastPlan(source, target, safe=False).apply(batch).child(
        "quantity"
    ).as_py() == [0]
    with pytest.raises(
        ValueError, match=r"required Arrow field \$\.quantity holds 1 null values"
    ):
        ArrowCastPlan(source, target, nullability="strict").apply(batch)

    # Text is text: into a string column the empty cell is the value it is.
    assert Serie.from_arrow_array(empty, Field("symbol", "utf8")).as_py() == [""]
    assert Serie.from_arrow_array(
        empty, Field("symbol", "utf8", nullable=False), nullability="strict"
    ).as_py() == [""]

    # The scalar door reads the same rule.
    assert DataType("int32").scalar("").is_null()
    assert DataType("utf8").scalar("").as_py() == ""
    with pytest.raises(ValueError, match="non-nullable field received null"):
        Field("quantity", "int32", nullable=False).scalar("")


class TestArrowNullability:
    """What a cast does about a required field the source cannot fill."""

    @staticmethod
    def _root() -> Any:
        return Field(
            "row", DataType("struct<id: int64, symbol: string not null>"), False
        )

    @staticmethod
    def _identifiers() -> Any:
        return pa.record_batch({"id": pa.array([1, 2], pa.int64())})

    def test_a_missing_required_column_is_filled_or_named(self) -> None:
        root = self._root()
        batch = self._identifiers()

        # Default repairs the hole with the target's canonical default.
        filled = Serie.from_arrow_batch(batch, root).into_arrow_batch()
        assert filled.column_names == ["id", "symbol"]
        assert filled.column("symbol").to_pylist() == ["", ""]
        assert filled.schema.field("symbol").nullable is False

        # Strict refuses it, from the two schemas alone, and names the path.
        with pytest.raises(
            ValueError,
            match=r"required Arrow field \$\.symbol is missing from the source",
        ):
            Serie.from_arrow_batch(batch, root, nullability="strict").into_arrow_batch()

        # The policy is a name, and an unknown one is refused by vocabulary.
        with pytest.raises(ValueError, match="expected one of default, strict"):
            Serie.from_arrow_batch(batch, root, nullability="lenient").into_arrow_batch()

    def test_a_required_column_holding_null_is_named_with_its_count(self) -> None:
        root = self._root()
        batch = pa.record_batch(
            {
                "id": pa.array([1, 2, 3], pa.int64()),
                "symbol": pa.array(["AAPL", None, None], pa.string()),
            }
        )

        assert Serie.from_arrow_batch(
            batch, root
        ).into_arrow_batch().column("symbol").to_pylist() == [
            "AAPL",
            "",
            "",
        ]
        with pytest.raises(
            ValueError, match=r"required Arrow field \$\.symbol holds 2 null values"
        ):
            Serie.from_arrow_batch(batch, root, nullability="strict").into_arrow_batch()

    def test_a_nullable_column_the_source_lacks_stays_null_under_both(self) -> None:
        # Nothing is required here, so strictness has nothing to refuse.
        root = Field("row", DataType("struct<id: int64, symbol: string>"), False)
        batch = self._identifiers()

        for nullability in ("default", "strict"):
            cast = Serie.from_arrow_batch(batch, root, nullability=nullability).into_arrow_batch()
            assert cast.column_names == ["id", "symbol"]
            assert cast.column("symbol").null_count == 2
            assert cast.column("symbol").to_pylist() == [None, None]

    def test_an_undeclared_source_column_is_dropped_under_both(self) -> None:
        root = self._root()
        batch = pa.record_batch(
            {
                "id": pa.array([1], pa.int64()),
                "symbol": pa.array(["AAPL"], pa.string()),
                "venue": pa.array(["XNAS"], pa.string()),
            }
        )

        # Strictness is about what the target declares, not about what the
        # source carries beyond it.
        for nullability in ("default", "strict"):
            cast = Serie.from_arrow_batch(batch, root, nullability=nullability).into_arrow_batch()
            assert cast.column_names == ["id", "symbol"]

    def test_a_nested_required_field_is_named_by_its_whole_path(self) -> None:
        root = Field(
            "row",
            DataType("struct<account: struct<id: int64, zip: string not null>>"),
            False,
        )
        without_zip = pa.record_batch(
            {
                "account": pa.array(
                    [{"id": 1}], pa.struct([pa.field("id", pa.int64())])
                )
            }
        )
        with_null_zip = pa.record_batch(
            {
                "account": pa.array(
                    [{"id": 1, "zip": "75001"}, {"id": 2, "zip": None}],
                    pa.struct(
                        [pa.field("id", pa.int64()), pa.field("zip", pa.string())]
                    ),
                )
            }
        )

        assert Serie.from_arrow_batch(without_zip, root).into_arrow_batch().to_pylist() == [
            {"account": {"id": 1, "zip": ""}}
        ]
        with pytest.raises(
            ValueError,
            match=r"required Arrow field \$\.account\.zip is missing from the source",
        ):
            Serie.from_arrow_batch(without_zip, root, nullability="strict").into_arrow_batch()
        with pytest.raises(
            ValueError,
            match=r"required Arrow field \$\.account\.zip holds 1 null values",
        ):
            Serie.from_arrow_batch(with_null_zip, root, nullability="strict").into_arrow_batch()

        # A collection is one step of that path too, spelled with brackets.
        listed = Field(
            "row", DataType("struct<users: serie<struct<zip: string not null>>>"), False
        )
        rows = pa.record_batch(
            {
                "users": pa.array(
                    [[{"zip": None}, {"zip": "75001"}], [{"zip": None}]],
                    pa.list_(pa.struct([pa.field("zip", pa.string())])),
                )
            }
        )
        with pytest.raises(
            ValueError,
            match=r"required Arrow field \$\.users\[\]\.zip holds 2 null values",
        ):
            Serie.from_arrow_batch(rows, listed, nullability="strict").into_arrow_batch()

    def test_safe_and_strictness_are_two_independent_answers(self) -> None:
        root = Field(
            "row", DataType("struct<id: int64, quantity: int8 not null>"), False
        )
        batch = pa.record_batch(
            {
                "id": pa.array([1, 2], pa.int64()),
                "quantity": pa.array([7, 130], pa.int64()),
            }
        )

        # `safe` turns the value the target cannot hold into a null, which the
        # default policy then repairs.
        assert Serie.from_arrow_batch(
            batch, root
        ).into_arrow_batch().column("quantity").to_pylist() == [7, 0]

        # Strictness refuses the null a lenient conversion would leave in a
        # required column, so the conversion is refused by the value itself.
        with pytest.raises(ValueError, match="Can't cast value 130 to type Int8"):
            Serie.from_arrow_batch(batch, root, nullability="strict").into_arrow_batch()

        # `safe=False` refuses the conversion itself, before any policy about
        # absence applies - so both policies raise the same conversion error.
        for nullability in ("default", "strict"):
            with pytest.raises(ValueError, match="Can't cast value 130 to type Int8"):
                Serie.from_arrow_batch(
                    batch, root, safe=False, nullability=nullability
                ).into_arrow_batch()

    def test_a_strict_reader_refuses_when_the_batch_is_pulled(self) -> None:
        root = self._root()
        stored = pa.schema(
            [pa.field("id", pa.int64()), pa.field("symbol", pa.string())]
        )
        batch = pa.record_batch(
            {
                "id": pa.array([1, 2], pa.int64()),
                "symbol": pa.array(["AAPL", None], pa.string()),
            },
            schema=stored,
        )

        # A null is a property of rows, so the reader is built and answers its
        # schema before anything refuses it.
        reader = SerieReader.from_arrow_reader(
            pa.RecordBatchReader.from_batches(stored, [batch]), root, nullability="strict"
        ).into_arrow_reader()
        assert reader.schema.names == ["id", "symbol"]
        with pytest.raises(
            pa.ArrowInvalid,
            match=r"required Arrow field \$\.symbol holds 1 null values",
        ):
            reader.read_all()

        # A missing column is a property of the schemas, so it is refused where
        # the schemas meet: building the reader, with no batch pulled.
        absent = pa.schema([pa.field("id", pa.int64())])
        with pytest.raises(
            ValueError,
            match=r"required Arrow field \$\.symbol is missing from the source",
        ):
            SerieReader.from_arrow_reader(
                pa.RecordBatchReader.from_batches(absent, []), root, nullability="strict"
            ).into_arrow_reader()


def test_the_bits_reading_crosses_every_same_width_pair() -> None:
    bits: dict[str, Any] = {"representation": "bits"}

    unsigned32 = pa.array(
        [0, 2**31 - 1, 2**31, 2**32 - 1, None],
        type=pa.uint32(),
    )
    signed32 = Serie.from_arrow_array(
        unsigned32, Field("digest", "int32"), **bits
    ).into_arrow_array()
    assert signed32.type == pa.int32()
    assert signed32.to_pylist() == [0, 2**31 - 1, -(2**31), -1, None]
    assert Serie.from_arrow_array(
        signed32, Field("digest", "uint32"), **bits
    ).into_arrow_array().equals(unsigned32)

    unsigned64 = pa.array(
        [0, 2**63 - 1, 2**63, 2**64 - 1, None],
        type=pa.uint64(),
    )
    signed64 = Serie.from_arrow_array(
        unsigned64, Field("digest", "int64"), **bits
    ).into_arrow_array()
    assert signed64.type == pa.int64()
    assert signed64.to_pylist() == [0, 2**63 - 1, -(2**63), -1, None]
    assert Serie.from_arrow_array(
        signed64, Field("digest", "uint64"), **bits
    ).into_arrow_array().equals(unsigned64)

    # Eight bytes are eight bytes: the integer, its opposite sign and the raw
    # payload are one buffer under three readings, and the chain round-trips.
    stored = Serie.from_arrow_array(
        unsigned64, Field("digest", "fixed_size_binary(8)"), **bits
    ).into_arrow_array()
    assert stored.type == pa.binary(8)
    assert stored.to_pylist()[3] == b"\xff" * 8
    assert Serie.from_arrow_array(
        stored, Field("digest", "uint64"), **bits
    ).into_arrow_array().equals(unsigned64)

    # The reading says what the bytes mean; nullability still says what an
    # absent value means.
    required = Serie.from_arrow_array(
        pa.array([None, 2**64 - 1], type=pa.uint64()),
        Field("digest", "int64", nullable=False),
        **bits,
    ).into_arrow_array()
    assert required.to_pylist() == [0, -1]
    with pytest.raises(ValueError, match=r"\$\.digest"):
        Serie.from_arrow_array(
            pa.array([None], type=pa.uint64()),
            Field("digest", "int64", nullable=False),
            nullability="strict",
            **bits,
        ).into_arrow_array()


def test_asking_for_bits_never_reinterprets_a_different_width() -> None:
    bits: dict[str, Any] = {"representation": "bits"}

    # Four bytes are not eight, so this is the ordinary numeric widening.
    widened = Serie.from_arrow_array(
        pa.array([7], type=pa.uint32()), Field("digest", "int64"), **bits
    ).into_arrow_array()
    assert widened.to_pylist() == [7]

    # And a datatype whose values follow a rule keeps that rule: a safe cast
    # nulls the cell it cannot read, a strict one names it.
    assert Serie.from_arrow_array(
        pa.array([b"\xff\xff\xff\xff"], type=pa.binary(4)), Field("ccy", "fixed_ascii(4)"), **bits
    ).into_arrow_array().to_pylist() == [None]
    with pytest.raises(ValueError, match="ccy"):
        Serie.from_arrow_array(
            pa.array([b"\xff\xff\xff\xff"], type=pa.binary(4)),
            Field("ccy", "fixed_ascii(4)"),
            safe=False,
            **bits,
        ).into_arrow_array()


def test_apply_proves_foreign_data_before_a_row_is_read() -> None:
    # Offsets that step backwards pass pyarrow's cheap check; landed
    # unproven, reading the rows would read past the value buffer.
    offsets = pa.py_buffer(struct.pack("<4i", 0, 4, 1, 4))
    array = pa.Array.from_buffers(pa.binary(), 3, [None, offsets, pa.py_buffer(b"abcd")])
    array.validate()
    plan = ArrowCastPlan(Field("item", "binary"), Field("item", "large_binary"))
    for value in (array, pa.chunked_array([array])):
        with pytest.raises(ValueError, match="non-monotonic offset"):
            plan.apply(value)
