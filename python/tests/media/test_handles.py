"""Streamed-write publication cadence at the Python boundary."""

from __future__ import annotations

import pathlib
from collections.abc import Iterator
from typing import Any

import pyarrow as pa
import pytest

from yggdryl import Csv, IOBase, Media
from yggdryl.coding import Gzip

ID_SCHEMA = pa.schema([pa.field("id", pa.int64(), nullable=False)])
TEXT_ID_SCHEMA = pa.schema([pa.field("id", pa.string(), nullable=False)])


def id_batch(ids: list[int]) -> pa.RecordBatch:
    """Build one native-integer batch."""
    return pa.record_batch({"id": ids}, schema=ID_SCHEMA)


def text_id_batch(ids: list[str]) -> pa.RecordBatch:
    """Build one text batch for a native cast-failure fixture."""
    return pa.record_batch({"id": ids}, schema=TEXT_ID_SCHEMA)


def ids_at(path: pathlib.Path) -> list[int]:
    """Read the externally visible integer ids from a fresh handle."""
    values = IOBase(path).read_arrow_reader().read_all().column("id").to_pylist()
    return sorted(values)


def seed(handle: IOBase, intent: str) -> list[int]:
    """Publish the state an interrupted operation must build on or replace."""
    stored = [1, 9] if intent == "merge" else [9]
    handle.overwrite_arrow_batch(id_batch(stored))
    return stored


def write_options(handle: IOBase, intent: str, commit_batches: int | None) -> Any:
    """Build valid options for one explicit write intent."""
    options = handle.record_options()
    options.commit_batch_num = commit_batches
    if intent == "merge":
        options.merge_by = ["id"]
    return options


@pytest.mark.parametrize("value", [True, False])
def test_commit_batch_num_rejects_booleans_like_the_javascript_binding(value: bool) -> None:
    handle = IOBase.from_bytes()
    handle.media_type = "application/vnd.apache.arrow.stream"
    options = handle.record_options()
    with pytest.raises(TypeError, match="commit_batch_num.*not bool"):
        options.commit_batch_num = value


def expected_prefix(intent: str) -> list[int]:
    """The first two incoming rows after one successful publication.

    Each fixture publishes rows 1 and 2 as one complete cadence of whole
    batches before its failure, so this is that cadence's state.
    """
    if intent == "append":
        return [1, 2, 9]
    if intent == "merge":
        return [1, 2, 9]
    return [1, 2]


WRITE_SHAPES = (
    "arrow_reader",
    "arrow_table",
    "arrow_batch",
    "records",
    "pandas",
    "pandas_frame",
    "polars",
    "polars_frame",
)


class Untouched:
    """A value that records either protocol or attribute inspection."""

    touches = 0

    def __iter__(self) -> Iterator[object]:
        type(self).touches += 1
        raise AssertionError("an invalid write iterated its input")

    def __getattribute__(self, name: str) -> object:
        type(self).touches += 1
        raise AssertionError(f"an invalid write inspected {name}")


@pytest.mark.parametrize("shape", WRITE_SHAPES)
@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_zero_cadence_is_rejected_before_every_input_is_inspected(
    tmp_path: pathlib.Path, shape: str, intent: str
) -> None:
    """A zero cadence is an options error, independent of Python shape."""
    handle = IOBase(tmp_path / f"zero-{intent}-{shape}.parquet")
    options = write_options(handle, intent, 0)
    source = Untouched()
    Untouched.touches = 0

    with pytest.raises(ValueError, match=r"\$\.commit_batch_num.*non-zero"):
        getattr(handle, f"{intent}_{shape}")(source, options=options)

    assert Untouched.touches == 0


@pytest.mark.parametrize("shape", WRITE_SHAPES)
def test_a_limited_merge_is_rejected_before_every_input_is_inspected(
    tmp_path: pathlib.Path, shape: str
) -> None:
    """A truncated keyed merge is invalid before any runtime conversion."""
    handle = IOBase(tmp_path / f"limited-merge-{shape}.parquet")
    options = write_options(handle, "merge", None)
    options.max_row_size = 1
    source = Untouched()
    Untouched.touches = 0

    with pytest.raises(ValueError, match="max_row_size.*merge_by"):
        getattr(handle, f"merge_{shape}")(source, options=options)

    assert Untouched.touches == 0


