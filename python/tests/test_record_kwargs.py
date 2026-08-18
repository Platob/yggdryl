"""Flattened record-option keywords on the ``IOBase`` record surface.

Every record method accepts each ``RecordOptions`` field as its own
keyword-only argument, resolved by one shared rule: the base comes from
``options`` when one is passed - or from the handle's media type otherwise -
and an explicit keyword always wins over the same field of the options
object, which itself is never mutated.
"""

from __future__ import annotations

import pathlib

import pyarrow as pa
import pytest

from yggdryl import IOBase, RecordOptions


ROWS = pa.table({"id": [1, 2, 3], "name": ["a", "b", None]})

SUFFIXES = ["arrows", "parquet", "avro"]


def handle(tmp_path: pathlib.Path, suffix: str, name: str = "t") -> IOBase:
    return IOBase(tmp_path / f"{name}.{suffix}")


class TestKwargsOnlyCalls:
    """The direct spelling works with no options object at all."""

    @pytest.mark.parametrize("suffix", SUFFIXES)
    def test_write_read_and_append_take_field_keywords(
        self, tmp_path: pathlib.Path, suffix: str
    ) -> None:
        resource = handle(tmp_path, suffix)
        resource.write_arrow(ROWS, root_name="record", safe=False)

        got = resource.read_arrow(select_by_names=["id"], batch_size=2)
        table = got.read_all()
        assert table.column_names == ["id"]
        assert table.column("id").to_pylist() == [1, 2, 3]

        resource.append_arrow(pa.table({"id": [4], "name": ["d"]}))
        assert resource.read_arrow().read_all().num_rows == 4

    @pytest.mark.parametrize("suffix", SUFFIXES)
    def test_a_merge_key_keyword_upserts_on_write(
        self, tmp_path: pathlib.Path, suffix: str
    ) -> None:
        resource = handle(tmp_path, suffix)
        resource.write_arrow(ROWS)
        resource.write_arrow(
            pa.table({"id": [3, 9], "name": ["c!", "i"]}),
            merge_by_names=["id"],
        )
        table = resource.read_arrow().read_all().sort_by("id")
        assert table.column("id").to_pylist() == [1, 2, 3, 9]
        assert table.column("name").to_pylist() == ["a", "b", "c!", "i"]

    @pytest.mark.parametrize("suffix", SUFFIXES)
    def test_a_merge_key_keyword_upserts_on_append_too(
        self, tmp_path: pathlib.Path, suffix: str
    ) -> None:
        """The key says which row an incoming row *is*, whichever verb carries it.

        An append that took the key and stored the row anyway contradicted the
        argument it was handed, and did it while returning successfully - the
        duplicate only shows up on the next read.
        """
        resource = handle(tmp_path, suffix)
        resource.write_arrow(ROWS)
        resource.append_arrow(
            pa.table({"id": [3, 9], "name": ["c!", "i"]}),
            merge_by_names=["id"],
        )
        table = resource.read_arrow().read_all().sort_by("id")
        assert table.column("id").to_pylist() == [1, 2, 3, 9]
        assert table.column("name").to_pylist() == ["a", "b", "c!", "i"]

        # A key naming no stored column is refused, as it already was on write.
        with pytest.raises(ValueError, match="nosuchcolumn"):
            resource.append_arrow(ROWS, merge_by_names=["nosuchcolumn"])

    @pytest.mark.parametrize("suffix", SUFFIXES)
    def test_an_append_naming_no_merge_key_still_stores_every_row(
        self, tmp_path: pathlib.Path, suffix: str
    ) -> None:
        """Without a key nothing identifies a row, so a repeat is a second row."""
        resource = handle(tmp_path, suffix)
        resource.write_arrow(ROWS)
        resource.append_arrow(pa.table({"id": [3], "name": ["c"]}))
        assert resource.read_arrow().read_all().column("id").to_pylist() == [1, 2, 3, 3]

    @pytest.mark.parametrize("suffix", SUFFIXES)
    def test_the_batch_reader_spellings_take_the_same_keywords(
        self, tmp_path: pathlib.Path, suffix: str
    ) -> None:
        resource = handle(tmp_path, suffix)
        resource.write_arrow_batch_reader(ROWS, root_name="record")
        resource.append_arrow_batch_reader(pa.table({"id": [4], "name": [None]}))
        got = resource.read_arrow_batch_reader(select_by_names=["name"])
        assert got.read_all().column_names == ["name"]

    def test_read_arrow_field_and_frames_take_keywords(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "parquet")
        resource.write_arrow(ROWS)
        root = resource.read_arrow_field(root_name="record")
        assert root.name == "record"
        frame = resource.read_pandas_frame(select_by_names=["id"])
        assert list(frame.columns) == ["id"]
        polars_frame = resource.read_polars_frame(select_by_names=["id"])
        assert polars_frame.columns == ["id"]

    def test_a_parquet_only_keyword_works_on_parquet(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "parquet")
        resource.write_arrow(
            ROWS,
            compression="zstd(3)",
            max_row_group_size=2,
            key_value_metadata={"writer": "kwargs"},
        )
        assert resource.read_arrow().read_all().num_rows == 3

    def test_a_parquet_only_keyword_is_refused_on_other_encodings(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "arrows")
        with pytest.raises(ValueError, match="expected Parquet options"):
            resource.write_arrow(ROWS, compression="zstd(3)")


    def test_a_batch_size_of_zero_is_refused_rather_than_read_as_nothing(
        self, tmp_path: pathlib.Path
    ) -> None:
        """Zero is not a small batch; it is a read that returns no rows at all.

        The readers chunk by this number, so storing it turned a hundred stored
        rows into a successful read of none. ``None`` already spells "no bound".
        """
        resource = handle(tmp_path, "parquet")
        resource.write_arrow(ROWS)

        with pytest.raises(ValueError, match="got 0"):
            resource.read_arrow(batch_size=0)

        assert resource.read_arrow(batch_size=None).read_all().num_rows == 3
        assert [batch.num_rows for batch in resource.read_arrow(batch_size=2)] == [2, 1]