@pytest.mark.parametrize("shape", WRITE_SHAPES)
@pytest.mark.parametrize("bound", ["max_row_size", "max_byte_size"])
def test_a_zero_limit_overwrite_uses_the_declared_field_without_input_inspection(
    tmp_path: pathlib.Path, shape: str, bound: str
) -> None:
    """Typed empty overwrite needs no schema export from its Python value."""
    path = tmp_path / f"zero-overwrite-{bound}-{shape}.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_batch(id_batch([9]))
    options = write_options(handle, "overwrite", None)
    options.field = ID_SCHEMA
    setattr(options, bound, 0)
    source = Untouched()
    Untouched.touches = 0

    getattr(handle, f"overwrite_{shape}")(source, options=options)

    assert Untouched.touches == 0
    assert ids_at(path) == []


@pytest.mark.parametrize("shape", WRITE_SHAPES)
@pytest.mark.parametrize("bound", ["max_row_size", "max_byte_size"])
def test_a_zero_limit_append_is_a_noop_without_input_inspection(
    tmp_path: pathlib.Path, shape: str, bound: str
) -> None:
    """No input or destination publication is needed when append admits zero."""
    path = tmp_path / f"zero-append-{bound}-{shape}.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_batch(id_batch([9]))
    options = write_options(handle, "append", None)
    setattr(options, bound, 0)
    source = Untouched()
    Untouched.touches = 0

    getattr(handle, f"append_{shape}")(source, options=options)

    assert Untouched.touches == 0
    assert ids_at(path) == [9]


@pytest.mark.parametrize("shape", WRITE_SHAPES)
def test_a_zero_limit_overwrite_requires_a_field_without_input_inspection(
    tmp_path: pathlib.Path, shape: str
) -> None:
    """An input schema cannot be inferred without inspecting the forbidden input."""
    path = tmp_path / f"zero-overwrite-untyped-{shape}.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_batch(id_batch([9]))
    options = write_options(handle, "overwrite", None)
    options.max_row_size = 0
    source = Untouched()
    Untouched.touches = 0

    with pytest.raises(ValueError, match="field"):
        getattr(handle, f"overwrite_{shape}")(source, options=options)

    assert Untouched.touches == 0
    assert ids_at(path) == [9]


def input_for(shape: str, table: pa.Table) -> object:
    """Spell the same rows as each public Python write representation."""
    if shape == "arrow_reader":
        return table.to_reader()
    if shape == "arrow_table":
        return table
    if shape == "arrow_batch":
        return table.to_batches()[0]
    if shape == "records":
        return table.to_pylist()
    if shape.startswith("pandas"):
        pytest.importorskip("pandas")
        frame = table.to_pandas()
        return frame if shape.endswith("_frame") else [frame]
    if shape.startswith("polars"):
        polars = pytest.importorskip("polars")
        frame = polars.from_arrow(table)
        return frame if shape.endswith("_frame") else [frame]
    raise AssertionError(shape)


@pytest.mark.parametrize("shape", WRITE_SHAPES)
@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_every_python_adapter_honours_every_intent_with_a_one_batch_cadence(
    tmp_path: pathlib.Path, shape: str, intent: str
) -> None:
    """Reader, held Arrow, rows, and frame adapters share the core splitter."""
    path = tmp_path / f"matrix-{intent}-{shape}.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_batch(id_batch([10]))
    incoming = pa.Table.from_batches([id_batch([10, 2])])
    options = write_options(handle, intent, 1)

    getattr(handle, f"{intent}_{shape}")(
        input_for(shape, incoming), options=options
    )

    expected = [2, 10, 10] if intent == "append" else [2, 10]
    assert ids_at(path) == expected


@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_a_source_failure_leaves_each_complete_prefix_visible(
    tmp_path: pathlib.Path, intent: str
) -> None:
    """An Arrow source error discards only its incomplete cadence.

    Two one-row batches complete the two-batch cadence and publish; the third
    batch opens a cadence the failure discards.
    """
    path = tmp_path / f"source-{intent}.parquet"
    handle = IOBase(path)
    seed(handle, intent)
    options = write_options(handle, intent, 2)

    def batches() -> Iterator[pa.RecordBatch]:
        yield id_batch([1])
        yield id_batch([2])
        yield id_batch([3])
        raise RuntimeError("source failed after one commit")

    reader = pa.RecordBatchReader.from_batches(ID_SCHEMA, batches())
    with pytest.raises(ValueError, match="source failed after one commit"):
        getattr(handle, f"{intent}_arrow_reader")(reader, options=options)

    assert ids_at(path) == expected_prefix(intent)