class TestResolutionOrder:
    """options is the base; an explicit keyword always wins; never mutated."""

    def test_a_keyword_overrides_the_same_field_on_the_options(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "parquet")
        options = RecordOptions("application/vnd.apache.parquet")
        options.select_by_names = ["name"]
        resource.write_arrow(ROWS)

        got = resource.read_arrow(options=options, select_by_names=["id"])
        assert got.read_all().column_names == ["id"]

    def test_the_options_object_is_never_mutated_by_the_call(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "parquet")
        options = RecordOptions("application/vnd.apache.parquet")
        options.merge_by_names = ["id"]
        options.batch_size = 7

        resource.write_arrow(
            ROWS, options=options, merge_by_names=[], batch_size=100
        )
        assert options.merge_by_names == ["id"]
        assert options.batch_size == 7

    def test_unmerged_fields_of_the_options_still_apply(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "avro")
        options = RecordOptions("application/avro")
        options.select_by_names = ["id"]
        resource.write_arrow(ROWS)
        got = resource.read_arrow(options=options, batch_size=1)
        assert got.read_all().column_names == ["id"]


class TestUnknownKeywords:
    """A misspelled keyword is a TypeError naming the argument."""

    def test_read_names_the_unknown_argument(self, tmp_path: pathlib.Path) -> None:
        resource = handle(tmp_path, "parquet")
        with pytest.raises(TypeError, match="batch_sze"):
            resource.read_arrow(batch_sze=10)

    def test_write_names_the_unknown_argument(self, tmp_path: pathlib.Path) -> None:
        resource = handle(tmp_path, "parquet")
        with pytest.raises(
            TypeError, match=r"write_arrow\(\) got an unexpected keyword argument"
        ):
            resource.write_arrow(ROWS, merge_names=["id"])

    def test_every_method_checks_before_it_touches_anything(
        self, tmp_path: pathlib.Path
    ) -> None:
        resource = handle(tmp_path, "parquet")
        with pytest.raises(TypeError, match="wrong"):
            resource.append_arrow(ROWS, wrong=1)
        assert not resource.exists()