@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_a_python_row_conversion_failure_leaves_each_complete_prefix_visible(
    tmp_path: pathlib.Path, intent: str
) -> None:
    """Rows convert one batch at a time, and the core counts the batches.

    At one row per batch, rows 1 and 2 are the two batches of one complete
    cadence, published before the third batch's row fails to convert.
    """
    path = tmp_path / f"conversion-{intent}.parquet"
    handle = IOBase(path)
    seed(handle, intent)
    options = write_options(handle, intent, 2)
    options.batch_row_size = 1

    def records() -> Iterator[dict[str, object]]:
        yield {"id": 1}
        yield {"id": 2}
        yield {"id": object()}

    with pytest.raises(ValueError, match="Could not convert"):
        getattr(handle, f"{intent}_records")(records(), options=options)

    assert ids_at(path) == expected_prefix(intent)


@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_row_batches_publish_whole_cadences_before_a_conversion_failure(
    tmp_path: pathlib.Path, intent: str
) -> None:
    """Two batches of 1,024 rows publish 2,048 rows; the third fails whole.

    A cadence counts whole batches and never cuts one, so the partial third
    batch - rows 2,049 to 2,500 and the row that fails - is discarded whole.
    """
    path = tmp_path / f"whole-batches-{intent}.parquet"
    handle = IOBase(path)
    seed(handle, intent)
    options = write_options(handle, intent, 2)
    options.batch_row_size = 1_024

    def records() -> Iterator[dict[str, object]]:
        for row_id in range(1, 2_501):
            yield {"id": row_id}
        yield {"id": object()}

    with pytest.raises(ValueError, match="Could not convert"):
        getattr(handle, f"{intent}_records")(records(), options=options)

    expected = list(range(1, 2_049))
    if intent == "append":
        expected.append(9)
        expected.sort()
    assert ids_at(path) == expected


@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_a_native_cast_failure_leaves_each_complete_prefix_visible(
    tmp_path: pathlib.Path, intent: str
) -> None:
    """A core cast error after a commit does not roll its prefix back.

    The first batch is one complete cadence of one batch; the second fails.
    """
    path = tmp_path / f"cast-{intent}.parquet"
    handle = IOBase(path)
    seed(handle, intent)
    options = write_options(handle, intent, 1)
    options.field = ID_SCHEMA
    options.safe = False

    reader = pa.RecordBatchReader.from_batches(
        TEXT_ID_SCHEMA,
        [text_id_batch(["1", "2"]), text_id_batch(["not-an-integer"])],
    )
    with pytest.raises(ValueError, match="not-an-integer"):
        getattr(handle, f"{intent}_arrow_reader")(reader, options=options)

    assert ids_at(path) == expected_prefix(intent)


@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_unset_cadence_publishes_nothing_before_a_failed_source_ends(
    tmp_path: pathlib.Path, intent: str
) -> None:
    """Without a cadence there is one publication, only after successful EOF."""
    path = tmp_path / f"once-{intent}.parquet"
    handle = IOBase(path)
    stored = seed(handle, intent)
    options = write_options(handle, intent, None)

    def batches() -> Iterator[pa.RecordBatch]:
        yield id_batch([1])
        yield id_batch([2])
        raise RuntimeError("source failed before its one publication")

    reader = pa.RecordBatchReader.from_batches(ID_SCHEMA, batches())
    with pytest.raises(ValueError, match="one publication"):
        getattr(handle, f"{intent}_arrow_reader")(reader, options=options)

    assert ids_at(path) == sorted(stored)


def test_a_cadence_larger_than_the_stream_publishes_only_the_final_remainder(
    tmp_path: pathlib.Path,
) -> None:
    """No incomplete prefix becomes visible before successful EOF."""
    path = tmp_path / "large-cadence.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_batch(id_batch([9]))
    options = write_options(handle, "overwrite", 10)
    observed: list[list[int]] = []

    def batches() -> Iterator[pa.RecordBatch]:
        yield id_batch([1, 2])
        observed.append(ids_at(path))

    handle.overwrite_arrow_reader(
        pa.RecordBatchReader.from_batches(ID_SCHEMA, batches()), options=options
    )

    assert observed == [[9]]
    assert ids_at(path) == [1, 2]


def test_a_full_cadence_publishes_before_the_next_batch_is_pulled(
    tmp_path: pathlib.Path,
) -> None:
    """The splitter never cuts a batch and never reads past a full cadence."""
    path = tmp_path / "whole-batches.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_batch(id_batch([9]))
    options = write_options(handle, "overwrite", 2)
    observed: list[list[int]] = []

    def batches() -> Iterator[pa.RecordBatch]:
        yield id_batch([1, 2])
        observed.append(ids_at(path))
        yield id_batch([3, 4])
        observed.append(ids_at(path))
        yield id_batch([5])
        observed.append(ids_at(path))
        raise RuntimeError("stop with one incomplete cadence")

    reader = pa.RecordBatchReader.from_batches(ID_SCHEMA, batches())
    with pytest.raises(ValueError, match="one incomplete cadence"):
        handle.overwrite_arrow_reader(reader, options=options)

    assert observed == [[9], [1, 2, 3, 4], [1, 2, 3, 4]]
    assert ids_at(path) == [1, 2, 3, 4]


def test_limits_apply_once_before_the_stream_is_split_into_commits(
    tmp_path: pathlib.Path,
) -> None:
    """A row limit is global and is never reset for each publication."""
    path = tmp_path / "limited.parquet"
    handle = IOBase(path)
    options = write_options(handle, "overwrite", 2)
    options.max_row_size = 3

    handle.overwrite_arrow_batch(id_batch([1, 2, 3, 4, 5]), options=options)

    assert ids_at(path) == [1, 2, 3]


def test_a_positive_row_limit_does_not_convert_or_pull_one_extra_python_row(
    tmp_path: pathlib.Path,
) -> None:
    """The row adapter stops at the global limit before the core slices it."""
    path = tmp_path / "row-limit-no-read-ahead.parquet"
    handle = IOBase(path)
    options = write_options(handle, "overwrite", None)
    options.max_row_size = 1
    pulled: list[int] = []

    def records() -> Iterator[dict[str, int]]:
        pulled.append(1)
        yield {"id": 1}
        pulled.append(2)
        raise AssertionError("the row after max_row_size was pulled")

    handle.overwrite_records(records(), options=options)

    assert pulled == [1]
    assert ids_at(path) == [1]


def test_a_positive_byte_limit_stops_before_requesting_another_arrow_batch(
    tmp_path: pathlib.Path,
) -> None:
    """After one admitted wide row, the limited core reader is terminal."""
    path = tmp_path / "byte-limit-no-read-ahead.parquet"
    handle = IOBase(path)
    options = write_options(handle, "overwrite", None)
    options.max_byte_size = 1
    pulled: list[int] = []

    def batches() -> Iterator[pa.RecordBatch]:
        pulled.append(1)
        yield id_batch([1])
        pulled.append(2)
        raise AssertionError("the batch after max_byte_size was pulled")

    handle.overwrite_arrow_reader(
        pa.RecordBatchReader.from_batches(ID_SCHEMA, batches()), options=options
    )

    assert pulled == [1]
    assert ids_at(path) == [1]


@pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
def test_empty_streams_keep_their_intent(
    tmp_path: pathlib.Path, intent: str
) -> None:
    """Empty overwrite clears; empty append and merge are no-ops."""
    path = tmp_path / f"empty-{intent}.parquet"
    handle = IOBase(path)
    stored = seed(handle, intent)
    options = write_options(handle, intent, 1)
    options.field = ID_SCHEMA
    empty = pa.RecordBatchReader.from_batches(ID_SCHEMA, [])

    getattr(handle, f"{intent}_arrow_reader")(empty, options=options)

    assert ids_at(path) == ([] if intent == "overwrite" else sorted(stored))


class TestHandleClasses:
    """`type(handle)` names the record implementation a name composes to."""

    def test_a_csv_or_tsv_name_composes_the_csv_class(self, tmp_path: pathlib.Path) -> None:
        for name in ("trades.csv", "trades.tsv", "trades.csv.gz"):
            handle = IOBase(tmp_path / name)
            assert type(handle) is Csv, name
            assert isinstance(handle, Media)
            # Descending leaves the record implementation behind.
            assert not isinstance(handle.into_handle(), Media), name
        assert Csv.__doc__ and "text/csv" in Csv.__doc__
        # The coding sits underneath the encoding, never over it.
        assert isinstance(IOBase(tmp_path / "trades.csv.gz").into_handle(), Gzip)
        assert str(IOBase(tmp_path / "trades.csv").record_options().mime_type) == "text/csv"
        assert IOBase(tmp_path / "trades.tsv").record_options().separator == "\t"
